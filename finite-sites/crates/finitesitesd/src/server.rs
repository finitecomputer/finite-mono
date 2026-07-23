//! HTTP server assembly: one listener, two planes.
//!
//! Requests whose Host matches the API host go to the control-plane API.
//! Requests whose Host is `{label}.{base_domain}` go to the site-serving
//! plane. Everything else (the bare listen address, a load balancer health
//! check) goes to the API. The API check runs first because in production
//! the API host (`api.finite.chat`) itself matches `*.finite.chat`; `api`
//! is also a reserved site name, so the two planes can never both claim a
//! host. The split is decided in one place, by host, before any route
//! matching.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::HOST;
use axum::response::Response;
use tower::util::ServiceExt as _;

use finitesites_blob::BlobStore;
use finitesites_engine::Engine;

use crate::apps::Supervisor;
use crate::identity::IdentityAuthority;
use crate::limiter::{RateLimiter, WINDOW_SECONDS};
use crate::mailer::Mailer;
use crate::{ServeOptions, api, git, sites};

const SERVING_ENGINE_POOL_SIZE: usize = 8;

pub struct AppState {
    /// The Engine owns the sole writable registry connection. This mutex
    /// serializes control-plane mutations; serving uses `serving_engines`.
    pub engine: Mutex<Engine>,
    /// Independent read-only SQLite connections for site traffic. The small
    /// mutex protects only idle-handle checkout; request work runs on Tokio's
    /// blocking pool and never holds the control-plane Engine mutex.
    pub serving_engines: ServingEnginePool,
    /// Immutable blob handle used outside the registry mutex. Content files
    /// are address-verified on every read, so concurrent serving cannot mix
    /// bytes across active versions.
    pub blobs: BlobStore,
    pub mailer: Box<dyn Mailer>,
    /// Owns app isolation (the runner) plus the density policy: wake on
    /// request, stop when idle.
    pub apps: Supervisor,
    pub login_limiter: RateLimiter,
    pub api_url: String,
    pub git_base_url: String,
    pub identity_authority: Option<IdentityAuthority>,
    pub viewer_session_service_token: Option<String>,
    pub base_domain: String,
    pub document_base_domain: String,
    pub data_dir: PathBuf,
    pub git_hook_helper_path: PathBuf,
    pub git_auto_reconcile: bool,
}

pub struct ServingEnginePool {
    available: Arc<Mutex<Vec<Engine>>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl ServingEnginePool {
    fn new(engine: &Engine, size: usize) -> Result<Self, finitesites_engine::EngineError> {
        assert!(size > 0);
        let mut available = Vec::with_capacity(size);
        for _ in 0..size {
            available.push(engine.serving_reader()?);
        }
        Ok(Self {
            available: Arc::new(Mutex::new(available)),
            permits: Arc::new(tokio::sync::Semaphore::new(size)),
        })
    }

    pub async fn run<F, R>(&self, work: F) -> Result<R, String>
    where
        F: FnOnce(&Engine) -> R + Send + 'static,
        R: Send + 'static,
    {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "serving engine pool closed".to_string())?;
        let engine = self
            .available
            .lock()
            .expect("serving engine pool mutex never poisoned")
            .pop()
            .expect("permit guarantees an available serving engine");
        let available = Arc::clone(&self.available);
        tokio::task::spawn_blocking(move || {
            let lease = ServingEngineLease {
                engine: Some(engine),
                available,
                _permit: permit,
            };
            work(
                lease
                    .engine
                    .as_ref()
                    .expect("serving engine lease owns engine"),
            )
        })
        .await
        .map_err(|error| format!("serving engine task failed: {error}"))
    }
}

struct ServingEngineLease {
    engine: Option<Engine>,
    available: Arc<Mutex<Vec<Engine>>>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for ServingEngineLease {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            self.available
                .lock()
                .expect("serving engine pool mutex never poisoned")
                .push(engine);
        }
    }
}

pub fn now_unix() -> u64 {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    assert!(now > 0);
    now as u64
}

