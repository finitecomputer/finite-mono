use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{CliError, validate_http_url};

const MAX_EMBEDDING_BATCH: usize = 64;
const MAX_EMBEDDING_INPUTS: usize = 4096;
const MAX_EMBEDDING_DIMENSIONS: usize = 8192;

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
        let wire: WireEmbeddingProviderResponse = response.into_json().map_err(|_| {
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
