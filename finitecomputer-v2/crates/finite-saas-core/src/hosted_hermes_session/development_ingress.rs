//! Local-harness transport for the native session exchange.
//!
//! Production Core reaches each configured hosted origin through public DNS and
//! public TLS roots. A local harness (devfinity) serves the same Runner-rendered
//! ingress on loopback with a temporary local CA instead. This configuration can
//! only narrow that transport: it trusts exactly the named CA (built-in roots are
//! disabled) and resolves origin hosts to a loopback address. Origins, routes,
//! account authorization and native authentication are unchanged.
use reqwest::{Certificate, Client, Url};
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentIngress {
    /// PEM root of the local ingress CA. Read per grant because the ingress
    /// may create it after Core starts.
    root_certificate_file: PathBuf,
    loopback_address: IpAddr,
}

impl DevelopmentIngress {
    /// Parse trusted deployment configuration; invalid values must stop startup.
    pub fn from_json(value: &str) -> Result<Self, &'static str> {
        let ingress: Self = serde_json::from_str(value).map_err(
            |_| "hosted Hermes development ingress must be {rootCertificateFile, loopbackAddress}",
        )?;
        if !ingress.loopback_address.is_loopback() {
            return Err("hosted Hermes development ingress must resolve to a loopback address");
        }
        if !ingress.root_certificate_file.is_absolute() {
            return Err("hosted Hermes development ingress CA path must be absolute");
        }
        Ok(ingress)
    }

    pub(super) fn client(&self, base_url: &str) -> Result<Client, &'static str> {
        let host = Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .ok_or("native location unavailable")?;
        let pem = std::fs::read(&self.root_certificate_file)
            .map_err(|_| "development ingress CA unavailable")?;
        let roots = Certificate::from_pem_bundle(&pem)
            .ok()
            .filter(|roots| !roots.is_empty())
            .ok_or("development ingress CA invalid")?;
        let mut builder = super::native_client_builder().tls_built_in_root_certs(false);
        for root in roots {
            builder = builder.add_root_certificate(root);
        }
        builder
            // The URL port is retained; only the address lookup is replaced.
            .resolve(&host, SocketAddr::new(self.loopback_address, 0))
            .build()
            .map_err(|_| "development ingress CA invalid")
    }
}