#[derive(Clone)]
struct Dispatcher {
    api: Router,
    git: Router,
    sites: Router,
    base_domain: String,
    document_base_domain: String,
    /// Port-stripped host of the configured `--api-url`, checked before the
    /// wildcard so `api.finite.chat` never falls into the sites plane.
    api_host: String,
    git_host: String,
}

pub fn build_app(state: Arc<AppState>) -> Router {
    let dispatcher = Dispatcher {
        api: api::router(state.clone()),
        git: git::router(state.clone()),
        sites: sites::router(state.clone()),
        base_domain: state.base_domain.clone(),
        document_base_domain: state.document_base_domain.clone(),
        api_host: host_of_url(&state.api_url),
        git_host: host_of_url(&state.git_base_url),
    };
    Router::new().fallback(dispatch).with_state(dispatcher)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plane {
    Api,
    Git,
    Sites,
}

/// The one routing decision: which plane serves this Host header.
pub fn plane_for_host(
    host: &str,
    api_host: &str,
    git_host: &str,
    base_domain: &str,
    document_base_domain: &str,
) -> Plane {
    if strip_port(host).eq_ignore_ascii_case(api_host) {
        return Plane::Api;
    }
    if strip_port(host).eq_ignore_ascii_case(git_host) {
        return Plane::Git;
    }
    if site_label(host, base_domain).is_some() {
        return Plane::Sites;
    }
    if site_label(host, document_base_domain).is_some() {
        return Plane::Sites;
    }
    Plane::Api
}

/// Route Git smart-HTTP paths on a shared API/Git origin. Production keeps
/// separate hosts, while constrained local guests may only be able to reach
/// one gateway IP and port. The strict `/{slug}.git[/...]` parser prevents
/// ordinary API paths from being reclassified.
pub fn plane_for_request(
    host: &str,
    path: &str,
    api_host: &str,
    git_host: &str,
    base_domain: &str,
    document_base_domain: &str,
) -> Plane {
    let request_host = strip_port(host);
    if api_host.eq_ignore_ascii_case(git_host)
        && request_host.eq_ignore_ascii_case(api_host)
        && git::is_git_request_path(path)
    {
        return Plane::Git;
    }
    plane_for_host(host, api_host, git_host, base_domain, document_base_domain)
}

async fn dispatch(State(dispatcher): State<Dispatcher>, request: Request<Body>) -> Response {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let router = match plane_for_request(
        host,
        request.uri().path(),
        &dispatcher.api_host,
        &dispatcher.git_host,
        &dispatcher.base_domain,
        &dispatcher.document_base_domain,
    ) {
        Plane::Sites => dispatcher.sites.clone(),
        Plane::Git => dispatcher.git.clone(),
        Plane::Api => dispatcher.api.clone(),
    };
    match router.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// Host (no port) of a URL like `https://api.finite.chat` or
/// `http://127.0.0.1:8787`.
pub fn host_of_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host_and_port = after_scheme.split(['/', '?']).next().unwrap_or("");
    strip_port(host_and_port).to_ascii_lowercase()
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        // IPv6 literal: [::1]:8787 -> [::1]
        return host
            .split_once("]:")
            .map(|(left, _)| &host[..left.len() + 1])
            .unwrap_or(host);
    }
    match host.rsplit_once(':') {
        Some((left, right)) if right.bytes().all(|b| b.is_ascii_digit()) => left,
        _ => host,
    }
}

/// Extract the site label from a Host header value: `hello.sites.localhost`
/// with base domain `sites.localhost` yields `hello`. Ports are stripped.
/// Multi-level labels (`a.b.sites.localhost`) are rejected: one wildcard
/// level keeps certificates and cookies simple.
pub fn site_label(host: &str, base_domain: &str) -> Option<String> {
    if host.is_empty() || host.starts_with('[') {
        // IPv6 literals are never site hosts.
        return None;
    }
    let without_port = match host.rsplit_once(':') {
        Some((left, right)) if right.bytes().all(|b| b.is_ascii_digit()) => left,
        _ => host,
    };
    let label = without_port.strip_suffix(base_domain)?.strip_suffix('.')?;
    if label.is_empty() || label.contains('.') {
        return None;
    }
    Some(label.to_ascii_lowercase())
}

