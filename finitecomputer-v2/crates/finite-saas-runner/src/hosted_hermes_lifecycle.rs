//! Opt-in hosted ingress orchestration. Core supplies eligibility; containerd
//! supplies placement and saved reservations; systemd supplies process lifetime.
use crate::hosted_hermes_caddy::HostedHermesRouteManifest;
use crate::hosted_hermes_guard::{HostedHermesGuard, systemctl};
use crate::hosted_hermes_inventory::{
    HERMES_CONTAINER_PORT, HostedHermesInventory, InvalidInventory,
};
use crate::{CoreHttpAgentCreationQueue, PlannedCommand, RunnerError, wait_with_captured_output};
use finite_saas_core::store::hosted_hermes::HostedRouteTarget;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::time::Duration;

const STATE_DIR: &str = "/run/finite-hosted-hermes";
const PROXY_UNIT: &str = "finite-hosted-hermes.service";
const RUNNER_UNIT: &str = "finite-saas-runner.service";
const CONFIG_FILE: &str = "/run/finite-hosted-hermes/caddy.json";
const MAX_READ_BYTES: usize = 16 * 1024 * 1024;
type Result<T> = std::result::Result<T, RunnerError>;
fn refused(reason: &str) -> RunnerError {
    RunnerError::RuntimeLaunch(format!("hosted Hermes: {reason}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedHermesConfig {
    pub public_origin: String,
    pub listen: SocketAddr,
    pub allowed_origins: Vec<String>,
}

pub struct HostedHermesLifecycle {
    config: HostedHermesConfig,
    guard: Mutex<HostedHermesGuard>,
    nerdctl: PathBuf,
    namespace: String,
    source_host: String,
    work_root: PathBuf,
    core: CoreHttpAgentCreationQueue,
}

impl std::fmt::Debug for HostedHermesLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HostedHermesLifecycle { .. }")
    }
}

impl HostedHermesLifecycle {
    /// Exercises failed mutations and their fence without a systemd host.
    /// Successful publication is covered by the disposable Linux proof.
    #[cfg(test)]
    pub(crate) fn for_test(nerdctl: PathBuf, namespace: String, root: &Path) -> Self {
        Self {
            config: HostedHermesConfig {
                public_origin: "https://unused.invalid".into(),
                listen: "127.0.0.1:0".parse().unwrap(),
                allowed_origins: vec![],
            },
            guard: Mutex::new(HostedHermesGuard::for_test(root)),
            nerdctl,
            namespace,
            source_host: "finite-lat-1".into(),
            work_root: root.into(),
            core: CoreHttpAgentCreationQueue::new("http://127.0.0.1:0", "unused-test-token")
                .unwrap(),
        }
    }

    pub fn new(
        config: HostedHermesConfig,
        nerdctl: PathBuf,
        namespace: String,
        source_host: String,
        work_root: PathBuf,
        core: CoreHttpAgentCreationQueue,
    ) -> Result<Self> {
        HostedHermesRouteManifest {
            public_origin: config.public_origin.clone(),
            listen: config.listen,
            admin_socket: Path::new(STATE_DIR).join("admin.sock"),
            allowed_origins: config.allowed_origins.clone(),
            routes: vec![],
        }
        .caddy_config()
        .map_err(|_| refused("invalid deployment configuration"))?;
        Ok(Self {
            config,
            nerdctl,
            namespace,
            source_host,
            work_root,
            core,
            guard: Mutex::new(HostedHermesGuard::acquire(
                Path::new(STATE_DIR),
                RUNNER_UNIT,
                PROXY_UNIT,
            )?),
        })
    }

