//! Render the dedicated hosted-Hermes edge from an ephemeral Runner projection.
//!
//! This is transport configuration, not a publication registry or authorization
//! decision. The caller must establish canonical runtime ownership and reserve
//! each upstream address for its entire published lifetime. Before an address
//! can belong to another runtime, the previous Caddy process must have exited:
//! an acknowledged reload does not drain accepted HTTP requests or their retries.
//! Kata discovery, that process-exit fence, and interrupted-operation recovery
//! are not integrated by this module. Do not activate it from the lease cycle.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_ROUTES: usize = 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostedHermesRouteManifest {
    pub public_origin: String,
    pub listen: SocketAddr,
    /// A socket in a Runner-owned private directory, not Caddy's TCP admin API.
    pub admin_socket: PathBuf,
    /// Exact browser origins permitted to read native API responses. Empty is
    /// closed to cross-origin browsers; HTTP is restricted to local development.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    pub routes: Vec<HostedHermesRoute>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostedHermesRoute {
    pub runtime_id: String,
    /// The already-reserved host port of this runtime's native Hermes listener.
    /// No remote host, upstream URL, credentials, or proxy headers are accepted.
    pub host_port: u16,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid hosted Hermes route manifest: {0}")]
pub struct InvalidManifest(&'static str);

impl HostedHermesRouteManifest {
    /// Produce stable Caddy JSON without modifying any file or running process.
    pub fn caddy_config(&self) -> Result<Value, InvalidManifest> {
        let origin =
            finite_saas_core::hosted_hermes::parse_hosted_hermes_origin(&self.public_origin)
                .map_err(InvalidManifest)?;
        let host = origin.host_str().expect("validated HTTPS origin");
        // URL brackets delimit an IPv6 authority; Caddy matches the hostname.
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        let port = origin.port_or_known_default().expect("HTTPS port");
        if self.listen.port() != port {
            return Err(InvalidManifest("listener port must match the HTTPS origin"));
        }
        if host == "localhost" && !self.listen.ip().is_loopback() {
            return Err(InvalidManifest("localhost requires a loopback listener"));
        }
        let admin_socket = self
            .admin_socket
            .to_str()
            .filter(|path| {
                path.starts_with('/')
                    && path.len() <= 100
                    && path.split('/').skip(1).all(|part| {
                        !part.is_empty()
                            && part != "."
                            && part != ".."
                            && part.bytes().all(|b| {
                                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
                            })
                    })
            })
            .ok_or(InvalidManifest(
                "admin socket must be a short, absolute plain path",
            ))?;
        if self.routes.len() > MAX_ROUTES {
            return Err(InvalidManifest("too many routes"));
        }
        let allowed_origins = validate_browser_origins(&self.allowed_origins)?;
        let mut ids = BTreeSet::new();
        let mut ports = BTreeSet::new();
        for route in &self.routes {
            if !valid_runtime_id(&route.runtime_id) {
                return Err(InvalidManifest(
                    "runtime ID must be a simple nonempty identifier",
                ));
            }
            if !ids.insert(&route.runtime_id) {
                return Err(InvalidManifest("duplicate runtime ID"));
            }
            if route.host_port == 0 || route.host_port == self.listen.port() {
                return Err(InvalidManifest(
                    "upstream port is zero or aliases the edge listener",
                ));
            }
            if !ports.insert(route.host_port) {
                return Err(InvalidManifest("duplicate upstream port"));
            }
        }

        let mut entries: Vec<_> = self.routes.iter().collect();
        entries.sort_by(|left, right| left.runtime_id.cmp(&right.runtime_id));
        let mut routes: Vec<Value> = entries
            .into_iter()
            .map(|entry| {
                let prefix = format!("/runtimes/{}", entry.runtime_id);
                let mut native_routes = vec![json!({ "handle": [{
                    "handler": "headers", "response": {
                        "add": { "Vary": ["Origin"] }, "deferred": true
                    }
                }] })];
                for origin in &allowed_origins {
                    native_routes.push(json!({
                        "match": [{ "header": { "Origin": [origin] } }],
                        "handle": [{ "handler": "headers", "response": {
                            "set": { "Access-Control-Allow-Origin": [origin] },
                            "deferred": true
                        } }]
                    }));
                    native_routes.push(json!({
                        "match": [{ "method": ["OPTIONS"], "header": {
                            "Origin": [origin], "Access-Control-Request-Method": ["*"]
                        } }],
                        "handle": [{ "handler": "static_response", "status_code": 204,
                            "headers": {
                                "Access-Control-Allow-Methods": ["GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"],
                                "Access-Control-Allow-Headers": ["Authorization, Content-Type"],
                                "Access-Control-Max-Age": ["300"]
                            }
                        }]
                    }));
                }
                native_routes.push(json!({ "handle": [
                    { "handler": "rewrite", "strip_path_prefix": &prefix },
                    {
                        "handler": "reverse_proxy",
                        "upstreams": [{ "dial": format!("127.0.0.1:{}", entry.host_port) }],
                        "headers": {
                            "request": { "set": { "X-Forwarded-Prefix": [&prefix] } },
                            // The explicit edge policy replaces native localhost
                            // CORS. Origin and native authentication pass through.
                            "response": { "delete": [
                                "Access-Control-Allow-Origin", "Access-Control-Allow-Credentials",
                                "Access-Control-Allow-Methods", "Access-Control-Allow-Headers",
                                "Access-Control-Expose-Headers", "Access-Control-Max-Age"
                            ] }
                        }
                    }
                ] }));
                json!({
                    "match": [{ "path": [&prefix, format!("{prefix}/*")] }],
                    "terminal": true,
                    "handle": [{ "handler": "subroute", "routes": native_routes }]
                })
            })
            .collect();
        routes.push(json!({ "handle": [{ "handler": "static_response", "status_code": 404 }] }));
        Ok(json!({
            "admin": {
                "listen": format!("unix/{admin_socket}"),
                "config": { "persist": false }
            },
            // Proxy failures can otherwise log WS tickets in a URI or protocol
            // header even with access logging disabled. Preserve diagnostics,
            // but remove the request object from the default structured log.
            "logging": { "logs": { "default": { "encoder": {
                "format": "filter", "wrap": { "format": "json" },
                "fields": { "request": { "filter": "delete" } }
            } } } },
            "apps": {
                "http": { "servers": { "hosted_hermes": {
                    "listen": [self.listen.to_string()],
                    "automatic_https": { "disable_redirects": true },
                    "routes": [{
                        "match": [{ "host": [host] }],
                        "terminal": true,
                        "handle": [{ "handler": "subroute", "routes": routes }]
                    }, { "handle": [{ "handler": "static_response", "status_code": 404 }] }]
                } } },
                // In a local HTTPS environment, trust the generated CA only in
                // that client/process. Never install a CA into the host store.
                "pki": { "certificate_authorities": { "local": { "install_trust": false } } }
            }
        }))
    }
}

fn validate_browser_origins(values: &[String]) -> Result<BTreeSet<String>, InvalidManifest> {
    if values.len() > 32 {
        return Err(InvalidManifest("too many browser origins"));
    }
    let mut origins = BTreeSet::new();
    for value in values {
        // Reuse the strict literal-origin parser while separately restricting
        // cleartext HTTP to explicit loopback development origins.
        let https_value = value
            .strip_prefix("http://")
            .map(|rest| format!("https://{rest}"));
        let parsed = finite_saas_core::hosted_hermes::parse_hosted_hermes_origin(
            https_value.as_deref().unwrap_or(value),
        )
        .map_err(InvalidManifest)?;
        if https_value.is_some()
            && !parsed.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        {
            return Err(InvalidManifest("HTTP browser origins must be loopback"));
        }
        let canonical = parsed
            .join(value)
            .map_err(|_| InvalidManifest("invalid browser origin"))?
            .origin()
            .ascii_serialization();
        if *value != canonical || !origins.insert(canonical) {
            return Err(InvalidManifest(
                "browser origins must be unique canonical origins",
            ));
        }
    }
    Ok(origins)
}

fn valid_runtime_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> HostedHermesRouteManifest {
        HostedHermesRouteManifest {
            public_origin: "https://agents.lat3.finite.computer".into(),
            listen: "0.0.0.0:443".parse().unwrap(),
            admin_socket: "/run/finite-hermes-caddy/admin.sock".into(),
            allowed_origins: vec!["https://finite.computer".into()],
            routes: vec![HostedHermesRoute {
                runtime_id: "runtime_1".into(),
                host_port: 30000,
            }],
        }
    }

    #[test]
    fn rejects_ambiguous_identity_upstream_and_admin_targets() {
        for id in [
            "", ".", "..", "a/b", "a%2Fb", "a?b", "a*b", "{host}", "a\nb",
        ] {
            let mut value = manifest();
            value.routes[0].runtime_id = id.into();
            assert!(value.caddy_config().is_err(), "accepted {id:?}");
        }
        for path in [
            "admin.sock",
            "/run/../admin.sock",
            "/run//admin.sock",
            "/run/a:1",
            "/run/a\nb",
        ] {
            let mut value = manifest();
            value.admin_socket = path.into();
            assert!(value.caddy_config().is_err(), "accepted {path:?}");
        }
        for port in [0, 443] {
            let mut value = manifest();
            value.routes[0].host_port = port;
            assert!(value.caddy_config().is_err());
        }
        for second in [
            HostedHermesRoute {
                runtime_id: "runtime_1".into(),
                host_port: 30001,
            },
            HostedHermesRoute {
                runtime_id: "runtime_2".into(),
                host_port: 30000,
            },
        ] {
            let mut value = manifest();
            value.routes.push(second);
            assert!(value.caddy_config().is_err());
        }
    }

    #[test]
    fn rejects_non_origin_or_untrusted_caddy_expression_syntax() {
        for origin in [
            "http://agents.example.com",
            "https://a.example/path",
            "https://user@a.example",
            "https://a.example?x",
            "https://a.example#x",
            "https://*.example",
            "https://{host}",
            "https://a.example:0",
            "https://a.example:65536",
            "https://a.example:443:443",
        ] {
            let mut value = manifest();
            value.public_origin = origin.into();
            assert!(value.caddy_config().is_err(), "accepted {origin:?}");
        }
        let mut value = manifest();
        value.listen = "127.0.0.1:8443".parse().unwrap();
        assert!(value.caddy_config().is_err());
        value.public_origin = "https://localhost:8443".into();
        assert!(value.caddy_config().is_ok());
        value.listen = "0.0.0.0:8443".parse().unwrap();
        assert!(value.caddy_config().is_err());
    }

    #[test]
    fn browser_origins_are_explicit_and_cleartext_is_local_only() {
        for origin in [
            "*",
            "null",
            "https://*.finite.computer",
            "https://{host}",
            "https://finite.computer/",
            "https://finite.computer/path",
            "https://user@finite.computer",
            "https://finite.computer?x",
            "http://finite.computer",
            "http://127.0.0.1.evil.test",
            "https://finite.computer\n",
            "https://finite.computer:0",
        ] {
            let mut value = manifest();
            value.allowed_origins = vec![origin.into()];
            assert!(value.caddy_config().is_err(), "accepted {origin:?}");
        }
        let mut value = manifest();
        value.allowed_origins = vec![
            "http://localhost:3000".into(),
            "http://127.0.0.1:13003".into(),
            "http://[::1]:3000".into(),
            "http://localhost:443".into(),
        ];
        assert!(value.caddy_config().is_ok());
        value.allowed_origins.push("http://localhost:3000".into());
        assert!(value.caddy_config().is_err());
    }

    #[test]
    fn full_native_surface_has_one_prefix_and_no_auth_or_origin_rewrite() {
        let config = manifest().caddy_config().unwrap();
        assert_eq!(
            config["admin"]["listen"],
            "unix//run/finite-hermes-caddy/admin.sock"
        );
        assert_eq!(config["admin"]["config"]["persist"], false);
        let server = &config["apps"]["http"]["servers"]["hosted_hermes"];
        let route = &server["routes"][0]["handle"][0]["routes"][0];
        assert_eq!(
            route["match"][0]["path"],
            json!(["/runtimes/runtime_1", "/runtimes/runtime_1/*"])
        );
        let handlers = &route["handle"][0]["routes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["handle"];
        assert_eq!(handlers[0]["strip_path_prefix"], "/runtimes/runtime_1");
        assert_eq!(
            handlers[1]["upstreams"],
            json!([{ "dial": "127.0.0.1:30000" }])
        );
        assert_eq!(
            handlers[1]["headers"]["request"]["set"],
            json!({ "X-Forwarded-Prefix": ["/runtimes/runtime_1"] })
        );
        assert!(server.get("logs").is_none());
        assert_eq!(
            config["logging"]["logs"]["default"]["encoder"]["fields"]["request"]["filter"],
            "delete"
        );
        assert_eq!(
            config["apps"]["pki"]["certificate_authorities"]["local"]["install_trust"],
            false
        );
    }

    #[test]
    fn core_and_edge_share_canonical_origin_parsing() {
        let mut value = manifest();
        value.public_origin = "https://AGENTS.EXAMPLE.COM:443/".into();
        let config = value.caddy_config().unwrap();
        assert_eq!(
            config["apps"]["http"]["servers"]["hosted_hermes"]["routes"][0]["match"][0]["host"],
            json!(["agents.example.com"])
        );
        value.public_origin = "https://[::1]/".into();
        value.listen = "[::1]:443".parse().unwrap();
        let config = value.caddy_config().unwrap();
        assert_eq!(
            config["apps"]["http"]["servers"]["hosted_hermes"]["routes"][0]["match"][0]["host"],
            json!(["::1"])
        );
    }

    #[test]
    fn projection_is_stable_and_has_no_extra_configuration_escape_hatch() {
        let mut value = manifest();
        value.routes.push(HostedHermesRoute {
            runtime_id: "runtime_2".into(),
            host_port: 30001,
        });
        let expected = value.caddy_config().unwrap();
        value.routes.reverse();
        assert_eq!(value.caddy_config().unwrap(), expected);
        let mut json = serde_json::to_value(&value).unwrap();
        json["routes"][0]["upstream"] = json!("http://another-runner:8080");
        assert!(serde_json::from_value::<HostedHermesRouteManifest>(json).is_err());
        value.routes.clear();
        assert!(value.caddy_config().is_ok());
    }
}