pub async fn serve(
    engine: Engine,
    mailer: Box<dyn Mailer>,
    apps: Supervisor,
    options: ServeOptions,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(options.listen)
        .await
        .map_err(|error| format!("cannot bind {}: {error}", options.listen))?;
    serve_on(listener, engine, mailer, apps, options).await
}

/// Serve on an already-bound listener. Split from `serve` so tests can bind
/// an ephemeral port first and build options around the real address.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    engine: Engine,
    mailer: Box<dyn Mailer>,
    apps: Supervisor,
    options: ServeOptions,
) -> Result<(), String> {
    crate::validate_viewer_session_service_token(options.viewer_session_service_token.as_deref())?;
    git::preflight_git_dependency().map_err(|error| {
        format!(
            "Git dependency preflight failed: {error}. Install Git and make it available on PATH"
        )
    })?;
    let blobs = engine.blob_store();
    let serving_engines = ServingEnginePool::new(&engine, SERVING_ENGINE_POOL_SIZE)
        .map_err(|error| format!("cannot open serving registry readers: {error}"))?;
    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        serving_engines,
        blobs,
        mailer,
        apps,
        login_limiter: RateLimiter::new(WINDOW_SECONDS),
        api_url: options.api_url.clone(),
        git_base_url: options.git_base_url.clone(),
        identity_authority: options
            .identity_authority_url
            .as_ref()
            .map(IdentityAuthority::new),
        viewer_session_service_token: options.viewer_session_service_token.clone(),
        base_domain: options.base_domain.clone(),
        document_base_domain: options.document_base_domain.clone(),
        data_dir: options.data_dir.clone(),
        git_hook_helper_path: options.git_hook_helper_path.clone(),
        git_auto_reconcile: options.git_auto_reconcile,
    });
    reconcile_apps(&state);
    reconcile_git_projects(&state);
    spawn_idle_reaper(state.clone());
    let app = build_app(state);
    eprintln!(
        "finitesitesd listening on {} (api: {}, git: {}, sites: *.{})",
        options.listen, options.api_url, options.git_base_url, options.base_domain
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("server error: {error}"))
}

fn reconcile_git_projects(state: &Arc<AppState>) {
    if !state.git_auto_reconcile {
        return;
    }
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    match git::reconcile_pending_events_with_apps(
        &mut engine,
        &state.data_dir,
        None,
        now_unix(),
        Some(&state.apps),
    ) {
        Ok(processed) if processed > 0 => {
            eprintln!("git reconcile: {processed} pending event(s) processed");
        }
        Ok(_) => {}
        Err(error) => eprintln!("git reconcile failed: {error}"),
    }
}

/// Bring every app site with an active version back up after a daemon
/// restart. Failures are logged, not fatal: one broken app must not stop
/// the platform from serving.
fn reconcile_apps(state: &Arc<AppState>) {
    let engine = state.engine.lock().expect("engine mutex never poisoned");
    let deploys = match engine.app_deploys() {
        Ok(deploys) => deploys,
        Err(error) => {
            eprintln!("app reconcile: cannot list app sites: {error}");
            return;
        }
    };
    // Bounded by the app port range.
    for deploy in &deploys {
        let bundle_path = engine.blob_file_path(&deploy.bundle_sha256);
        if let Err(error) = state.apps.deploy(deploy, &bundle_path, now_unix()) {
            eprintln!("app reconcile: {} failed: {error}", deploy.site_id);
        }
    }
    if !deploys.is_empty() {
        eprintln!("app reconcile: {} app site(s) processed", deploys.len());
    }
}

