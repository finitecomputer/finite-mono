use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{CliError, validate_http_url};

const MAX_EMBEDDING_BATCH: usize = 64;
const MAX_EMBEDDING_INPUTS: usize = 4096;
const MAX_EMBEDDING_DIMENSIONS: usize = 8192;
const MAX_EMBEDDING_TEXT_CHARS: usize = 32 * 1024;
const MAX_EMBEDDING_PAGE_TITLE_CHARS: usize = 512;
const MAX_EMBEDDING_HEADINGS: usize = 12;
const MAX_EMBEDDING_HEADING_CHARS: usize = 512;
const MAX_EMBEDDING_TOTAL_CHARS: usize = 8 * 1024 * 1024;
const MAX_EMBEDDING_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Runtime-injected connection settings for one replaceable Embedding Provider.
#[derive(Clone)]
pub struct EmbeddingProviderConfig {
    pub endpoint: String,
    pub bearer_token: String,
    pub timeout: Duration,
}

/// Provider-specific transport hidden behind the Hybrid Wiki Search adapter.
#[derive(Clone)]
pub struct EmbeddingProviderAdapter {
    endpoint: String,
    bearer_token: String,
    agent: ureq::Agent,
}

/// Minimum plaintext input accepted by the provider adapter.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProviderInput {
    id: String,
    kind: EmbeddingProviderInputKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_title: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    heading_ancestry: Vec<String>,
    text: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum EmbeddingProviderInputKind {
    Section,
    Query,
}

/// One validated provider generation returned to indexing/search code.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingProviderResponse {
    pub model: String,
    pub model_version: String,
    pub dimensions: usize,
    pub vectors: Vec<EmbeddingProviderVector>,
}

/// One request-correlated vector returned by the provider.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingProviderVector {
    pub id: String,
    pub embedding: Vec<f32>,
}

impl fmt::Debug for EmbeddingProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingProviderConfig")
            .field("endpoint", &self.endpoint)
            .field("bearer_token", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl fmt::Debug for EmbeddingProviderAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingProviderAdapter")
            .field("endpoint", &self.endpoint)
            .field("bearer_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEmbeddingProviderResponse {
    model: String,
    model_version: String,
    dimensions: usize,
    vectors: Vec<WireEmbeddingProviderVector>,
}

#[derive(Debug, Deserialize)]
struct WireEmbeddingProviderVector {
    id: String,
    embedding: Vec<f32>,
}

impl EmbeddingProviderInput {
    pub fn section(
        id: impl Into<String>,
        page_title: impl Into<String>,
        heading_ancestry: Vec<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: EmbeddingProviderInputKind::Section,
            page_title: Some(page_title.into()),
            heading_ancestry,
            text: text.into(),
        }
    }

    pub fn query(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: EmbeddingProviderInputKind::Query,
            page_title: None,
            heading_ancestry: Vec::new(),
            text: text.into(),
        }
    }
}

