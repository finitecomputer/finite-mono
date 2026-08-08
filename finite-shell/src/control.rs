//! Boot, the control socket (`/data/shell/shell.sock`), and the flip engine.
//!
//! Socket protocol: line-delimited JSON. One request object
//! `{"verb": "...", ...}` per line, answered by exactly one response line
//! `{"ok": bool, "error"?: {"code", "message"}, ...}`.
//!
//! Crash-safe flip ordering: every transition is recorded in state.json as
//! it happens, and the `current` symlink swap is the commit point. On boot,
//! an `in_progress` flip record is reconciled against the actual symlinks —
//! the symlinks are the truth.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::Mutex as TokioMutex;

use crate::generations::{
    self, PAYLOAD_AGENTD_RELATIVE, StageRequest, contained_generation_dir, read_link_version,
    remove_link, set_link_version,
};
use crate::state::{BadGeneration, FlipOutcome, FlipRecord, SeedRecord, SharedState, now_rfc3339};
use crate::supervise::{AgentdSpec, AgentdSupervisor, start_agentd};
use crate::{
    CONTROL_TOKEN_ENV, DataLayout, SHELL_VERSION, ShellError, ShellSettings, fixup, health, shims,
};

pub const SEED_TARBALL_NAME: &str = "payload.tar.gz";
pub const SEED_MANIFEST_NAME: &str = "payload.tar.gz.manifest.json";

/// The running shell: settings, durable state, and the one supervised agentd.
#[derive(Clone)]
pub struct ShellRuntime {
    pub settings: Arc<ShellSettings>,
    pub layout: Arc<DataLayout>,
    pub state: SharedState,
    supervisor: Arc<TokioMutex<Option<AgentdSupervisor>>>,
    /// Serializes stage/flip/rollback across the socket verbs AND the
    /// autonomous channel poller; a transition in progress refuses another.
    pub(crate) transition_gate: Arc<TokioMutex<()>>,
    /// The per-boot control-socket token. Honest threat model: the token
    /// file is 0600 root-owned, but payload processes also run as root
    /// today, so the file alone is a weak boundary — the real boundary for
    /// LLM-spawned children is that agentd receives the token only via env
    /// and scrubs it from every child it spawns. This de-fangs "a
    /// prompt-injected payload process execs `finite-shell ctl rollback` in
    /// a loop" without claiming more than it delivers; SO_PEERCRED +
    /// non-root payloads are the real fix and remain future work.
    control_token: Arc<String>,
    /// Rate limit for unauthorized-request log lines (unix seconds of the
    /// last line): a rejected caller in a loop must not fill the log.
    unauthorized_log_at: Arc<AtomicU64>,
}

impl ShellRuntime {
    /// Boot the shell against `/data`: verify writability, seed on first
    /// boot, reconcile an interrupted flip from the symlinks, run the venv
    /// fixup, write shims, and start agentd. HTTP/socket serving is separate
    /// ([`ShellRuntime::serve_socket`], [`ShellRuntime::serve_http`]).
    pub async fn boot(settings: ShellSettings) -> Result<Self, ShellError> {
        let layout = settings.layout();
        verify_data_writable(&layout)?;
        fs::create_dir_all(layout.generations_dir())?;
        fs::create_dir_all(layout.shell_dir())?;
        fs::set_permissions(layout.shell_dir(), fs::Permissions::from_mode(0o700))?;

        let state = SharedState::load(&layout.state_path());
        let control_token = generate_control_token()?;
        write_control_token_file(&layout, &control_token)?;
        let runtime = Self {
            settings: Arc::new(settings),
            layout: Arc::new(layout),
            state,
            supervisor: Arc::new(TokioMutex::new(None)),
            transition_gate: Arc::new(TokioMutex::new(())),
            control_token: Arc::new(control_token),
            unauthorized_log_at: Arc::new(AtomicU64::new(0)),
        };

        // Disk hygiene before anything else touches generations/: stale
        // staging debris from a crashed stage, then unreferenced generation
        // directories.
        generations::sweep_stale_temp(&runtime.layout)?;

        if read_link_version(&runtime.layout.current_link()).is_none() {
            runtime.unpack_seed()?;
        }
        let pending_gate_rerun = runtime.reconcile_interrupted_flip()?;

        let current = read_link_version(&runtime.layout.current_link())
            .ok_or_else(|| ShellError::DataDir("no current generation after boot".to_owned()))?;
        generations::validate_generation_name(&current)?;
        // Post-stage fixup is idempotent; running it at boot repairs a
        // generation staged by an older shell or interrupted mid-fixup.
        fixup::apply_venv_fixup(
            &runtime.layout.generation_dir(&current),
            &runtime.layout.generation_dir(&current),
            &runtime.settings.shell_python,
        )?;
        runtime.write_current_shims()?;

        if let Some(flip) = pending_gate_rerun {
            // A flip committed but its health gate never resolved: the
            // adopted generation is unverified. Rerun the gate against it
            // (same timeout); on failure take the normal rollback + bad-list
            // path instead of booting an ungated candidate.
            runtime.rerun_gate_for_adopted_flip(&flip).await?;
        } else {
            runtime.start_agentd_for_current().await?;
        }

        if let Ok(pruned) = generations::prune_generations(&runtime.layout, &runtime.state)
            && !pruned.is_empty()
        {
            eprintln!("finite-shell: pruned unreferenced generations: {pruned:?}");
        }
        Ok(runtime)
    }