/// Periodically stop apps that have been idle past the timeout. This is the
/// density mechanism: idle tenants cost ~0 memory and wake on the next
/// request. The check runs every minute; reaping itself is bounded by the
/// app count.
fn spawn_idle_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let state = state.clone();
            // Runner calls (systemctl/ctr) are blocking; keep them off the
            // async reactor.
            let _ = tokio::task::spawn_blocking(move || {
                let deploys = {
                    let engine = state.engine.lock().expect("engine mutex");
                    engine.app_deploys()
                };
                let deploys = match deploys {
                    Ok(deploys) => deploys,
                    Err(error) => {
                        eprintln!("idle reaper: cannot list apps: {error}");
                        return;
                    }
                };
                let stopped = state.apps.reap_idle(&deploys, now_unix());
                if !stopped.is_empty() {
                    eprintln!("idle reaper: stopped {} idle app(s)", stopped.len());
                }
            })
            .await;
        }
    });
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("finitesitesd shutting down");
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ServingEnginePool, host_of_url, site_label, strip_port};

    #[tokio::test]
    async fn serving_pool_does_not_head_of_line_block_independent_reads() {
        let directory = tempfile::tempdir().unwrap();
        let store = finitesites_store::Store::open(&directory.path().join("registry.db")).unwrap();
        let blobs = finitesites_blob::BlobStore::open(&directory.path().join("blobs")).unwrap();
        let engine = finitesites_engine::Engine::new(
            store,
            blobs,
            [7; 32],
            finitesites_engine::EngineConfig {
                base_domain: "sites.test".to_string(),
                document_base_domain: "docs.sites.test".to_string(),
                site_url_scheme: "https".to_string(),
                site_url_port: None,
            },
        );
        let pool = Arc::new(ServingEnginePool::new(&engine, 2).unwrap());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slow_pool = Arc::clone(&pool);
        let slow = tokio::spawn(async move {
            slow_pool
                .run(move |_| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .await
                .unwrap();
        });
        started_rx.await.unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            pool.run(|_| "independent"),
        )
        .await
        .expect("second serving reader must not wait for the first");
        assert_eq!(result.unwrap(), "independent");

        release_tx.send(()).unwrap();
        slow.await.unwrap();
    }

    #[tokio::test]
    async fn serving_pool_keeps_permit_until_cancelled_blocking_work_returns() {
        let directory = tempfile::tempdir().unwrap();
        let store = finitesites_store::Store::open(&directory.path().join("registry.db")).unwrap();
        let blobs = finitesites_blob::BlobStore::open(&directory.path().join("blobs")).unwrap();
        let engine = finitesites_engine::Engine::new(
            store,
            blobs,
            [7; 32],
            finitesites_engine::EngineConfig {
                base_domain: "sites.test".to_string(),
                document_base_domain: "docs.sites.test".to_string(),
                site_url_scheme: "https".to_string(),
                site_url_port: None,
            },
        );
        let pool = Arc::new(ServingEnginePool::new(&engine, 1).unwrap());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slow_pool = Arc::clone(&pool);
        let slow = tokio::spawn(async move {
            slow_pool
                .run(move |_| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .await
                .unwrap();
        });
        started_rx.await.unwrap();

        slow.abort();
        assert!(slow.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                pool.run(|_| "must wait"),
            )
            .await
            .is_err(),
            "cancelling the async caller must not release the blocking worker's permit"
        );

        release_tx.send(()).unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            pool.run(|_| "available again"),
        )
        .await
        .expect("serving engine returns after cancelled blocking work finishes")
        .unwrap();
        assert_eq!(result, "available again");
    }

    #[test]
    fn host_of_url_extraction() {
        assert_eq!(host_of_url("https://api.finite.chat"), "api.finite.chat");
        assert_eq!(host_of_url("http://127.0.0.1:8787"), "127.0.0.1");
        assert_eq!(
            host_of_url("https://API.Finite.Chat/path?q=1"),
            "api.finite.chat"
        );
    }

    #[test]
    fn strip_port_handles_ipv6() {
        assert_eq!(strip_port("api.finite.chat:443"), "api.finite.chat");
        assert_eq!(strip_port("api.finite.chat"), "api.finite.chat");
        assert_eq!(strip_port("[::1]:8787"), "[::1]");
        assert_eq!(strip_port("[::1]"), "[::1]");
    }

    // The production-shaped regression: api.finite.chat matches the
    // *.finite.chat wildcard but must classify as the API host.
    #[test]
    fn api_host_wins_over_wildcard() {
        use super::{Plane, plane_for_host};
        let base = "finite.chat";
        let document_base = "docs.finite.chat";
        let api_host = host_of_url("https://api.finite.chat");
        let git_host = host_of_url("https://git.finite.chat");
        assert_eq!(
            plane_for_host("api.finite.chat", &api_host, &git_host, base, document_base),
            Plane::Api
        );
        assert_eq!(
            plane_for_host(
                "api.finite.chat:443",
                &api_host,
                &git_host,
                base,
                document_base
            ),
            Plane::Api
        );
        assert_eq!(
            plane_for_host("API.finite.chat", &api_host, &git_host, base, document_base),
            Plane::Api
        );
        assert_eq!(
            plane_for_host("git.finite.chat", &api_host, &git_host, base, document_base),
            Plane::Git
        );
        assert_eq!(
            plane_for_host(
                "hello.finite.chat",
                &api_host,
                &git_host,
                base,
                document_base
            ),
            Plane::Sites
        );
        assert_eq!(
            plane_for_host(
                "hello.docs.finite.chat",
                &api_host,
                &git_host,
                base,
                document_base
            ),
            Plane::Sites
        );
        assert_eq!(
            plane_for_host("finite.chat", &api_host, &git_host, base, document_base),
            Plane::Api
        );
        assert_eq!(
            plane_for_host("127.0.0.1:8787", &api_host, &git_host, base, document_base),
            Plane::Api
        );
    }

    #[test]
    fn shared_api_and_git_origin_routes_only_repository_paths_to_git() {
        use super::{Plane, plane_for_request};

        let host = host_of_url("http://192.168.64.1:8787");
        assert_eq!(
            plane_for_request(
                "192.168.64.1:8787",
                "/demo.git/info/refs",
                &host,
                &host,
                "sites.localhost",
                "docs.sites.localhost",
            ),
            Plane::Git
        );
        assert_eq!(
            plane_for_request(
                "192.168.64.1:8787",
                "/api/v1/projects",
                &host,
                &host,
                "sites.localhost",
                "docs.sites.localhost",
            ),
            Plane::Api
        );
        assert_eq!(
            plane_for_request(
                "192.168.64.1:8787",
                "/demo.gitx/info/refs",
                &host,
                &host,
                "sites.localhost",
                "docs.sites.localhost",
            ),
            Plane::Api
        );
    }

    #[test]
    fn distinct_api_host_does_not_steal_git_shaped_paths() {
        use super::{Plane, plane_for_request};

        assert_eq!(
            plane_for_request(
                "api.finite.chat",
                "/demo.git/info/refs",
                "api.finite.chat",
                "git.finite.chat",
                "finite.chat",
                "docs.finite.chat",
            ),
            Plane::Api
        );
    }

    #[test]
    fn site_label_extraction() {
        let base = "sites.localhost";
        assert_eq!(
            site_label("hello.sites.localhost", base),
            Some("hello".into())
        );
        assert_eq!(
            site_label("hello.sites.localhost:8787", base),
            Some("hello".into())
        );
        assert_eq!(
            site_label("HELLO.sites.localhost", base),
            Some("hello".into())
        );
        assert_eq!(site_label("sites.localhost", base), None);
        assert_eq!(site_label("a.b.sites.localhost", base), None);
        assert_eq!(site_label("127.0.0.1:8787", base), None);
        assert_eq!(site_label("evil-sites.localhost", base), None);
        assert_eq!(site_label("[::1]:8787", base), None);
        assert_eq!(site_label("", base), None);
    }
}
