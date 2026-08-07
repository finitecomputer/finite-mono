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
use std::time::SystemTime;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::Mutex as TokioMutex;

use crate::generations::{
    self, PAYLOAD_AGENTD_RELATIVE, StageRequest, read_link_version, remove_link, set_link_version,
};
use crate::state::{BadGeneration, FlipOutcome, FlipRecord, SeedRecord, SharedState, now_rfc3339};
use crate::supervise::{AgentdSpec, AgentdSupervisor, start_agentd};
use crate::{DataLayout, SHELL_VERSION, ShellError, ShellSettings, fixup, health, shims};

pub const SEED_TARBALL_NAME: &str = "payload.tar.gz";
pub const SEED_MANIFEST_NAME: &str = "payload.tar.gz.manifest.json";

/// The running shell: settings, durable state, and the one supervised agentd.
#[derive(Clone)]
pub struct ShellRuntime {
    pub settings: Arc<ShellSettings>,
    pub layout: Arc<DataLayout>,
    pub state: SharedState,
    supervisor: Arc<TokioMutex<Option<AgentdSupervisor>>>,
    /// Serializes stage/flip/rollback; a flip in progress refuses another.
    transition_gate: Arc<TokioMutex<()>>,
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
        let runtime = Self {
            settings: Arc::new(settings),
            layout: Arc::new(layout),
            state,
            supervisor: Arc::new(TokioMutex::new(None)),
            transition_gate: Arc::new(TokioMutex::new(())),
        };

        if read_link_version(&runtime.layout.current_link()).is_none() {
            runtime.unpack_seed()?;
        }
        runtime.reconcile_interrupted_flip()?;

