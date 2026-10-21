//! Dedicated v3 lookup: a full page plus the canonical participating subset.
//! Legacy reconstruction cannot receive the full-page request type.
mod http;
mod validation;
use super::*;
pub(crate) use http::http_scoped_description_lookup;
pub(crate) use validation::validate_scoped_response;

pub(crate) const DESCRIPTIONS_VERSION_V3: &str = "finite-core-brain-descriptions-v3";
pub(crate) const OPERATOR_SOURCE: &str = "operatorScopedRecord";

#[derive(Debug, Clone)]
pub(crate) struct ScopedLookupRequest {
    pub(crate) brain_server: String,
    pub(crate) brain_id: String,
    pub(crate) requested_by_hex: String,
    pub(crate) keys: Vec<String>,
    pub(crate) participating_keys: Vec<String>,
}
impl ScopedLookupRequest {
    pub(crate) fn validate(&self) -> Result<(), CoreLookupFailure> {
        let page: BTreeSet<_> = self.keys.iter().collect();
        let subset: BTreeSet<_> = self.participating_keys.iter().collect();
        if self.keys.is_empty()
            || self.keys.len() > CORE_BATCH_KEYS
            || page.len() != self.keys.len()
            || subset.len() != self.participating_keys.len()
            || !subset.is_subset(&page)
            || !self.keys.iter().all(|key| canonical_hex(key))
            || !canonical_hex(&self.requested_by_hex)
            || self.brain_id.is_empty()
            || self.brain_id.len() > 256
        {
            return Err(CoreLookupFailure::Unavailable(
                "invalid local v3 page or subset".to_owned(),
            ));
        }
        Ok(())
    }
    pub(crate) fn legacy(&self) -> Option<CoreLookupRequest> {
        (!self.participating_keys.is_empty()).then(|| CoreLookupRequest {
            brain_server: self.brain_server.clone(),
            brain_id: self.brain_id.clone(),
            requested_by_hex: self.requested_by_hex.clone(),
            keys: self.participating_keys.clone(),
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OperatorAnnotation {
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) observed_at: String,
}
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OperatorRecord {
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) revision: String,
    pub(crate) observed_at: String,
}
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScopedCoreDescription {
    pub(crate) public_key_hex: String,
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) responsible_account: Option<CoreResponsibleAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<CoreSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) operator_annotation: Option<OperatorAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) operator_record: Option<OperatorRecord>,
}
impl ScopedCoreDescription {
    pub(crate) fn legacy(&self) -> CoreDescription {
        CoreDescription {
            public_key_hex: self.public_key_hex.clone(),
            state: self.state.clone(),
            kind: self.kind.clone(),
            display_name: self.display_name.clone(),
            account_email: self.account_email.clone(),
            lifecycle: self.lifecycle.clone(),
            responsible_account: self.responsible_account.clone(),
            source: self.source.clone(),
        }
    }
}
impl From<CoreDescription> for ScopedCoreDescription {
    fn from(row: CoreDescription) -> Self {
        Self {
            public_key_hex: row.public_key_hex,
            state: row.state,
            kind: row.kind,
            display_name: row.display_name,
            account_email: row.account_email,
            lifecycle: row.lifecycle,
            responsible_account: row.responsible_account,
            source: row.source,
            operator_annotation: None,
            operator_record: None,
        }
    }
}
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScopedCoreResponse {
    pub(crate) version: String,
    pub(crate) brain_id: String,
    pub(crate) checked_at: String,
    pub(crate) results: Vec<ScopedCoreDescription>,
}
impl From<CoreDescriptionsResponse> for ScopedCoreResponse {
    fn from(response: CoreDescriptionsResponse) -> Self {
        Self {
            version: response.version,
            brain_id: response.brain_id,
            checked_at: response.checked_at,
            results: response.results.into_iter().map(Into::into).collect(),
        }
    }
}

/// Transport context survives failure so non-participating keys are never
/// misrepresented as attempted by a reconstructed legacy lookup.
pub(crate) struct ScopedLookupReport {
    pub(crate) result: Result<Option<ScopedCoreResponse>, CoreLookupFailure>,
    pub(crate) legacy: bool,
    pub(crate) negotiation: &'static str,
}
pub(crate) type ScopedDescriptionLookup =
    Arc<dyn Fn(&ScopedLookupRequest) -> ScopedLookupReport + Send + Sync>;

#[cfg(test)]
pub(crate) fn legacy_fixture(lookup: CoreDescriptionLookup) -> ScopedDescriptionLookup {
    Arc::new(move |request| ScopedLookupReport {
        result: request
            .legacy()
            .map(|legacy| {
                lookup(&legacy)
                    .and_then(|response| validate_core_response(&legacy, response))
                    .map(Into::into)
            })
            .transpose(),
        legacy: true,
        negotiation: "afterVersionRejection",
    })
}

#[cfg(test)]
mod tests;
