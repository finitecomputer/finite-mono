//! Local stand-ins for the production hosted-Hermes transport (docker-saas).
//!
//! The ingress reconciler mirrors the Runner's hosted-Hermes lifecycle for one
//! local Docker host: Core's host-scoped route targets, owned container labels
//! and saved loopback ports, the native `/api/status` probe, and the production
//! `finite-saas-runner render-hosted-hermes-caddy` renderer. Only PKI differs:
//! a temporary internal issuer for the synthetic origin host, trusted by no
//! system store. Every projection change restarts Caddy after confirmed exit.
//! Production's systemd address-reuse fence is out of scope for one local host.
//!
//! agentd accepts cleartext Core only on its own loopback, while production
//! guests reach Core's dedicated runtime listener over public HTTPS. The Core
//! bridge therefore runs a sidecar in each local runtime's network namespace
//! that forwards that loopback port to the same listener on the Docker host.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const NATIVE_HERMES_PORT: u16 = 8642;
const RUNTIME_LABEL: &str = "computer.finite.v2.runtime";
const SOURCE_HOST_LABEL: &str = "computer.finite.v2.source_host_id";
const SOURCE_MACHINE_LABEL: &str = "computer.finite.v2.source_machine_id";
const PROJECT_LABEL: &str = "computer.finite.v2.project_id";
pub const CORE_BRIDGE_SOURCE_HOST_LABEL: &str =
    "computer.finite.devfinity.core-bridge.source-host-id";
const CORE_BRIDGE_TARGET_LABEL: &str = "computer.finite.devfinity.core-bridge.target";
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);