    /// Fail closed for hosted traffic on Core/inspection failures. Callers may
    /// still perform fenced lifecycle work for existing Chat and onboarding.
    pub fn reconcile(&self) -> Result<()> {
        let guard = self
            .guard
            .lock()
            .map_err(|_| refused("fence lock is poisoned"))?;
        self.observe("reconciling", None);
        match self.reconcile_locked(&guard) {
            Ok(serving) => {
                match guard.proxy_identity() {
                    Ok(identity) => {
                        self.observe(if serving { "serving" } else { "empty" }, Some(identity))
                    }
                    Err(_) => self.observe("unknown", None),
                }
                Ok(())
            }
            Err(error) => {
                self.observe("failed", None);
                guard.stop_proxy()?;
                Err(error)
            }
        }
    }

    pub(crate) fn execute(
        &self,
        command: &PlannedCommand,
        timeout: Duration,
        execute: impl FnOnce(&PlannedCommand, Duration) -> Result<Output>,
    ) -> Result<Output> {
        let verb = command.args.get(2).and_then(|arg| arg.to_str());
        if command.program != self.nerdctl
            || command.args.first().and_then(|arg| arg.to_str()) != Some("--namespace")
            || command.args.get(1).and_then(|arg| arg.to_str()) != Some(self.namespace.as_str())
        {
            return Err(refused("unexpected provider command shape"));
        }
        match verb {
            Some("inspect" | "ps" | "port" | "info" | "pull") => return execute(command, timeout),
            Some("run" | "create" | "rm" | "start" | "stop" | "rename" | "restart") => {}
            _ => return Err(refused("unclassified provider operation")),
        }
        let removal = if verb == Some("rm") {
            if command.args.len() != 5 || command.args[3] != "--force" {
                return Err(refused("unexpected remove command shape"));
            }
            Some(
                command.args[4]
                    .to_str()
                    .ok_or_else(|| refused("invalid removal target"))?,
            )
        } else {
            None
        };
        let guard = self
            .guard
            .lock()
            .map_err(|_| refused("fence lock is poisoned"))?;
        guard.before_publication()?;
        let mut command = command.clone();
        if matches!(verb, Some("start" | "restart")) {
            // Starting compute reinstalls saved CNI mappings. Refuse any
            // collision, including one held by a stopped container elsewhere.
            self.inventory()?;
        }
        if matches!(verb, Some("run" | "create")) {
            let inventory = self.inventory()?;
            let port = inventory
                .available_ports()
                .find(|port| TcpListener::bind(("127.0.0.1", *port)).is_ok())
                .ok_or_else(|| refused("no unreserved Hermes port is available"))?;
            command.args.splice(
                3..3,
                [
                    OsString::from("--publish"),
                    OsString::from(format!("127.0.0.1:{port}:{HERMES_CONTAINER_PORT}")),
                ],
            );
        }
        // Resolve authority, inventory and readiness while the old process is
        // still serving. The successor omits the record being removed. Start
        // it before provider IO: slow removal cannot prolong host-wide downtime
        // and no request in either process can follow that address into reuse.
        let successor = removal
            .map(|name| self.projection(Some(name)))
            .transpose()?;
        guard.begin_mutation(removal.is_some())?;
        if let Some(successor) = successor {
            self.publish(successor, &guard)?;
        }
        let output = execute(&command, timeout)?;
        if !output.status.success() {
            // Preserve the original provider result, but do not let a later
            // retry in this invocation cross an unresolved mutation.
            return Ok(output);
        }
        guard.finish_mutation()?;
        if removal.is_none() && self.reconcile_locked(&guard).is_err() {
            guard.stop_proxy()?;
            eprintln!(
                "hosted Hermes routes unavailable after provider mutation; ingress is stopped"
            );
        }
        Ok(output)
    }

    /// Same-image upgrades must still add the native port to an old container.
    pub(crate) fn has_native_binding(&self, container_name: &str) -> Result<bool> {
        let ports = self.provider(&self.namespace, &["port", container_name])?;
        let native = ports
            .lines()
            .filter(|line| line.starts_with("8642/tcp -> "))
            .collect::<Vec<_>>();
        Ok(native.len() == 1
            && native[0]
                .strip_prefix("8642/tcp -> 127.0.0.1:")
                .and_then(|port| port.parse::<u16>().ok())
                .is_some_and(|port| {
                    (crate::hosted_hermes_inventory::HOSTED_PORT_FIRST
                        ..=crate::hosted_hermes_inventory::HOSTED_PORT_LAST)
                        .contains(&port)
                }))
    }

