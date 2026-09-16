//! Configured locations only: this module does not grant access or establish
//! native Hermes readiness. Core owns placement resolution; clients treat the
//! returned URL as opaque.
use crate::normalize_source_host_id;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_hermes_origins_reject_invalid_configuration() {
        for value in [
            "[]",
            r#"{"host":"https://a.example.test","host":"https://b.example.test"}"#,
            r#"{"host":null}"#,
            r#"{" Host ":"https://agents.example.test"}"#,
            r#"{"bad/host":"https://agents.example.test"}"#,
            r#"{"host":"http://agents.example.test"}"#,
            r#"{"host":"https://agents.example.test:0"}"#,
            r#"{"host":"https://agents.example.test:65536"}"#,
            r#"{"host":"https://*.example.test"}"#,
            r#"{"host":"https://{env.DOMAIN}"}"#,
            r#"{"host":"https://user:password@agents.example.test"}"#,
            r#"{"host":"https://agents.example.test/path"}"#,
            r#"{"host":"https://agents.example.test/?target=other"}"#,
            r#"{"host":"https://agents.example.test/#fragment"}"#,
        ] {
            assert!(HostedHermesOrigins::from_json(value).is_err());
        }
    }

    #[test]
    fn hosted_hermes_locations_are_opaque_and_never_claim_readiness() {
        let origins =
            HostedHermesOrigins::from_json(r#"{"host":"https://agents.example.test:8443/"}"#)
                .unwrap();
        let location = serde_json::to_value(origins.location("host", "agent/runtime?x=1")).unwrap();
        assert_eq!(
            location["baseUrl"],
            "https://agents.example.test:8443/runtimes/agent%2Fruntime%3Fx=1/"
        );
        assert_eq!(location["availability"], "unqualified");
        let missing =
            serde_json::to_value(origins.location("other-host", "agent-runtime")).unwrap();
        assert!(missing["baseUrl"].is_null());
        assert_eq!(missing["availability"], "not_configured");
    }
}