    /// The per-boot control token (tests and embedding callers).
    pub fn control_token(&self) -> &str {
        &self.control_token
    }

    /// First boot: verify + unpack the in-image seed payload and point
    /// `current` at it. Refuses to run without the release public key —
    /// unsigned payloads never reach `/data`.
    fn unpack_seed(&self) -> Result<(), ShellError> {
        let public_key_hex = self
            .settings
            .release_public_key
            .as_deref()
            .ok_or(ShellError::MissingReleaseKey)?;
        let public_key =
            finite_release::parse_verifying_key_hex(public_key_hex).map_err(|error| {
                ShellError::Config(format!("release public key is invalid: {error}"))
            })?;
        let tarball = self.settings.seed_dir.join(SEED_TARBALL_NAME);
        let manifest_path = self.settings.seed_dir.join(SEED_MANIFEST_NAME);
        let unpack_dir = self.layout.generations_dir().join(".seed-tmp");
        if unpack_dir.exists() {
            fs::remove_dir_all(&unpack_dir)?;
        }
        let verified = finite_release::verify_payload_bundle(
            &tarball,
            &manifest_path,
            &public_key,
            None,
            &unpack_dir,
        )?;
        let manifest = verified.manifest;
        if !unpack_dir.join(PAYLOAD_AGENTD_RELATIVE).is_file() {
            fs::remove_dir_all(&unpack_dir)?;
            return Err(ShellError::Contract(format!(
                "seed payload does not contain {PAYLOAD_AGENTD_RELATIVE}"
            )));
        }
        let version = manifest.version_label.clone();
        let final_dir = contained_generation_dir(&self.layout, &version)?;
        fixup::apply_venv_fixup(&unpack_dir, &final_dir, &self.settings.shell_python)?;
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }
        fs::rename(&unpack_dir, &final_dir)?;
        set_link_version(&self.layout.current_link(), &version)?;
        self.state.update(|state| {
            state.seed = Some(SeedRecord {
                version_label: version.clone(),
                tree_digest: manifest.tree_digest.clone(),
                recorded_at: now_rfc3339(),
            });
        })?;
        Ok(())
    }

    /// If state.json says a flip was in progress, the shell died mid-flip.
    /// The symlinks are the truth: record where they actually landed. When
    /// the swap had NOT committed the old generation simply boots
    /// (`Interrupted`). When it HAD committed, the adopted generation never
    /// passed its health gate — the returned record tells boot to rerun the
    /// gate instead of trusting an unverified candidate.
    fn reconcile_interrupted_flip(&self) -> Result<Option<FlipRecord>, ShellError> {
        let snapshot = self.state.snapshot();
        let Some(flip) = snapshot.last_flip else {
            return Ok(None);
        };
        if flip.outcome != FlipOutcome::InProgress {
            return Ok(None);
        }
        let actual_current = read_link_version(&self.layout.current_link());
        let committed = actual_current.as_deref() == Some(flip.to.as_str());
        self.state.update(|state| {
            if let Some(record) = state.last_flip.as_mut() {
                record.outcome = FlipOutcome::Interrupted;
                record.detail = Some(format!(
                    "shell restarted mid-flip; symlinks say current={} ({})",
                    actual_current.as_deref().unwrap_or("<none>"),
                    if committed {
                        "the swap had committed; the health gate reruns at boot"
                    } else {
                        "the swap had not committed"
                    }
                ));
            }
            if committed {
                // The swap was the commit point; the staged generation was
                // consumed.
                if state
                    .staged
                    .as_ref()
                    .is_some_and(|staged| staged.version_label == flip.to)
                {
                    state.staged = None;
                }
            }
        })?;
        Ok(committed.then_some(flip))
    }

    /// Boot-time completion of a committed-but-ungated flip: start the
    /// adopted generation's agentd, run the same health gate, and on failure
    /// perform the normal rollback + bad-list path.
    async fn rerun_gate_for_adopted_flip(&self, flip: &FlipRecord) -> Result<(), ShellError> {
        // Evidence files must come from the incoming generation, exactly as
        // in a live flip.
        let _ = fs::remove_file(self.layout.agentd_status_file());
        let _ = fs::remove_file(self.layout.finitechat_ready_file());
        let gate_started = SystemTime::now();
        self.start_agentd_for_current().await?;
        match self.await_health_gate(gate_started).await {
            Ok(()) => {
                self.state.update(|state| {
                    if let Some(record) = state.last_flip.as_mut() {
                        record.outcome = FlipOutcome::Success;
                        record.detail =
                            Some("interrupted flip's health gate rerun at boot passed".to_owned());
                    }
                })?;
                Ok(())
            }
            Err(reason) => {
                let reason = format!("boot gate rerun failed: {reason}");
                match self.restore_previous(flip).await {
                    Ok(()) => {
                        self.state.update(|state| {
                            if !state.is_bad(&flip.to) {
                                state.bad.push(BadGeneration {
                                    version_label: flip.to.clone(),
                                    reason: reason.clone(),
                                    at: now_rfc3339(),
                                });
                            }
                            if let Some(record) = state.last_flip.as_mut() {
                                record.outcome = FlipOutcome::RolledBack;
                                record.detail = Some(reason.clone());
                            }
                        })?;
                        Ok(())
                    }
                    Err(restore_error) => {
                        self.state.update(|state| {
                            if let Some(record) = state.last_flip.as_mut() {
                                record.outcome = FlipOutcome::FailedOpen;
                                record.detail = Some(format!(
                                    "{reason}; restoration also failed: {restore_error}"
                                ));
                            }
                        })?;
                        // The shell stays up (healthz surfaces failed_open);
                        // boot itself does not error out.
                        Ok(())
                    }
                }
            }
        }
    }

    fn write_current_shims(&self) -> Result<Vec<String>, ShellError> {
        shims::write_shims(
            &self.settings.shim_dir,
            &self.layout.current_link().join("bin"),
            &self.layout.current_via_link().join("bin"),
        )
    }

    fn agentd_spec(&self) -> Result<AgentdSpec, ShellError> {
        let current_real = fs::canonicalize(self.layout.current_link()).map_err(|error| {
            ShellError::DataDir(format!("current generation is unreadable: {error}"))
        })?;
        let mut env: BTreeMap<String, String> = self.settings.agentd_env.clone();
        env.insert(
            "FINITE_PAYLOAD_ROOT".to_owned(),
            current_real.display().to_string(),
        );
        // The control-socket token rides only to the one supervised agentd;
        // agentd scrubs it from every child it spawns.
        env.insert(
            CONTROL_TOKEN_ENV.to_owned(),
            self.control_token.as_ref().clone(),
        );
        env.insert(
            "PATH".to_owned(),
            format!(
                "{}:{}",
                current_real.join("bin").display(),
                self.settings.base_path
            ),
        );
        Ok(AgentdSpec {
            program: current_real.join("bin/finite-agentd"),
            args: vec!["serve".to_owned()],
            env,
            backoff_initial: self.settings.restart_backoff_initial,
            backoff_max: self.settings.restart_backoff_max,
            crash_loop_window: self.settings.crash_loop_window,
            crash_loop_restarts: self.settings.crash_loop_restarts,
        })
    }

    async fn start_agentd_for_current(&self) -> Result<(), ShellError> {
        let spec = self.agentd_spec()?;
        let mut slot = self.supervisor.lock().await;
        *slot = Some(start_agentd(spec, self.state.clone()));
        Ok(())
    }

    async fn stop_agentd(&self) -> Result<(), ShellError> {
        let supervisor = self.supervisor.lock().await.take();
        if let Some(supervisor) = supervisor {
            supervisor.stop(self.settings.quiesce_timeout).await?;
        }
        Ok(())
    }

    pub async fn agentd_supervision(&self) -> Option<crate::supervise::AgentdSupervisionStatus> {
        self.supervisor
            .lock()
            .await
            .as_ref()
            .map(AgentdSupervisor::status)
    }

    /// Stop agentd gracefully — the shell's own (PID 1) shutdown path.
    pub async fn shutdown(&self) {
        let _ = self.stop_agentd().await;
    }

    // ------------------------------------------------------------------
    // Socket serving
    // ------------------------------------------------------------------

    /// Bind the control socket (dir 0700, socket 0600) and serve verbs until
    /// the process exits.
    pub fn bind_socket(&self) -> Result<UnixListener, ShellError> {
        let path = self.layout.socket_path();
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        // virtiofs (the Apple Container /data bind) accepts the bind but
        // rejects chmod on socket inodes with EINVAL. The 0700 shell
        // directory is the effective protection there, so a failed tighten
        // is reported, not fatal.
        if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "finite-shell: control socket permissions were not tightened ({error}); relying on the 0700 {}",
                self.layout.shell_dir().display()
            );
        }
        Ok(listener)
    }

    pub async fn serve_socket(self, listener: UnixListener) {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let runtime = self.clone();
            tokio::spawn(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut lines = BufReader::new(read_half).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_owned();
                    if line.is_empty() {
                        continue;
                    }
                    let response = runtime.handle_request_line(&line).await;
                    let mut bytes = response.to_string().into_bytes();
                    bytes.push(b'\n');
                    if write_half.write_all(&bytes).await.is_err() {
                        return;
                    }
                }
            });
        }
    }

    /// The full `/healthz` body for a booted shell, including the live
    /// supervision snapshot the readiness verdict depends on.
    pub fn health_body(&self) -> Value {
        let supervision = self
            .supervisor
            .try_lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(AgentdSupervisor::status));
        health::runtime_health(
            &self.settings,
            &self.layout,
            &self.state.snapshot(),
            supervision.as_ref(),
        )
    }

    /// Serve `/healthz` + `/contact` on the configured HTTP address.
    pub async fn serve_http(self, listener: TcpListener) {
        let runtime = self.clone();
        health::serve_http(listener, move || runtime.health_body()).await;
    }

    pub async fn handle_request_line(&self, line: &str) -> Value {
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return error_response(&ShellError::InvalidRequest(format!(
                    "request is not JSON: {error}"
                )));
            }
        };
        // Token handshake: every request must carry the per-boot token. A
        // mismatch is answered (rate-limited in the log) without touching
        // any verb machinery.
        let presented = request.get("token").and_then(Value::as_str);
        if presented != Some(self.control_token.as_str()) {
            self.log_unauthorized(presented.is_some());
            return error_response(&ShellError::Unauthorized);
        }
        let verb = request.get("verb").and_then(Value::as_str).unwrap_or("");
        match self.dispatch(verb, &request).await {
            Ok(response) => response,
            Err(error) => error_response(&error),
        }
    }

    /// Log an unauthorized socket request at most once per ten seconds: a
    /// refused caller in a loop must not fill the log.
    fn log_unauthorized(&self, token_present: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let last = self.unauthorized_log_at.load(Ordering::Relaxed);
        if now.saturating_sub(last) < 10 {
            return;
        }
        if self
            .unauthorized_log_at
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            eprintln!(
                "finite-shell: refused a control-socket request with {} token",
                if token_present { "a wrong" } else { "no" }
            );
        }
    }

    async fn dispatch(&self, verb: &str, request: &Value) -> Result<Value, ShellError> {
        match verb {
            "status" => self.verb_status().await,
            "stage" => self.verb_stage(request).await,
            "flip" => self.verb_flip().await,
            "rollback" => self.verb_rollback().await,
            "set-channel" => self.verb_set_channel(request),
            other => Err(ShellError::InvalidRequest(format!(
                "unsupported verb {other:?}"
            ))),
        }
    }

    async fn verb_status(&self) -> Result<Value, ShellError> {
        let state = self.state.snapshot();
        let supervision = self.agentd_supervision().await;
        Ok(json!({
            "ok": true,
            "shellVersion": SHELL_VERSION,
            "current": read_link_version(&self.layout.current_link()),
            "previous": read_link_version(&self.layout.previous_link()),
            "channel": fs::read_to_string(self.layout.channel_path())
                .ok()
                .map(|value| value.trim().to_owned()),
            "state": serde_json::to_value(&state)?,
            "agentd": supervision
                .map(serde_json::to_value)
                .transpose()?
                .unwrap_or(Value::Null),
        }))
    }

    async fn verb_stage(&self, request: &Value) -> Result<Value, ShellError> {
        let stage_request: StageRequest = parse_request(request)?;
        let _gate = self.transition_gate.lock().await;
        let staged =
            generations::stage_payload(&self.settings, &self.layout, &self.state, &stage_request)
                .await?;
        Ok(json!({
            "ok": true,
            "result": "staged",
            "versionLabel": staged.version_label,
            "artifactId": staged.artifact_id,
            "treeDigest": staged.tree_digest,
            "tarballSha256": staged.tarball_sha256,
        }))
    }

    async fn verb_flip(&self) -> Result<Value, ShellError> {
        let gate = Arc::clone(&self.transition_gate)
            .try_lock_owned()
            .map_err(|_| ShellError::FlipInProgress)?;
        let (from, to) = self.start_flip_locked(gate)?;
        Ok(json!({
            "ok": true,
            "result": "flip_started",
            "from": from,
            "to": to,
        }))
    }

    /// Start the flip to the staged generation while already holding the
    /// transition gate — the one flip engine, shared by the socket verb and
    /// the channel poller. Records `in_progress` before returning so callers
    /// (and `ctl flip --wait`) always observe the transition, then runs it in
    /// the background holding the gate.
    pub(crate) fn start_flip_locked(
        &self,
        gate: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<(Option<String>, String), ShellError> {
        let staged = self
            .state
            .snapshot()
            .staged
            .ok_or(ShellError::NothingStaged)?;
        let to = staged.version_label;
        if !contained_generation_dir(&self.layout, &to)?.is_dir() {
            return Err(ShellError::Contract(format!(
                "staged generation {to} is missing on disk"
            )));
        }
        let from = read_link_version(&self.layout.current_link());
        self.begin_flip_record(&from, &to)?;
        let runtime = self.clone();
        let flip_to = to.clone();
        tokio::spawn(async move {
            let _gate = gate;
            runtime.run_transition(flip_to, TransitionKind::Flip).await;
        });
        Ok((from, to))
    }

    async fn verb_rollback(&self) -> Result<Value, ShellError> {
        let gate = Arc::clone(&self.transition_gate)
            .try_lock_owned()
            .map_err(|_| ShellError::FlipInProgress)?;
        let to = read_link_version(&self.layout.previous_link())
            .ok_or(ShellError::NoPreviousGeneration)?;
        if !contained_generation_dir(&self.layout, &to)?.is_dir() {
            return Err(ShellError::Contract(format!(
                "previous generation {to} is missing on disk"
            )));
        }
        let from = read_link_version(&self.layout.current_link());
        self.begin_flip_record(&from, &to)?;
        let runtime = self.clone();
        let rollback_to = to.clone();
        tokio::spawn(async move {
            let _gate = gate;
            runtime
                .run_transition(rollback_to, TransitionKind::Rollback)
                .await;
        });
        Ok(json!({
            "ok": true,
            "result": "rollback_started",
            "from": from,
            "to": to,
        }))
    }

    fn verb_set_channel(&self, request: &Value) -> Result<Value, ShellError> {
        let channel = request
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| ShellError::InvalidRequest("channel is required".to_owned()))?;
        if !matches!(channel, "stable" | "canary") {
            return Err(ShellError::InvalidRequest(format!(
                "channel must be \"stable\" or \"canary\", not {channel:?}"
            )));
        }
        // The channel file is a plain name (consumed by M4's channel pull),
        // written atomically like every other shell-owned file.
        let path = self.layout.channel_path();
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, format!("{channel}\n"))?;
        fs::rename(&temporary, &path)?;
        Ok(json!({ "ok": true, "channel": channel }))
    }

    fn begin_flip_record(&self, from: &Option<String>, to: &str) -> Result<(), ShellError> {
        let prior_previous = read_link_version(&self.layout.previous_link());
        self.state.update(|state| {
            state.last_flip = Some(FlipRecord {
                from: from.clone(),
                to: to.to_owned(),
                at: now_rfc3339(),
                outcome: FlipOutcome::InProgress,
                prior_previous,
                detail: None,
            });
        })
    }

    /// The flip machinery, shared by `flip` and `rollback`: quiesce agentd,
    /// swap symlinks (previous first, then the `current` commit point),
    /// rewrite shims, start the new agentd, health-gate, and on any failure
    /// attempt actual restoration. The journal records what ACTUALLY
    /// happened: `rolled_back` only after restoration verifiably succeeded,
    /// `failed_open` when restoration failed or is unverifiable (healthz
    /// surfaces that prominently and reports ready:false).
    async fn run_transition(&self, to: String, kind: TransitionKind) {
        let outcome = self.transition_inner(&to, kind).await;
        let Err((error, progress)) = outcome else {
            return;
        };
        eprintln!("finite-shell: {kind:?} to {to} failed: {error}");
        let flip = self.state.snapshot().last_flip.clone();
        let Some(flip) = flip.filter(|flip| flip.outcome == FlipOutcome::InProgress) else {
            return;
        };
        let reason = format!("transition failed: {error}");
        // The transition may have stopped the old agentd and/or committed
        // the symlink swap before erroring. Attempt actual restoration and
        // record what really happened — never claim rolled_back without it.
        let restored = match progress {
            TransitionProgress::NothingChanged => {
                // Nothing was touched, but the old agentd may or may not be
                // running (the error was before the stop). Verify it.
                self.verify_restored(&flip.from).await
            }
            TransitionProgress::OldStopped | TransitionProgress::Committed => {
                self.restore_previous(&flip).await
            }
        };
        let _ = self.state.update(|state| match &restored {
            Ok(()) => {
                if let Some(record) = state.last_flip.as_mut() {
                    record.outcome = FlipOutcome::RolledBack;
                    record.detail = Some(reason.clone());
                }
            }
            Err(restore_error) => {
                if let Some(record) = state.last_flip.as_mut() {
                    record.outcome = FlipOutcome::FailedOpen;
                    record.detail = Some(format!("{reason}; restoration failed: {restore_error}"));
                }
            }
        });
    }

    /// Stop whatever runs now, restore the symlinks/shims to the flip's
    /// `from`/`prior_previous`, restart the old agentd, and verify the
    /// restoration actually took effect.
    async fn restore_previous(&self, flip: &FlipRecord) -> Result<(), ShellError> {
        self.stop_agentd().await?;
        let _ = fs::remove_file(self.layout.agentd_status_file());
        let _ = fs::remove_file(self.layout.finitechat_ready_file());
        match &flip.from {
            Some(from) => {
                contained_generation_dir(&self.layout, from)?;
                set_link_version(&self.layout.current_link(), from)?;
            }
            None => remove_link(&self.layout.current_link())?,
        }
        match &flip.prior_previous {
            Some(previous) => set_link_version(&self.layout.previous_link(), previous)?,
            None => remove_link(&self.layout.previous_link())?,
        }
        if flip.from.is_some() {
            self.write_current_shims()?;
            self.start_agentd_for_current().await?;
        }
        self.verify_restored(&flip.from).await
    }

    /// Restoration is only claimed when it is verifiable: `current` points
    /// at the expected generation and (when one exists) its agentd
    /// supervision is live again.
    async fn verify_restored(&self, expected_current: &Option<String>) -> Result<(), ShellError> {
        let actual = read_link_version(&self.layout.current_link());
        if actual != *expected_current {
            return Err(ShellError::Supervisor(format!(
                "current points at {actual:?}, expected {expected_current:?}"
            )));
        }
        if expected_current.is_some() && self.agentd_supervision().await.is_none() {
            return Err(ShellError::Supervisor(
                "the previous generation's agentd is not supervised".to_owned(),
            ));
        }
        Ok(())
    }

    async fn transition_inner(
        &self,
        to: &str,
        kind: TransitionKind,
    ) -> Result<(), (ShellError, TransitionProgress)> {
        let snapshot = self.state.snapshot();
        let flip = snapshot
            .last_flip
            .clone()
            .filter(|flip| flip.outcome == FlipOutcome::InProgress && flip.to == to)
            .ok_or_else(|| {
                (
                    ShellError::Supervisor(
                        "transition started without an in_progress record".to_owned(),
                    ),
                    TransitionProgress::NothingChanged,
                )
            })?;
        let from = flip.from.clone();
        let at = |progress: TransitionProgress| move |error: ShellError| (error, progress);

        // 1. Quiesce the running agentd (SIGTERM, bounded wait, SIGKILL —
        //    the whole process group, so no old-generation straggler holds a
        //    loopback port into the new generation's startup).
        self.stop_agentd()
            .await
            .map_err(at(TransitionProgress::NothingChanged))?;

        // The status/ready files belong to the processes that just died; the
        // health gate must only ever see evidence written by the incoming
        // generation. (An orphaned bridge once kept its ready file fresh and
        // made a broken flip look healthy.)
        let _ = std::fs::remove_file(self.layout.agentd_status_file());
        let _ = std::fs::remove_file(self.layout.finitechat_ready_file());

        // 2. Retarget `previous` at the outgoing generation, then commit by
        //    swapping `current`. Both are atomic renames of tmp symlinks.
        if let Some(from) = &from {
            set_link_version(&self.layout.previous_link(), from)
                .map_err(at(TransitionProgress::OldStopped))?;
        }
        let gate_started = SystemTime::now();
        set_link_version(&self.layout.current_link(), to) // COMMIT POINT
            .map_err(at(TransitionProgress::OldStopped))?;
        self.state
            .update(|state| {
                if kind == TransitionKind::Flip
                    && state
                        .staged
                        .as_ref()
                        .is_some_and(|staged| staged.version_label == to)
                {
                    state.staged = None;
                }
            })
            .map_err(at(TransitionProgress::Committed))?;

        // 3. Shims + new agentd.
        self.write_current_shims()
            .map_err(at(TransitionProgress::Committed))?;
        self.start_agentd_for_current()
            .await
            .map_err(at(TransitionProgress::Committed))?;

        // 4. Health gate.
        match self.await_health_gate(gate_started).await {
            Ok(()) => {
                // Reclaim disk only after the gate passed: everything except
                // current, previous, and staged. Prune before the journal
                // records success so an observer of the outcome sees the
                // post-prune disk state.
                if let Ok(pruned) = generations::prune_generations(&self.layout, &self.state)
                    && !pruned.is_empty()
                {
                    eprintln!("finite-shell: pruned unreferenced generations: {pruned:?}");
                }
                self.state
                    .update(|state| {
                        if let Some(flip) = state.last_flip.as_mut() {
                            flip.outcome = FlipOutcome::Success;
                        }
                    })
                    .map_err(at(TransitionProgress::Committed))?;
                Ok(())
            }
            Err(reason) => {
                // Gate failure: restore, and record what actually happened.
                let restored = self.restore_previous(&flip).await;
                self.state
                    .update(|state| {
                        if kind == TransitionKind::Flip && !state.is_bad(to) {
                            state.bad.push(BadGeneration {
                                version_label: to.to_owned(),
                                reason: reason.clone(),
                                at: now_rfc3339(),
                            });
                        }
                        if let Some(flip) = state.last_flip.as_mut() {
                            match &restored {
                                Ok(()) => {
                                    flip.outcome = FlipOutcome::RolledBack;
                                    flip.detail = Some(reason.clone());
                                }
                                Err(restore_error) => {
                                    flip.outcome = FlipOutcome::FailedOpen;
                                    flip.detail = Some(format!(
                                        "{reason}; restoration failed: {restore_error}"
                                    ));
                                }
                            }
                        }
                    })
                    .map_err(at(TransitionProgress::Committed))?;
                Ok(())
            }
        }
    }

    /// The cheap gate: within the health timeout the agentd process must be
    /// alive, the agentd status file freshly written, and the finitechat
    /// ready file present — the same files health_server.py reads. No
    /// inference roundtrip (canary soak is the real verification).
    async fn await_health_gate(&self, since: SystemTime) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + self.settings.health_timeout;
        loop {
            let supervision = self.agentd_supervision().await;
            let process_alive = supervision
                .as_ref()
                .is_some_and(|status| status.state == "running" && status.pid.is_some());
            let status_fresh = health::file_written_since(&self.layout.agentd_status_file(), since);
            let ready_present =
                health::file_written_since(&self.layout.finitechat_ready_file(), since);
            if process_alive && status_fresh && ready_present {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "health gate failed: agentd_running={process_alive} status_fresh={status_fresh} ready_file={ready_present}"
                ));
            }
            tokio::time::sleep(self.settings.health_gate_poll).await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionKind {
    /// Health-gate failures mark the target version bad.
    Flip,
    /// Never marks bad.
    Rollback,
}