        let current = read_link_version(&runtime.layout.current_link())
            .ok_or_else(|| ShellError::DataDir("no current generation after boot".to_owned()))?;
        // Post-stage fixup is idempotent; running it at boot repairs a
        // generation staged by an older shell or interrupted mid-fixup.
        fixup::apply_venv_fixup(
            &runtime.layout.generation_dir(&current),
            &runtime.layout.generation_dir(&current),
            &runtime.settings.shell_python,
        )?;
        runtime.write_current_shims()?;
        runtime.start_agentd_for_current().await?;
        Ok(runtime)
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
        let final_dir = self.layout.generation_dir(&version);
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
    /// The symlinks are the truth: record the interruption and where the
    /// symlinks actually landed; healthz surfaces it via `last_flip`.
    fn reconcile_interrupted_flip(&self) -> Result<(), ShellError> {
        let snapshot = self.state.snapshot();
        let Some(flip) = snapshot.last_flip else {
            return Ok(());
        };
        if flip.outcome != FlipOutcome::InProgress {
            return Ok(());
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
                        "the swap had committed"
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
        Ok(())
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

    /// Serve `/healthz` + `/contact` on the configured HTTP address.
    pub async fn serve_http(self, listener: TcpListener) {
        let settings = (*self.settings).clone();
        let layout = (*self.layout).clone();
        let runtime = self.clone();
        health::serve_http(listener, settings, layout, move || {
            let supervision = runtime
                .supervisor
                .try_lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(AgentdSupervisor::status));
            (runtime.state.snapshot(), supervision)
        })
        .await;
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
        let verb = request.get("verb").and_then(Value::as_str).unwrap_or("");
        match self.dispatch(verb, &request).await {
            Ok(response) => response,
            Err(error) => error_response(&error),
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
        let staged = self
            .state
            .snapshot()
            .staged
            .ok_or(ShellError::NothingStaged)?;
        let to = staged.version_label;
        if !self.layout.generation_dir(&to).is_dir() {
            return Err(ShellError::Contract(format!(
                "staged generation {to} is missing on disk"
            )));
        }
        let from = read_link_version(&self.layout.current_link());
        // Record in_progress before answering so `ctl flip --wait` always
        // observes the transition.
        self.begin_flip_record(&from, &to)?;
        let runtime = self.clone();
        let flip_to = to.clone();
        tokio::spawn(async move {
            let _gate = gate;
            runtime.run_transition(flip_to, TransitionKind::Flip).await;
        });
        Ok(json!({
            "ok": true,
            "result": "flip_started",
            "from": from,
            "to": to,
        }))
    }

    async fn verb_rollback(&self) -> Result<Value, ShellError> {
        let gate = Arc::clone(&self.transition_gate)
            .try_lock_owned()
            .map_err(|_| ShellError::FlipInProgress)?;
        let to = read_link_version(&self.layout.previous_link())
            .ok_or(ShellError::NoPreviousGeneration)?;
        if !self.layout.generation_dir(&to).is_dir() {
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
    /// rewrite shims, start the new agentd, health-gate, and on gate failure
    /// undo everything and (for flips only) mark the version bad.
    async fn run_transition(&self, to: String, kind: TransitionKind) {
        if let Err(error) = self.transition_inner(&to, kind).await {
            let _ = self.state.update(|state| {
                if let Some(flip) = state.last_flip.as_mut()
                    && flip.outcome == FlipOutcome::InProgress
                {
                    flip.outcome = FlipOutcome::RolledBack;
                    flip.detail = Some(format!("transition failed: {error}"));
                }
            });
            eprintln!("finite-shell: {kind:?} to {to} failed: {error}");
        }
    }

    async fn transition_inner(&self, to: &str, kind: TransitionKind) -> Result<(), ShellError> {
        let snapshot = self.state.snapshot();
        let flip = snapshot
            .last_flip
            .clone()
            .filter(|flip| flip.outcome == FlipOutcome::InProgress && flip.to == to)
            .ok_or_else(|| {
                ShellError::Supervisor(
                    "transition started without an in_progress record".to_owned(),
                )
            })?;
        let from = flip.from.clone();
        let prior_previous = flip.prior_previous.clone();

        // 1. Quiesce the running agentd (SIGTERM, bounded wait, SIGKILL).
        self.stop_agentd().await?;

        // 2. Retarget `previous` at the outgoing generation, then commit by
        //    swapping `current`. Both are atomic renames of tmp symlinks.
        if let Some(from) = &from {
            set_link_version(&self.layout.previous_link(), from)?;
        }
        let gate_started = SystemTime::now();
        set_link_version(&self.layout.current_link(), to)?; // COMMIT POINT
        self.state.update(|state| {
            if kind == TransitionKind::Flip
                && state
                    .staged
                    .as_ref()
                    .is_some_and(|staged| staged.version_label == to)
            {
                state.staged = None;
            }
        })?;

        // 3. Shims + new agentd.
        self.write_current_shims()?;
        self.start_agentd_for_current().await?;

        // 4. Health gate.
        match self.await_health_gate(gate_started).await {
            Ok(()) => {
                self.state.update(|state| {
                    if let Some(flip) = state.last_flip.as_mut() {
                        flip.outcome = FlipOutcome::Success;
                    }
                })?;
                Ok(())
            }
            Err(reason) => {
                // Undo: stop the new agentd, restore both symlinks, restore
                // shims, restart the old agentd.
                self.stop_agentd().await?;
                if let Some(from) = &from {
                    set_link_version(&self.layout.current_link(), from)?;
                } else {
                    remove_link(&self.layout.current_link())?;
                }
                match &prior_previous {
                    Some(previous) => {
                        set_link_version(&self.layout.previous_link(), previous)?;
                    }
                    None => remove_link(&self.layout.previous_link())?,
                }
                if from.is_some() {
                    self.write_current_shims()?;
                    self.start_agentd_for_current().await?;
                }
                self.state.update(|state| {
                    if kind == TransitionKind::Flip && !state.is_bad(to) {
                        state.bad.push(BadGeneration {
                            version_label: to.to_owned(),
                            reason: reason.clone(),
                            at: now_rfc3339(),
                        });
                    }
                    if let Some(flip) = state.last_flip.as_mut() {
                        flip.outcome = FlipOutcome::RolledBack;
                        flip.detail = Some(reason.clone());
                    }
                })?;
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
            let ready_present = self.layout.finitechat_ready_file().exists();
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

fn parse_request<T: serde::de::DeserializeOwned>(request: &Value) -> Result<T, ShellError> {
    let mut body = request.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("verb");
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