impl EmbeddingProviderAdapter {
    pub fn new(config: EmbeddingProviderConfig) -> Result<Self, CliError> {
        let endpoint = config.endpoint.trim().trim_end_matches('/').to_owned();
        validate_http_url(&endpoint)?;
        if config.bearer_token.trim().is_empty() {
            return Err(CliError::InvalidInput(
                "Embedding Provider bearer token cannot be empty".to_owned(),
            ));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(config.timeout)
            .redirects(0)
            .build();
        Ok(Self {
            endpoint,
            bearer_token: config.bearer_token,
            agent,
        })
    }

    pub fn embed(
        &self,
        inputs: &[EmbeddingProviderInput],
    ) -> Result<EmbeddingProviderResponse, CliError> {
        validate_input_identifiers(inputs)?;
        let mut generation = None::<(String, String, usize)>;
        let mut vectors = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(MAX_EMBEDDING_BATCH) {
            let response = self.embed_batch(batch)?;
            let next_generation = (
                response.model.clone(),
                response.model_version.clone(),
                response.dimensions,
            );
            if generation
                .as_ref()
                .is_some_and(|current| current != &next_generation)
            {
                return Err(CliError::EmbeddingProvider(
                    "provider changed model contract between batches".to_owned(),
                ));
            }
            generation.get_or_insert(next_generation);
            vectors.extend(response.vectors);
        }
        let (model, model_version, dimensions) = generation.expect("non-empty input was validated");
        Ok(EmbeddingProviderResponse {
            model,
            model_version,
            dimensions,
            vectors,
        })
    }

    fn embed_batch(
        &self,
        inputs: &[EmbeddingProviderInput],
    ) -> Result<EmbeddingProviderResponse, CliError> {
        let url = format!("{}/v1/embeddings", self.endpoint);
        let request = self
            .agent
            .post(&url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .set("Authorization", &format!("Bearer {}", self.bearer_token));
        let response = request.send_json(serde_json::json!({ "inputs": inputs }));
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) => {
                return Err(CliError::EmbeddingProvider(format!(
                    "provider returned status {status}"
                )));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(CliError::EmbeddingProvider(format!(
                    "provider request failed: {}",
                    safe_transport_error(&error.to_string())
                )));
            }
        };
        let mut body = Vec::new();
        response
            .into_reader()
            .take((MAX_EMBEDDING_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| {
                CliError::EmbeddingProvider("provider response was not readable".to_owned())
            })?;
        if body.len() > MAX_EMBEDDING_RESPONSE_BYTES {
            return Err(CliError::EmbeddingProvider(
                "provider response exceeded its byte limit".to_owned(),
            ));
        }
        let wire: WireEmbeddingProviderResponse = serde_json::from_slice(&body).map_err(|_| {
            CliError::EmbeddingProvider("provider returned invalid JSON".to_owned())
        })?;
        validate_provider_response(inputs, wire)
    }
}

fn validate_input_identifiers(inputs: &[EmbeddingProviderInput]) -> Result<(), CliError> {
    if inputs.is_empty() || inputs.len() > MAX_EMBEDDING_INPUTS {
        return Err(CliError::InvalidInput(format!(
            "Embedding Provider request must contain 1 to {MAX_EMBEDDING_INPUTS} inputs"
        )));
    }
    let mut identifiers = BTreeSet::new();
    let mut total_chars = 0_usize;
    for input in inputs {
        if input.id.is_empty()
            || input.id.len() > 128
            || !input
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            || !identifiers.insert(input.id.as_str())
        {
            return Err(CliError::InvalidInput(
                "Embedding Provider input IDs must be unique safe opaque identifiers".to_owned(),
            ));
        }
        let text_chars = input.text.chars().count();
        if input.text.trim().is_empty() || text_chars > MAX_EMBEDDING_TEXT_CHARS {
            return Err(CliError::InvalidInput(format!(
                "Embedding Provider text must contain 1 to {MAX_EMBEDDING_TEXT_CHARS} characters"
            )));
        }
        match input.kind {
            EmbeddingProviderInputKind::Section => {
                if !input.page_title.as_deref().is_some_and(|title| {
                    !title.trim().is_empty()
                        && title.chars().count() <= MAX_EMBEDDING_PAGE_TITLE_CHARS
                }) {
                    return Err(CliError::InvalidInput(
                        "Section embeddings require a bounded Page title".to_owned(),
                    ));
                }
                if input.heading_ancestry.len() > MAX_EMBEDDING_HEADINGS
                    || input.heading_ancestry.iter().any(|heading| {
                        heading.trim().is_empty()
                            || heading.chars().count() > MAX_EMBEDDING_HEADING_CHARS
                    })
                {
                    return Err(CliError::InvalidInput(
                        "Section embedding heading ancestry exceeds its bounds".to_owned(),
                    ));
                }
            }
            EmbeddingProviderInputKind::Query => {
                if input.page_title.is_some() || !input.heading_ancestry.is_empty() {
                    return Err(CliError::InvalidInput(
                        "Query embeddings may contain only query text".to_owned(),
                    ));
                }
            }
        }
        total_chars = total_chars
            .saturating_add(text_chars)
            .saturating_add(
                input
                    .page_title
                    .as_deref()
                    .map(str::chars)
                    .map(Iterator::count)
                    .unwrap_or_default(),
            )
            .saturating_add(
                input
                    .heading_ancestry
                    .iter()
                    .map(|heading| heading.chars().count())
                    .sum::<usize>(),
            );
        if total_chars > MAX_EMBEDDING_TOTAL_CHARS {
            return Err(CliError::InvalidInput(format!(
                "Embedding Provider request exceeds {MAX_EMBEDDING_TOTAL_CHARS} total characters"
            )));
        }
    }
    Ok(())
}

fn validate_provider_response(
    inputs: &[EmbeddingProviderInput],
    wire: WireEmbeddingProviderResponse,
) -> Result<EmbeddingProviderResponse, CliError> {
    if wire.model.trim().is_empty() || wire.model_version.trim().is_empty() {
        return Err(CliError::EmbeddingProvider(
            "provider omitted model identity or version".to_owned(),
        ));
    }
    if wire.dimensions == 0 || wire.dimensions > MAX_EMBEDDING_DIMENSIONS {
        return Err(CliError::EmbeddingProvider(
            "provider returned an invalid vector dimension".to_owned(),
        ));
    }
    let mut vectors = BTreeMap::new();
    for vector in wire.vectors {
        if vector.embedding.len() != wire.dimensions
            || vector.embedding.iter().any(|value| !value.is_finite())
            || vectors
                .insert(vector.id.clone(), vector.embedding)
                .is_some()
        {
            return Err(CliError::EmbeddingProvider(
                "provider returned duplicate, non-finite, or wrong-dimension vectors".to_owned(),
            ));
        }
    }
    if vectors.len() != inputs.len() {
        return Err(CliError::EmbeddingProvider(
            "provider vector count did not match the request".to_owned(),
        ));
    }
    let vectors = inputs
        .iter()
        .map(|input| {
            vectors
                .remove(&input.id)
                .map(|embedding| EmbeddingProviderVector {
                    id: input.id.clone(),
                    embedding,
                })
                .ok_or_else(|| {
                    CliError::EmbeddingProvider(format!(
                        "provider omitted vector for input {}",
                        input.id
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !vectors.is_empty()
        && !vectors
            .iter()
            .all(|vector| vector.embedding.len() == wire.dimensions)
    {
        return Err(CliError::EmbeddingProvider(
            "provider returned inconsistent vector dimensions".to_owned(),
        ));
    }
    Ok(EmbeddingProviderResponse {
        model: wire.model,
        model_version: wire.model_version,
        dimensions: wire.dimensions,
        vectors,
    })
}

fn safe_transport_error(error: &str) -> String {
    let error = error.replace(['\r', '\n'], " ");
    error.chars().take(256).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_rejects_oversized_plaintext_before_transport() {
        let input =
            EmbeddingProviderInput::query("query-1", "x".repeat(MAX_EMBEDDING_TEXT_CHARS + 1));

        let error = validate_input_identifiers(&[input]).unwrap_err();

        assert!(error.to_string().contains("text must contain"));
    }

    #[test]
    fn provider_response_rejects_unknown_duplicate_and_non_finite_vectors() {
        let inputs = [EmbeddingProviderInput::query("query-1", "find this")];
        let invalid = [
            WireEmbeddingProviderResponse {
                model: "model".to_owned(),
                model_version: "v1".to_owned(),
                dimensions: 1,
                vectors: vec![WireEmbeddingProviderVector {
                    id: "unknown".to_owned(),
                    embedding: vec![1.0],
                }],
            },
            WireEmbeddingProviderResponse {
                model: "model".to_owned(),
                model_version: "v1".to_owned(),
                dimensions: 1,
                vectors: vec![
                    WireEmbeddingProviderVector {
                        id: "query-1".to_owned(),
                        embedding: vec![1.0],
                    },
                    WireEmbeddingProviderVector {
                        id: "query-1".to_owned(),
                        embedding: vec![1.0],
                    },
                ],
            },
            WireEmbeddingProviderResponse {
                model: "model".to_owned(),
                model_version: "v1".to_owned(),
                dimensions: 1,
                vectors: vec![WireEmbeddingProviderVector {
                    id: "query-1".to_owned(),
                    embedding: vec![f32::NAN],
                }],
            },
        ];

        for response in invalid {
            assert!(validate_provider_response(&inputs, response).is_err());
        }
    }
}
