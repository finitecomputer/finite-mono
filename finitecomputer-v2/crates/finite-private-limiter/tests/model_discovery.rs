use axum::{body::Body, http::Request};
use finite_private_limiter::{LimiterConfig, ModelCatalogConfig, NumesConfig, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;

fn config() -> LimiterConfig {
    // Neither service exists: discovering metadata must not contact either one.
    let mut config = LimiterConfig::new(
        "http://127.0.0.1:1/core".into(),
        "synthetic-core-secret".into(),
        "http://127.0.0.1:1/model".into(),
        "synthetic-model-secret".into(),
        "https://finite.example/dashboard".into(),
    );
    config.upstream_model = Some("glm-5-3-flash".into());
    config.model_aliases = vec!["glm-5-2".into(), "glm-5.3-flash".into()];
    config.numes = Some(NumesConfig {
        api_key: "synthetic-numes-source-key-000000000000".into(),
        control_api_key: "synthetic-finite-control-key-0000000000".into(),
        max_concurrent: 0,
        max_output_tokens: Some(4096),
        request_timeout: Duration::from_secs(120),
    });
    config.model_catalog = Some(serde_json::from_value(metadata()).unwrap());
    config
}

fn metadata() -> Value {
    json!({"name":"GLM-5.3 Flash", "context_window":393216,
        "tool_calling":false, "reasoning":true, "pricing":{}})
}

#[tokio::test]
async fn tinfoil_discovery_is_public_and_available_while_marketplace_is_paused() {
    let app = app(config()).unwrap();
    for credential in [None, Some("synthetic-numes-source-key-000000000000")] {
        let mut request = Request::get("/v1/models");
        if let Some(credential) = credential {
            request = request.header("authorization", format!("Bearer {credential}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert!(!response.headers().contains_key("x-tinfoil-usage-metrics"));
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let actual: Value = serde_json::from_slice(&bytes).unwrap();
        // Tinfoil catalogue fields plus OpenAI list identity; aliases must not
        // become separately advertised marketplace models. Unknown prices stay {}.
        assert_eq!(
            actual,
            json!({"object":"list", "data":[{
                "id":"glm-5-3-flash", "object":"model", "created":0, "owned_by":"finite",
                "name":"GLM-5.3 Flash", "type":"chat", "endpoints":["/v1/chat/completions"],
                "context_window":393216, "multimodal":false, "tool_calling":false,
                "reasoning":true, "supported_efforts":[], "pricing":{}
            }]})
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-"));
    }
}

#[tokio::test]
async fn only_explicit_prices_are_published_in_tinfoil_units() {
    let mut config = config();
    let pricing = &mut config.model_catalog.as_mut().unwrap().pricing;
    pricing.input = Some(0.25);
    pricing.output = Some(1.5);
    let response = app(config)
        .unwrap()
        .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let actual: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        actual["data"][0]["pricing"],
        json!({"inputTokenPricePer1M":0.25,"outputTokenPricePer1M":1.5})
    );
}

#[tokio::test]
async fn unconfigured_catalogue_advertises_no_guessed_model_capabilities() {
    let mut config = config();
    config.model_catalog = None;
    let response = app(config)
        .unwrap()
        .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"object":"list","data":[]})
    );
}

#[test]
fn invalid_or_ambiguous_catalogue_configuration_fails_at_startup() {
    for (field, value) in [
        ("name", json!(" ")),
        ("context_window", json!(0)),
        (
            "pricing",
            json!({"inputTokenPricePer1M":-1,"outputTokenPricePer1M":1}),
        ),
        ("pricing", json!({"inputTokenPricePer1M":1})),
    ] {
        let mut input = metadata();
        input[field] = value;
        let mut config = config();
        config.model_catalog = Some(serde_json::from_value(input).unwrap());
        assert!(app(config).is_err());
    }
    let mut config = config();
    let pricing = &mut config.model_catalog.as_mut().unwrap().pricing;
    pricing.input = Some(f64::NAN);
    pricing.output = Some(1.0);
    assert!(app(config).is_err());
    for invalid in [
        json!({"name":"test","context_window":393216}),
        json!({"name":"test","context_window":393216,"tool_calling":false,"reasoning":true,"pricing":{"input_token_price":1}}),
    ] {
        assert!(serde_json::from_value::<ModelCatalogConfig>(invalid).is_err());
    }
}