/// How far a failed transition got — what restoration must undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionProgress {
    /// The error happened before the old agentd was stopped.
    NothingChanged,
    /// The old agentd was stopped but the `current` swap did not commit.
    OldStopped,
    /// The `current` swap committed; the candidate may be selected/running.
    Committed,
}

/// Generate the shell's per-boot control token: 32 random bytes, hex.
fn generate_control_token() -> Result<String, ShellError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| ShellError::Config(format!("cannot gather entropy: {error}")))?;
    Ok(hex::encode(bytes))
}

/// Write the token file (0600, atomic) for `finite-shell ctl`, which runs as
/// root via container exec and reads it directly. The file carries the bare
/// hex token plus a newline.
fn write_control_token_file(layout: &DataLayout, token: &str) -> Result<(), ShellError> {
    use std::io::Write as _;
    let path = layout.control_token_path();
    let parent = path
        .parent()
        .ok_or_else(|| ShellError::Io(std::io::Error::other("token path has no parent")))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(format!("{token}\n").as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| ShellError::Io(error.error))?;
    Ok(())
}

/// Read the control token file next to `socket_path` (the ctl client's side
/// of the handshake).
pub fn read_control_token_near(socket_path: &std::path::Path) -> Option<String> {
    let dir = socket_path.parent()?;
    let token = fs::read_to_string(dir.join("control-token")).ok()?;
    let token = token.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

fn parse_request<T: serde::de::DeserializeOwned>(request: &Value) -> Result<T, ShellError> {
    let mut body = request.clone();
    if let Some(object) = body.as_object_mut() {
        // Socket framing fields, not verb-body fields.
        object.remove("verb");
        object.remove("token");
    }
    serde_json::from_value(body)
        .map_err(|error| ShellError::InvalidRequest(format!("request body is invalid: {error}")))
}

pub fn error_response(error: &ShellError) -> Value {
    json!({
        "ok": false,
        "error": { "code": error.code(), "message": error.to_string() },
    })
}

/// Refuse to start unless `/data` is a writable directory.
fn verify_data_writable(layout: &DataLayout) -> Result<(), ShellError> {
    if !layout.data_dir.is_dir() {
        return Err(ShellError::DataDir(format!(
            "{} is not a mounted directory",
            layout.data_dir.display()
        )));
    }
    let probe = layout.data_dir.join(".finite-shell-write-probe");
    fs::write(&probe, b"probe")
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|error| {
            ShellError::DataDir(format!(
                "{} is not writable: {error}",
                layout.data_dir.display()
            ))
        })
}

