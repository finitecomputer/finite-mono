//! Runtime-owned Iroh endpoint; Core admissions are the only remote authority.
use crate::core_registration::AdmissionRefresh;
use crate::{AgentdError, CoreEndpointRegistration, EndpointRegistrationOutcome, Ledger};
use iroh::{
    Endpoint, EndpointId, RelayMode, RelayUrl,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_proxy_utils::{
    ALPN, HttpProxyRequest, HttpProxyRequestKind,
    upstream::{AuthError, AuthHandler, UpstreamProxy},
};
use n0_error::Result;
use std::path::PathBuf;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Debug, Default)]
struct State {
    peers: HashMap<EndpointId, Instant>,
    connections: HashMap<usize, Connection>,
}
impl State {
    fn allowed(&self, id: EndpointId) -> bool {
        self.peers
            .get(&id)
            .is_some_and(|until| *until > Instant::now())
    }
    fn close_denied(&mut self) {
        for c in self.connections.values() {
            if !self.allowed(c.remote_id()) {
                c.close(403u32.into(), b"admission ended");
            }
        }
    }
}
#[derive(Debug, Clone)]
struct Acl {
    state: Arc<Mutex<State>>,
    target: String,
}
impl AuthHandler for Acl {
    async fn authorize(&self, id: EndpointId, request: &HttpProxyRequest) -> Result<(), AuthError> {
        let target_ok = matches!(&request.kind,HttpProxyRequestKind::Tunnel{target} if target.to_string()==self.target);
        if target_ok && self.state.lock().unwrap().allowed(id) {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }
}
#[derive(Debug)]
struct Guard {
    state: Arc<Mutex<State>>,
    id: usize,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.state.lock().unwrap().connections.remove(&self.id);
    }
}
#[derive(Debug)]
struct Enforced {
    state: Arc<Mutex<State>>,
    proxy: UpstreamProxy,
}
impl ProtocolHandler for Enforced {
    async fn accept(&self, c: Connection) -> std::result::Result<(), AcceptError> {
        let id = c.stable_id();
        {
            let mut state = self.state.lock().unwrap();
            // Registration and admission check share the revocation lock.
            if state.connections.len() >= 256 || !state.allowed(c.remote_id()) {
                c.close(403u32.into(), b"not admitted");
                return Ok(());
            }
            state.connections.insert(id, c.clone());
        }
        let _guard = Guard {
            state: self.state.clone(),
            id,
        };
        self.proxy.accept(c).await
    }
    async fn shutdown(&self) {
        self.proxy.shutdown().await;
    }
}

// Configuration contains bootstrap material and deliberately has no Debug impl.
pub struct IrohConfig {
    core_url: String,
    credential: String,
    relay: RelayUrl,
    ledger_path: PathBuf,
    port: u16,
}
impl IrohConfig {
    pub fn from_env() -> std::result::Result<Self, AgentdError> {
        let required = |key: &str| {
            std::env::var(key).map_err(|_| AgentdError::Config(format!("{key} is required")))
        };
        let relay = required("FINITE_IROH_RELAY_URL")?
            .parse::<RelayUrl>()
            .map_err(|_| AgentdError::Config("invalid Iroh relay URL".into()))?;
        let home = std::env::var("FINITECHAT_HOME")
            .or_else(|_| std::env::var("FINITE_AGENT_HOME"))
            .unwrap_or_else(|_| "/data/agent".into());
        let port = 8642;
        Ok(Self {
            core_url: required("FINITE_CORE_URL")?,
            credential: required("FINITE_CORE_CREDENTIAL")?,
            relay,
            ledger_path: PathBuf::from(home).join("agentd/agentd.sqlite3"),
            port,
        })
    }
}

pub async fn run_iroh(config: IrohConfig) -> std::result::Result<(), AgentdError> {
    let map_error = |_| AgentdError::Transport("Iroh transport failed".into());
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(config.relay.clone().into()))
        .bind()
        .await
        .map_err(map_error)?;
    let registration = CoreEndpointRegistration::new(
        &config.core_url,
        config.credential,
        &Ledger::open(config.ledger_path)?,
        endpoint.id().to_string(),
        config.relay.to_string(),
    )?;
    let state = Arc::new(Mutex::new(State::default()));
    let router = Router::builder(endpoint.clone())
        .accept(
            ALPN,
            Enforced {
                state: state.clone(),
                proxy: UpstreamProxy::new(Acl {
                    state: state.clone(),
                    target: format!("127.0.0.1:{}", config.port),
                })
                .map_err(|_| AgentdError::Transport("Iroh proxy setup failed".into()))?,
            },
        )
        .spawn();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    // Refresh cannot block expiry enforcement: its HTTP request is a separate
    // future while this loop continues ticking and handling shutdown.
    let refresh = async {
        loop {
            match registration.register().await {
                Ok(EndpointRegistrationOutcome::Accepted) => break,
                Ok(EndpointRegistrationOutcome::Superseded) => return,
                _ => tokio::time::sleep(Duration::from_secs(5)).await,
            }
        }
        loop {
            let started = Instant::now();
            match registration.admissions().await {
                Ok(AdmissionRefresh::Snapshot(snapshot)) => {
                    let peers: std::result::Result<HashMap<_, _>, _> = snapshot
                        .peers
                        .into_iter()
                        .map(|peer| {
                            peer.peer_id.parse::<EndpointId>().map(|id| {
                                (
                                    id,
                                    started + Duration::from_secs(peer.expires_in_seconds.min(300)),
                                )
                            })
                        })
                        .collect();
                    match peers {
                        Ok(peers) => {
                            let mut acl = state.lock().unwrap();
                            acl.peers = peers;
                            acl.close_denied();
                        }
                        Err(_) => {
                            let mut acl = state.lock().unwrap();
                            acl.peers.clear();
                            acl.close_denied();
                        }
                    }
                }
                Ok(AdmissionRefresh::Superseded) => return,
                Ok(AdmissionRefresh::Unauthorized) => {
                    let mut acl = state.lock().unwrap();
                    acl.peers.clear();
                    acl.close_denied();
                }
                Err(_) => {} // Preserve only the previously granted deadline.
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    };
    tokio::pin!(refresh);
    let mut expiry = tokio::time::interval(Duration::from_millis(100));
    let authority_ended = loop {
        tokio::select! {
            _ = &mut refresh => break true,
            _ = terminate.recv() => break false,
            _ = tokio::signal::ctrl_c() => break false,
            _ = expiry.tick() => {
                let mut acl = state.lock().unwrap();
                acl.peers.retain(|_, until| *until > Instant::now());
                acl.close_denied();
            }
        }
    };
    endpoint.close().await;
    router
        .shutdown()
        .await
        .map_err(|_| AgentdError::Transport("Iroh shutdown failed".into()))?;
    if authority_ended {
        // A superseded generation must never compete by restarting itself with
        // a newer one. Stay closed until the owning supervisor shuts us down.
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
    }
    Ok(())
}