    fn reconcile_locked(&self, guard: &HostedHermesGuard) -> Result<bool> {
        guard.before_publication()?;
        let projection = self.projection(None)?;
        let serving = projection.is_some();
        self.publish(projection, guard)?;
        Ok(serving)
    }

    // Diagnostic evidence for finite-status only. Never read by allocation,
    // publication or recovery. Missing/unwritable evidence must not stop Chat.
    fn observe(&self, state: &str, identity: Option<(String, String)>) {
        let checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or(0);
        let (pid, invocation) = identity.unwrap_or_default();
        let observation = serde_json::json!({
            "version": 1, "state": state, "checkedAt": checked_at,
            "proxyPid": pid, "proxyInvocation": invocation,
        });
        let path = Path::new(STATE_DIR);
        if write_observation(path, &observation).is_err() {
            eprintln!("hosted Hermes diagnostic evidence unavailable");
        }
    }

    fn projection(&self, excluded: Option<&str>) -> Result<Option<Vec<u8>>> {
        let targets = self.targets()?;
        let mut inventory = self.inventory()?;
        if let Some(name) = excluded {
            inventory.exclude_container(&self.namespace, name);
        }
        let mut routes = inventory
            .routes(
                &self.namespace,
                &self.source_host,
                &self.work_root,
                &targets,
            )
            .map_err(|_| refused("container ownership or saved reservations are inconsistent"))?;
        // Applied Core intent can predate an agent restart. Require the current
        // native listener too; this is an ordinary status read, never inference.
        let client = ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(500))
            .redirects(0)
            .build();
        routes.retain(|route| {
            client
                .get(&format!("http://127.0.0.1:{}/api/status", route.host_port))
                .call()
                .is_ok_and(|response| response.status() == 200)
        });
        if routes.is_empty() {
            return Ok(None);
        }
        let manifest = HostedHermesRouteManifest {
            public_origin: self.config.public_origin.clone(),
            listen: self.config.listen,
            admin_socket: Path::new(STATE_DIR).join("admin.sock"),
            allowed_origins: self.config.allowed_origins.clone(),
            routes,
        };
        let bytes = serde_json::to_vec(
            &manifest
                .caddy_config()
                .map_err(|_| refused("route projection is invalid"))?,
        )
        .map_err(|_| refused("cannot encode route projection"))?;
        Ok(Some(bytes))
    }

    /// Only consumes a projection computed under the host fence. During a
    /// marked removal this projection excludes the retiring container; it is
    /// the sole publication allowed before that mutation has completed.
    fn publish(&self, projection: Option<Vec<u8>>, guard: &HostedHermesGuard) -> Result<()> {
        let Some(bytes) = projection else {
            return guard.stop_proxy();
        };
        let unchanged = std::fs::read(CONFIG_FILE).is_ok_and(|old| old == bytes);
        if !unchanged {
            // Every withdrawal terminates accepted requests and open clients.
            // Never substitute a reload acknowledgement for confirmed exit.
            guard.stop_proxy()?;
            let temporary = Path::new(STATE_DIR).join("caddy.next.json");
            std::fs::write(&temporary, &bytes)
                .and_then(|_| std::fs::rename(&temporary, CONFIG_FILE))
                .map_err(|_| refused("cannot replace ephemeral proxy configuration"))?;
        }
        // The pre-start gate compares this immutable binary identity before
        // an upgrade/rollback may run a different Runner against live ingress.
        let executable =
            std::env::current_exe().map_err(|_| refused("cannot identify publisher"))?;
        std::fs::write(
            Path::new(STATE_DIR).join("publisher"),
            executable.as_os_str().as_encoded_bytes(),
        )
        .map_err(|_| refused("cannot record publisher identity"))?;
        systemctl(&["start", PROXY_UNIT])?;
        Ok(())
    }

    fn targets(&self) -> Result<Vec<HostedRouteTarget>> {
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .redirects(0)
            .build()
            .get(&format!(
                "{}/api/core/v1/hosted-hermes-route-targets",
                self.core.base_url
            ))
            .set("authorization", &format!("Bearer {}", self.core.api_token))
            .call()
            .map_err(|_| refused("Core route eligibility is unavailable"))?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take((MAX_READ_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| refused("Core route eligibility is unreadable"))?;
        if bytes.len() > MAX_READ_BYTES {
            return Err(refused("Core route eligibility exceeds bound"));
        }
        serde_json::from_slice(&bytes).map_err(|_| refused("Core route eligibility is malformed"))
    }

    fn inventory(&self) -> Result<HostedHermesInventory> {
        let namespaces = self.provider(&self.namespace, &["namespace", "ls", "--quiet"])?;
        let mut inventory = HostedHermesInventory::default();
        let mut seen = BTreeSet::new();
        for namespace in namespaces.lines() {
            if !seen.insert(namespace) || seen.len() > 128 {
                return Err(refused("ambiguous namespace inventory"));
            }
            let ids = self.provider(namespace, &["ps", "--all", "--no-trunc", "--quiet"])?;
            if ids.len() > 256 * 1024 {
                return Err(refused("container ID listing exceeds bound"));
            }
            let ids = ids.lines().collect::<BTreeSet<_>>();
            if ids.is_empty() {
                continue;
            }
            let mut args = vec!["inspect"];
            args.extend(ids.iter().copied());
            let inspect = self.provider(namespace, &args)?;
            #[derive(Deserialize)]
            struct Identity {
                #[serde(rename = "Id")]
                id: String,
            }
            let inspected: Vec<Identity> = serde_json::from_str(&inspect)
                .map_err(|_| refused("container inspect is malformed"))?;
            if inspected.len() != ids.len()
                || inspected
                    .iter()
                    .map(|record| record.id.as_str())
                    .collect::<BTreeSet<_>>()
                    != ids
            {
                return Err(refused("container inventory changed during inspection"));
            }
            inventory
                .add_namespace(namespace, inspect.as_bytes(), |id| {
                    self.provider(namespace, &["port", id])
                        .map_err(|_| InvalidInventory::port_read_failed())
                })
                .map_err(|_| refused("saved port inventory is incomplete or ambiguous"))?;
        }
        // A fresh host may not have the Runner namespace until its first run.
        // The complete host inventory is still authoritative for reservations.
        Ok(inventory)
    }

    fn provider(&self, namespace: &str, args: &[&str]) -> Result<String> {
        let child = Command::new(&self.nerdctl)
            .args(["--namespace", namespace])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| refused("cannot inspect provider"))?;
        let output = wait_with_captured_output(child, &self.nerdctl, Duration::from_secs(15))?;
        if !output.status.success() || output.stdout.len() > MAX_READ_BYTES {
            return Err(refused("provider inventory read failed"));
        }
        String::from_utf8(output.stdout).map_err(|_| refused("provider inventory is not UTF-8"))
    }
}

fn write_observation(root: &Path, observation: &serde_json::Value) -> std::io::Result<()> {
    let next = root.join("status.next.json");
    std::fs::write(&next, serde_json::to_vec(observation)?)?;
    std::fs::rename(next, root.join("status.json"))
}

#[cfg(test)]
mod observation_tests {
    use super::*;

    #[test]
    fn diagnostic_replacement_leaves_no_partial_record_or_mutation_marker() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        write_observation(root, &serde_json::json!({"state":"serving"})).unwrap();
        write_observation(root, &serde_json::json!({"state":"failed"})).unwrap();
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("status.json")).unwrap()).unwrap();
        assert_eq!(record["state"], "failed");
        assert!(!root.join("status.next.json").exists());
        assert!(!root.join("mutation-in-progress").exists());
        assert!(write_observation(&root.join("absent"), &record).is_err());
    }
}
