//! Configured locations only: this module does not grant access or establish
//! native Hermes readiness. Core owns placement resolution; clients treat the
//! returned URL as opaque.
use crate::normalize_source_host_id;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
#[cfg(test)]
mod tests;

/// Shared Core/Runner validation of the single HTTPS origin used for hosted
/// Hermes. The normalized URL is safe to use as an exact Caddy host matcher.
pub fn parse_hosted_hermes_origin(value: &str) -> Result<reqwest::Url, &'static str> {
    let origin =
        reqwest::Url::parse(value).map_err(|_| "hosted Hermes locations must be HTTPS origins")?;
    if origin.scheme() != "https"
        || origin
            .host_str()
            .is_none_or(|host| host.contains(['*', '{', '}']))
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.port() == Some(0)
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(
            "hosted Hermes locations must be literal HTTPS origins without credentials, paths, queries, or fragments",
        );
    }
    Ok(origin)
}

#[derive(Clone, Debug, Default)]
pub struct HostedHermesOrigins(BTreeMap<String, reqwest::Url>);

/// Shared bound for Core's complete host projection and Runner's route renderer.
pub const MAX_HOSTED_HERMES_ROUTES: usize = 1024;

impl HostedHermesOrigins {
    /// Parse trusted deployment configuration. Invalid configuration must stop
    /// startup rather than silently routing a runtime to an unintended origin.
    pub fn from_json(value: &str) -> Result<Self, &'static str> {
        let UniqueOrigins(entries) = serde_json::from_str(value).map_err(
            |_| "hosted Hermes origins must be a JSON object of source hosts to HTTPS origins",
        )?;
        let mut origins = BTreeMap::new();
        for (host, value) in entries {
            if normalize_source_host_id(&host).ok().as_ref() != Some(&host) {
                return Err("hosted Hermes source hosts must use canonical source host IDs");
            }
            let origin = parse_hosted_hermes_origin(&value)?;
            origins.insert(host, origin);
        }
        Ok(Self(origins))
    }

    pub(crate) fn has_host(&self, source_host: &str) -> bool {
        self.0.contains_key(source_host)
    }

    pub(crate) fn location(&self, source_host: &str, runtime_id: &str) -> HostedHermesLocation {
        let base_url = self.0.get(source_host).map(|origin| {
            let mut url = origin.clone();
            // The stable Core runtime ID is one encoded path segment, never a
            // source machine name or a client-supplied URL.
            url.path_segments_mut()
                .expect("validated HTTPS origin")
                .clear()
                .push("runtimes")
                .push(runtime_id)
                .push("");
            url.to_string()
        });
        HostedHermesLocation {
            runtime_id: runtime_id.to_string(),
            availability: if base_url.is_some() {
                HostedHermesAvailability::Unqualified
            } else {
                HostedHermesAvailability::NotConfigured
            },
            base_url,
        }
    }
}

// Serde's ordinary map deserializer accepts duplicate keys with last-wins
// behavior. A deployment typo must not silently select another routing origin.
struct UniqueOrigins(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for UniqueOrigins {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueOrigins;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a source-host to HTTPS-origin map")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut origins = BTreeMap::new();
                while let Some((host, origin)) = map.next_entry::<String, String>()? {
                    if origins.insert(host, origin).is_some() {
                        return Err(serde::de::Error::custom("duplicate source host"));
                    }
                }
                Ok(UniqueOrigins(origins))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedHermesLocation {
    pub runtime_id: String,
    pub base_url: Option<String>,
    pub availability: HostedHermesAvailability,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedHermesAvailability {
    /// A deployment origin exists. No native authentication, service readiness,
    /// enablement, or connection grant has been established by this response.
    Unqualified,
    NotConfigured,
}
