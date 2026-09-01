//! Environment-only configuration. Secret NAMES live here; VALUES live in
//! the service environment files under `/etc` (see `infra/`), never in git.

use std::net::SocketAddr;

use anyhow::{Context, Result, bail};

pub const LISTEN_ENV: &str = "FINITE_GATE_LISTEN";
pub const PUBLIC_URL_ENV: &str = "FINITE_GATE_PUBLIC_URL";
pub const SIGNING_KEY_ENV: &str = "FINITE_GATE_SIGNING_KEY";
pub const WORKOS_CLIENT_ID_ENV: &str = "FINITE_GATE_WORKOS_CLIENT_ID";
pub const WORKOS_API_KEY_ENV: &str = "FINITE_GATE_WORKOS_API_KEY";
pub const DEV_EMAIL_ENV: &str = "FINITE_GATE_DEV_EMAIL";

pub const DEFAULT_LISTEN: &str = "127.0.0.1:8792";
pub const DEFAULT_DEV_EMAIL: &str = "dev@finite.computer";

#[derive(Debug, Clone)]
pub struct GateConfig {
    pub listen: SocketAddr,
    /// The gate's own canonical origin (no trailing slash), e.g.
    /// `https://auth.finite.computer`.
    pub public_url: String,
    /// Vouch signing secret. Its x-only public key is what finitesitesd
    /// pins as `FINITE_SITES_AUTH_GATE_PUBKEY`.
    pub signing_key: [u8; 32],
    /// Present ⇒ production WorkOS AuthKit mode; absent ⇒ dev mode.
    pub workos_client_id: Option<String>,
    pub workos_api_key: Option<String>,
    pub dev_email: String,
}

impl GateConfig {
    pub fn dev_mode(&self) -> bool {
        self.workos_client_id.is_none()
    }

    /// The x-only public key hex for the configured signing key, so an
    /// operator can pin it on finitesitesd (`finite-gated print-pubkey`).
    pub fn public_key_hex(&self) -> Result<String> {
        finite_authn::vouch::gate_pubkey_for_secret(&self.signing_key)
            .context("derive gate public key")
    }

    pub fn from_env() -> Result<GateConfig> {
        let listen = match std::env::var(LISTEN_ENV) {
            Ok(raw) if !raw.trim().is_empty() => raw
                .trim()
                .parse()
                .with_context(|| format!("{LISTEN_ENV} must be an ip:port"))?,
            _ => DEFAULT_LISTEN
                .parse()
                .expect("default listen address parses"),
        };
        let signing_key = parse_signing_key(&std::env::var(SIGNING_KEY_ENV).unwrap_or_default())?;
        let workos_client_id = non_empty_env(WORKOS_CLIENT_ID_ENV);
        let workos_api_key = non_empty_env(WORKOS_API_KEY_ENV);
        if workos_client_id.is_some() != workos_api_key.is_some() {
            bail!(
                "{WORKOS_CLIENT_ID_ENV} and {WORKOS_API_KEY_ENV} must be set together; omit both for dev mode"
            );
        }
        let public_url = match non_empty_env(PUBLIC_URL_ENV) {
            Some(url) => {
                let parsed = url::Url::parse(&url)
                    .with_context(|| format!("{PUBLIC_URL_ENV} must be an absolute URL"))?;
                if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
                    bail!("{PUBLIC_URL_ENV} must be an origin, not a URL with a path");
                }
                parsed.origin().ascii_serialization()
            }
            None => {
                if workos_client_id.is_some() {
                    bail!("{PUBLIC_URL_ENV} is required when WorkOS is configured");
                }
                format!("http://{listen}")
            }
        };
        let dev_email =
            non_empty_env(DEV_EMAIL_ENV).unwrap_or_else(|| DEFAULT_DEV_EMAIL.to_string());
        if dev_email.len() > 254 || !dev_email.contains('@') {
            bail!("{DEV_EMAIL_ENV} must be an email address");
        }
        Ok(GateConfig {
            listen,
            public_url,
            signing_key,
            workos_client_id,
            workos_api_key,
            dev_email,
        })
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_signing_key(raw: &str) -> Result<[u8; 32]> {
    let raw = raw.trim();
    if !finite_authn::hex::is_hex32(raw) {
        bail!(
            "{SIGNING_KEY_ENV} must be exactly 64 lowercase hex characters (openssl rand -hex 32)"
        );
    }
    finite_authn::hex::decode32(raw).context("decode signing key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_key_must_be_64_lowercase_hex() {
        assert!(
            parse_signing_key("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .is_ok()
        );
        assert!(parse_signing_key("").is_err());
        assert!(parse_signing_key("zz").is_err());
        assert!(
            parse_signing_key("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF")
                .is_err()
        );
    }
}