/// A small TCP forwarder run by the runtime image's own Python.
const CORE_BRIDGE_FORWARDER: &str = r#"import asyncio, sys
listen_port, host, port = int(sys.argv[1]), sys.argv[2], int(sys.argv[3])
async def pipe(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
        if writer.can_write_eof():
            writer.write_eof()
    except OSError:
        pass
async def handle(client_reader, client_writer):
    try:
        upstream_reader, upstream_writer = await asyncio.open_connection(host, port)
    except OSError:
        client_writer.close()
        return
    await asyncio.gather(pipe(client_reader, upstream_writer), pipe(upstream_reader, client_writer))
    for writer in (client_writer, upstream_writer):
        writer.close()
async def main():
    server = await asyncio.start_server(handle, "127.0.0.1", listen_port)
    async with server:
        await server.serve_forever()
asyncio.run(main())
"#;

pub struct IngressConfig {
    pub core_url: String,
    /// Runner credential for the host-scoped route-target projection.
    pub runner_token: String,
    pub source_host_id: String,
    pub public_host: String,
    pub port: u16,
    pub allowed_origins: Vec<String>,
    pub admin_socket: PathBuf,
    pub state_dir: PathBuf,
    pub docker_bin: PathBuf,
    pub renderer_bin: PathBuf,
    pub caddy_bin: PathBuf,
}

impl IngressConfig {
    fn public_origin(&self) -> String {
        format!("https://{}:{}", self.public_host, self.port)
    }

    fn config_file(&self) -> PathBuf {
        self.state_dir.join("caddy.json")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteTarget {
    runtime_id: String,
    project_id: String,
    source_machine_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    runtime_id: String,
    host_port: u16,
}

/// Exactly the manifest the production renderer accepts; nothing else.
fn manifest(config: &IngressConfig, routes: &[Route]) -> Value {
    json!({
        "public_origin": config.public_origin(),
        "listen": format!("127.0.0.1:{}", config.port),
        "admin_socket": config.admin_socket,
        "allowed_origins": config.allowed_origins,
        "routes": routes.iter().map(|route| json!({
            "runtime_id": route.runtime_id,
            "host_port": route.host_port,
        })).collect::<Vec<_>>(),
    })
}

/// Retain the rendered HTTP routing and CORS verbatim; issue the synthetic
/// host's certificate from Caddy's local CA instead of a public ACME issuer.
fn with_local_pki(mut rendered: Value, public_host: &str) -> Result<Value> {
    let apps = rendered
        .get_mut("apps")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("rendered hosted Hermes configuration has no apps"))?;
    if apps.contains_key("tls") {
        bail!("rendered hosted Hermes configuration unexpectedly selects TLS issuance");
    }
    apps.insert(
        "tls".into(),
        json!({ "automation": { "policies": [{
            "subjects": [public_host],
            "issuers": [{ "module": "internal" }],
        }] } }),
    );
    Ok(rendered)
}

/// `docker port <container> 8642/tcp` must name exactly one loopback binding.
fn loopback_port(output: &str) -> Option<u16> {
    let mut bindings = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let port = bindings.next()?.strip_prefix("127.0.0.1:")?.parse().ok()?;
    (bindings.next().is_none() && port != 0).then_some(port)
}

fn owned_runtime(
    labels: &BTreeMap<String, String>,
    source_host: &str,
    target: &RouteTarget,
) -> bool {
    let label = |name: &str| labels.get(name).map(String::as_str);
    label(RUNTIME_LABEL) == Some("true")
        && label(SOURCE_HOST_LABEL) == Some(source_host)
        && label(SOURCE_MACHINE_LABEL) == Some(target.source_machine_id.as_str())
        && label(PROJECT_LABEL) == Some(target.project_id.as_str())
}

pub fn run_ingress(config: IngressConfig) -> Result<()> {
    let stop = stop_on_signal()?;
    std::fs::create_dir_all(config.state_dir.join("data"))?;
    std::fs::create_dir_all(config.state_dir.join("config"))?;
    let mut caddy: Option<Child> = None;
    let mut published: Option<Vec<u8>> = None;
    let mut last_report = String::new();
    while !stop.load(Ordering::SeqCst) {
        let (routes, report) = match projection(&config) {
            Ok(routes) => {
                let report = format!(
                    "serving {} route(s){}",
                    routes.len(),
                    routes
                        .iter()
                        .map(|route| format!(
                            "; /runtimes/{} -> 127.0.0.1:{}",
                            route.runtime_id, route.host_port
                        ))
                        .collect::<String>()
                );
                (routes, report)
            }
            // Fail closed for hosted traffic, as the Runner lifecycle does.
            Err(error) => (Vec::new(), format!("no routes: {error:#}")),
        };
        if report != last_report {
            println!(
                "hosted Hermes ingress {} at {}",
                report,
                config.public_origin()
            );
            last_report = report;
        }
        match render(&config, &routes) {
            Ok(bytes) => {
                let exited = caddy
                    .as_mut()
                    .is_none_or(|child| child.try_wait().ok().flatten().is_some());
                if exited || published.as_ref() != Some(&bytes) {
                    stop_caddy(&mut caddy);
                    write_atomically(&config.config_file(), &bytes)?;
                    caddy = Some(start_caddy(&config)?);
                    published = Some(bytes);
                }
            }
            Err(error) => eprintln!("hosted Hermes ingress render failed: {error:#}"),
        }
        sleep_until_stopped(&stop, RECONCILE_INTERVAL);
    }
    stop_caddy(&mut caddy);
    Ok(())
}

fn projection(config: &IngressConfig) -> Result<Vec<Route>> {
    let targets: Vec<RouteTarget> = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .redirects(0)
        .build()
        .get(&format!(
            "{}/api/core/v1/hosted-hermes-route-targets",
            config.core_url
        ))
        .set("authorization", &format!("Bearer {}", config.runner_token))
        .call()
        .map_err(|error| anyhow!("Core route eligibility is unavailable: {error}"))?
        .into_json()
        .context("Core route eligibility is malformed")?;
    let probe = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(500))
        .redirects(0)
        .build();
    let mut routes = Vec::new();
    for target in targets {
        let labels: BTreeMap<String, String> = serde_json::from_str(&docker_output(
            &config.docker_bin,
            &[
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                &target.source_machine_id,
            ],
        )?)
        .context("container labels are malformed")?;
        if !owned_runtime(&labels, &config.source_host_id, &target) {
            bail!("{} is not this host's runtime", target.source_machine_id);
        }
        let port = loopback_port(&docker_output(
            &config.docker_bin,
            &[
                "port",
                &target.source_machine_id,
                &format!("{NATIVE_HERMES_PORT}/tcp"),
            ],
        )?)
        .ok_or_else(|| anyhow!("{} has no loopback native port", target.source_machine_id))?;
        // Applied intent can predate a restart; also require the listener.
        if probe
            .get(&format!("http://127.0.0.1:{port}/api/status"))
            .call()
            .is_ok_and(|response| response.status() == 200)
        {
            routes.push(Route {
                runtime_id: target.runtime_id,
                host_port: port,
            });
        }
    }
    Ok(routes)
}

fn render(config: &IngressConfig, routes: &[Route]) -> Result<Vec<u8>> {
    let manifest_file = config.state_dir.join("manifest.json");
    write_atomically(
        &manifest_file,
        &serde_json::to_vec_pretty(&manifest(config, routes))?,
    )?;
    let output = Command::new(&config.renderer_bin)
        .arg("render-hosted-hermes-caddy")
        .arg("--manifest")
        .arg(&manifest_file)
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run the hosted Hermes Caddy renderer")?;
    if !output.status.success() {
        bail!("the hosted Hermes Caddy renderer rejected the manifest");
    }
    let rendered = serde_json::from_slice(&output.stdout).context("renderer output is not JSON")?;
    Ok(serde_json::to_vec_pretty(&with_local_pki(
        rendered,
        &config.public_host,
    )?)?)
}

fn start_caddy(config: &IngressConfig) -> Result<Child> {
    Command::new(&config.caddy_bin)
        .arg("run")
        .arg("--config")
        .arg(config.config_file())
        .env("HOME", &config.state_dir)
        .env("XDG_DATA_HOME", config.state_dir.join("data"))
        .env("XDG_CONFIG_HOME", config.state_dir.join("config"))
        .spawn()
        .context("failed to start Caddy for the hosted Hermes ingress")
}

/// Confirmed exit before republishing, never a reload acknowledgement.
fn stop_caddy(caddy: &mut Option<Child>) {
    let Some(mut child) = caddy.take() else {
        return;
    };
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub struct CoreBridgeConfig {
    pub source_host_id: String,
    pub port: u16,
    pub upstream_host: String,
    pub docker_bin: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeContainer {
    id: String,
    name: String,
    image: String,
    started_at: String,
}

impl RuntimeContainer {
    /// A restarted runtime has a new network namespace and needs a new bridge.
    fn bridge_target(&self) -> String {
        format!("{}@{}", self.id, self.started_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeContainer {
    id: String,
    target: String,
    running: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BridgePlan {
    remove: Vec<String>,
    create: Vec<RuntimeContainer>,
}

fn bridge_plan(runtimes: &[RuntimeContainer], bridges: &[BridgeContainer]) -> BridgePlan {
    let wanted = runtimes
        .iter()
        .map(RuntimeContainer::bridge_target)
        .collect::<BTreeSet<_>>();
    let mut serving = BTreeSet::new();
    let mut plan = BridgePlan::default();
    for bridge in bridges {
        if bridge.running && wanted.contains(&bridge.target) && serving.insert(&bridge.target) {
            continue;
        }
        plan.remove.push(bridge.id.clone());
    }
    plan.create = runtimes
        .iter()
        .filter(|runtime| !serving.contains(&runtime.bridge_target()))
        .cloned()
        .collect();
    plan
}

pub fn run_core_bridge(config: CoreBridgeConfig) -> Result<()> {
    let stop = stop_on_signal()?;
    while !stop.load(Ordering::SeqCst) {
        if let Err(error) = reconcile_bridges(&config) {
            eprintln!("runtime Core bridge reconciliation failed: {error:#}");
        }
        sleep_until_stopped(&stop, RECONCILE_INTERVAL);
    }
    // The bridge is meaningful only while this stack's Core runs.
    let bridges = bridge_containers(&config)?;
    remove_containers(&config.docker_bin, bridges.iter().map(|bridge| &bridge.id));
    Ok(())
}

fn reconcile_bridges(config: &CoreBridgeConfig) -> Result<()> {
    let plan = bridge_plan(&runtime_containers(config)?, &bridge_containers(config)?);
    remove_containers(&config.docker_bin, plan.remove.iter());
    for runtime in plan.create {
        let status = Command::new(&config.docker_bin)
            .args(bridge_run_args(config, &runtime))
            .stdout(Stdio::null())
            .status()
            .context("failed to start a runtime Core bridge")?;
        if status.success() {
            println!(
                "runtime Core bridge: {} 127.0.0.1:{port} -> {}:{port}",
                runtime.name,
                config.upstream_host,
                port = config.port
            );
        } else {
            eprintln!("runtime Core bridge for {} failed to start", runtime.name);
        }
    }
    Ok(())
}

/// Shares only the runtime's network namespace; the runtime image supplies
/// Python, so no extra image is pulled. Its HEALTHCHECK is not the bridge's.
fn bridge_run_args(config: &CoreBridgeConfig, runtime: &RuntimeContainer) -> Vec<String> {
    let port = config.port.to_string();
    let mut args = ["run", "--detach", "--rm", "--no-healthcheck", "--name"]
        .map(String::from)
        .to_vec();
    args.extend([
        format!("{}-core-bridge", runtime.name),
        "--network".into(),
        format!("container:{}", runtime.id),
        "--label".into(),
        format!("{CORE_BRIDGE_SOURCE_HOST_LABEL}={}", config.source_host_id),
        "--label".into(),
        format!("{CORE_BRIDGE_TARGET_LABEL}={}", runtime.bridge_target()),
        "--entrypoint".into(),
        "python3".into(),
        runtime.image.clone(),
        "-c".into(),
        CORE_BRIDGE_FORWARDER.into(),
        port.clone(),
        config.upstream_host.clone(),
        port,
    ]);
    args
}

#[derive(Deserialize)]
struct Inspected {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "State")]
    state: InspectedState,
    #[serde(rename = "Config")]
    config: InspectedConfig,
}

#[derive(Deserialize)]
struct InspectedState {
    #[serde(rename = "Running")]
    running: bool,
    #[serde(rename = "StartedAt")]
    started_at: String,
}

#[derive(Deserialize)]
struct InspectedConfig {
    #[serde(rename = "Labels", default)]
    labels: Option<BTreeMap<String, String>>,
}

fn inspect_listed(docker: &Path, filters: &[String], all: bool) -> Result<Vec<Inspected>> {
    let mut args = vec![
        "ps".to_string(),
        "--quiet".to_string(),
        "--no-trunc".to_string(),
    ];
    if all {
        args.push("--all".into());
    }
    for filter in filters {
        args.extend(["--filter".to_string(), filter.clone()]);
    }
    let listed = docker_output(docker, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    let ids = listed.split_whitespace().collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut inspect = vec!["inspect"];
    inspect.extend(ids);
    serde_json::from_str(&docker_output(docker, &inspect)?).context("docker inspect is malformed")
}

fn runtime_containers(config: &CoreBridgeConfig) -> Result<Vec<RuntimeContainer>> {
    let filters = [
        format!("label={RUNTIME_LABEL}=true"),
        format!("label={SOURCE_HOST_LABEL}={}", config.source_host_id),
    ];
    Ok(inspect_listed(&config.docker_bin, &filters, false)?
        .into_iter()
        .filter(|inspected| inspected.state.running)
        .map(|inspected| RuntimeContainer {
            name: inspected.name.trim_start_matches('/').to_string(),
            id: inspected.id,
            image: inspected.image,
            started_at: inspected.state.started_at,
        })
        .collect())
}

fn bridge_containers(config: &CoreBridgeConfig) -> Result<Vec<BridgeContainer>> {
    let filters = [format!(
        "label={CORE_BRIDGE_SOURCE_HOST_LABEL}={}",
        config.source_host_id
    )];
    Ok(inspect_listed(&config.docker_bin, &filters, true)?
        .into_iter()
        .map(|inspected| BridgeContainer {
            target: inspected
                .config
                .labels
                .unwrap_or_default()
                .remove(CORE_BRIDGE_TARGET_LABEL)
                .unwrap_or_default(),
            id: inspected.id,
            running: inspected.state.running,
        })
        .collect())
}

fn remove_containers<'a>(docker: &Path, ids: impl Iterator<Item = &'a String>) {
    for id in ids {
        let _ = Command::new(docker)
            .args(["rm", "--force", id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn docker_output(docker: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(docker)
        .args(args)
        .output()
        .with_context(|| format!("failed to run docker {}", args.first().unwrap_or(&"")))?;
    if !output.status.success() {
        bail!(
            "docker {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("docker output is not UTF-8")
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let next = path.with_extension("next");
    std::fs::write(&next, bytes).with_context(|| format!("failed to write {}", next.display()))?;
    std::fs::rename(&next, path).with_context(|| format!("failed to replace {}", path.display()))
}

/// Handlers are installed before returning so an early SIGTERM still lets
/// the loop stop Caddy or remove bridges instead of orphaning them.
fn stop_on_signal() -> Result<Arc<AtomicBool>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    #[cfg(unix)]
    let (mut interrupt, mut terminate) = {
        use tokio::signal::unix::{SignalKind, signal};
        let _context = runtime.enter();
        (
            signal(SignalKind::interrupt())?,
            signal(SignalKind::terminate())?,
        )
    };
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        runtime.block_on(async {
            #[cfg(unix)]
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        });
        flag.store(true, Ordering::SeqCst);
    });
    Ok(stop)
}

fn sleep_until_stopped(stop: &AtomicBool, duration: Duration) {
    let started = Instant::now();
    while started.elapsed() < duration && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests;
