//! Gate configuration. The account application owns browser authentication.
use anyhow::{Context, Result, bail};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct GateConfig {
    pub listen: SocketAddr,
    pub public_url: String,
    pub signing_key: [u8; 32],
    pub account_url: String,
    pub account_token: String,
    pub site_base_domain: String,
}
impl GateConfig {
    pub fn public_key_hex(&self) -> Result<String> {
        finite_authn::vouch::gate_pubkey_for_secret(&self.signing_key)
            .context("derive gate public key")
    }
    pub fn validate(&self) -> Result<()> {
        self.public_key_hex()?;
        if !finite_authn::hex::is_hex32(&self.account_token) {
            bail!("FINITE_GATE_ACCOUNT_TOKEN must be 64 lowercase hex characters");
        }
        let public = secure_url(&self.public_url)?;
        if public.path() != "/" {
            bail!("FINITE_GATE_PUBLIC_URL must be an origin");
        }
        secure_url(&self.account_url)?;
        let domain = &self.site_base_domain;
        if domain.len() > 253 || !domain.split('.').all(valid_label) {
            bail!("FINITE_GATE_SITE_BASE_DOMAIN must be a DNS name");
        }
        Ok(())
    }
    pub fn from_env() -> Result<Self> {
        let required =
            |name: &str| std::env::var(name).with_context(|| format!("{name} is required"));
        let config = Self {
            listen: std::env::var("FINITE_GATE_LISTEN")
                .unwrap_or_else(|_| "127.0.0.1:8792".into())
                .parse()?,
            public_url: required("FINITE_GATE_PUBLIC_URL")?,
            signing_key: finite_authn::hex::decode32(&required("FINITE_GATE_SIGNING_KEY")?)
                .context("FINITE_GATE_SIGNING_KEY must be 64 lowercase hex characters")?,
            account_url: required("FINITE_GATE_ACCOUNT_URL")?,
            account_token: required("FINITE_GATE_ACCOUNT_TOKEN")?,
            site_base_domain: std::env::var("FINITE_GATE_SITE_BASE_DOMAIN")
                .unwrap_or_else(|_| "finite.site".into()),
        };
        config.validate()?;
        Ok(config)
    }
}
pub(crate) fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub(crate) fn secure_url(raw: &str) -> Result<url::Url> {
    let url = url::Url::parse(raw)?;
    let host = url.host_str().context("URL needs a host")?;
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!(
            "URL must use HTTPS (HTTP only on localhost or loopback), without credentials, query or fragment"
        );
    }
    Ok(url)
}