/// Boot lifecycle shared with the HTTP server, which binds BEFORE boot: a
/// boot failure must keep serving a diagnosable `/healthz` instead of
/// exiting PID 1 into a restart loop that serves nothing (the zombie class:
/// health probes see connection-refused forever while the container
/// restart-loops on a deterministic failure).
#[derive(Clone, Default)]
pub struct BootHealth {
    inner: Arc<std::sync::RwLock<BootPhase>>,
}

#[derive(Default)]
enum BootPhase {
    #[default]
    Bootstrapping,
    Failed {
        detail: String,
    },
    Ready(ShellRuntime),
}

impl BootHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_failed(&self, detail: String) {
        *self.inner.write().expect("boot phase lock poisoned") = BootPhase::Failed { detail };
    }

    pub fn set_ready(&self, runtime: ShellRuntime) {
        *self.inner.write().expect("boot phase lock poisoned") = BootPhase::Ready(runtime);
    }

    /// The `/healthz` body for the current phase: a degraded bootstrapping /
    /// bootstrap_failed shape before boot, the full runtime health after.
    pub fn health_body(&self) -> Value {
        match &*self.inner.read().expect("boot phase lock poisoned") {
            BootPhase::Bootstrapping => json!({
                "ready": false,
                "error": "bootstrapping",
                "shell_version": SHELL_VERSION,
            }),
            BootPhase::Failed { detail } => json!({
                "ready": false,
                "error": "bootstrap_failed",
                "detail": detail,
                "shell_version": SHELL_VERSION,
            }),
            BootPhase::Ready(runtime) => runtime.health_body(),
        }
    }
}

