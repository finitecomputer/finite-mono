//! Tinfoil-compatible public discovery. Metadata is deployment configuration,
//! not an upstream inference request or a subscription/marketplace admission.
use super::{AppState, LimiterConfigError};
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalogConfig {
    pub name: String,
    pub context_window: u64,
    pub tool_calling: bool,
    pub reasoning: bool,
    /// Unix creation timestamp if known; zero means unspecified.
    #[serde(default)]
    pub created: u64,
    /// Absent rates stay absent: unspecified pricing must not advertise free use.
    #[serde(default)]
    pub pricing: ModelPricing,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPricing {
    #[serde(
        rename = "inputTokenPricePer1M",
        skip_serializing_if = "Option::is_none"
    )]
    pub input: Option<f64>,
    #[serde(
        rename = "outputTokenPricePer1M",
        skip_serializing_if = "Option::is_none"
    )]
    pub output: Option<f64>,
}

impl ModelCatalogConfig {
    pub(super) fn validate(&self) -> Result<(), LimiterConfigError> {
        if self.name.trim().is_empty() || self.context_window == 0 {
            return Err(LimiterConfigError::InvalidModelCatalog(
                "name and positive context_window are required".into(),
            ));
        }
        if self.pricing.input.is_some() != self.pricing.output.is_some()
            || [self.pricing.input, self.pricing.output]
                .into_iter()
                .flatten()
                .any(|price| !price.is_finite() || price < 0.0)
        {
            return Err(LimiterConfigError::InvalidModelCatalog(
                "prompt and completion prices must both be specified or both omitted; rates must be finite and nonnegative".into(),
            ));
        }
        Ok(())
    }
}

pub(super) async fn list(State(state): State<AppState>) -> Json<Value> {
    let data: Vec<_> = state
        .config
        .model_catalog
        .iter()
        .map(|catalog| {
            json!({
                "id": state.config.default_model,
                "object": "model",
                "created": catalog.created,
                "owned_by": "finite",
                "name": catalog.name,
                "type": "chat",
                "endpoints": ["/v1/chat/completions"],
                "context_window": catalog.context_window,
                "multimodal": false,
                "tool_calling": catalog.tool_calling,
                "reasoning": catalog.reasoning,
                "supported_efforts": [],
                "pricing": catalog.pricing,
            })
        })
        .collect();
    Json(json!({"object": "list", "data": data}))
}