/// Sanitize a boot error for the public healthz body: single line, bounded.
fn sanitized_boot_detail(error: &ShellError) -> String {
    let detail = error.to_string().replace(['\n', '\r'], " ");
    detail.chars().take(512).collect()
}

/// Boot with bounded backoff, forever, publishing each failure into
/// `health`. The container restart no longer helps when the failure is
/// deterministic — but an operator or provider can now SEE it in `/healthz`
/// instead of connection-refused.
pub async fn boot_until_ready(
    settings: ShellSettings,
    health: BootHealth,
    backoff_initial: Duration,
    backoff_max: Duration,
) -> ShellRuntime {
    let mut backoff = backoff_initial;
    loop {
        match ShellRuntime::boot(settings.clone()).await {
            Ok(runtime) => {
                health.set_ready(runtime.clone());
                return runtime;
            }
            Err(error) => {
                eprintln!("finite-shell: boot failed (retrying in {backoff:?}): {error}");
                health.set_failed(sanitized_boot_detail(&error));
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(backoff_max);
            }
        }
    }
}

/// One client request over the control socket (used by `finite-shell ctl`).
pub fn ctl_roundtrip(socket_path: &PathBuf, request: &Value) -> Result<Value, ShellError> {
    use std::io::{BufRead, BufReader as StdBufReader, Write};
    let stream = std::os::unix::net::UnixStream::connect(socket_path)?;
    let mut writer = stream.try_clone()?;
    let mut line = request.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;
    let mut reader = StdBufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    serde_json::from_str(response.trim()).map_err(ShellError::from)
}
