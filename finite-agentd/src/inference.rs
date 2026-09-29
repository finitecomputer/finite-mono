//! The shared inference contract: saved-route classification (§3.3), the
//! additive status fields derived from stored facts (§3.4), the `model` blocks
//! agentd writes (§3.6), and the Finite Private settings agentd reads from its
//! environment.

use serde::Serialize;
use serde_json::{Value, json};

use crate::AgentdError;
use crate::codex::CodexRouteStatus;
use crate::facts::{InferenceFacts, PoolEntries, ProviderEntryFact, Tri, YesNo};
use crate::intent::{IntentKind, IntentPhase, IntentRecord, IntentRoute, IntentState};
use crate::ledger::hex_digest;

pub(crate) const FINITE_PRIVATE_PRODUCT_BASE_URL: &str =
    "https://finite-private.finite.containers.tinfoil.dev/v1";
pub(crate) const HISTORICAL_FINITE_PRIVATE_BASE_URL: &str =
    "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
const CANONICAL_FINITE_PRIVATE_MODEL: &str = "glm-5-3-flash";
const LEGACY_FINITE_PRIVATE_MODELS: &[&str] =
    &["glm-5-2", "deepseek-v4-flash-0731", "glm-5.3-flash"];
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Capabilities this agentd always advertises; the PR2 and PR3 modules add theirs.
const BASE_CAPABILITIES: &[&str] = &[
    "inference.status.v2",
    "inference.select.v1",
    "inference.disconnect.v1",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SavedRoute {
    FinitePrivate,
    Openrouter,
    OpenaiCodex,
    Other,
}

/// Classifies the raw `config.yaml` `model` value (§3.3). `fp_base_url` is the
/// configured Finite Private URL; the product and retired URLs are always
/// recognized.
pub(crate) fn classify_saved_route(model: &Value, fp_base_url: Option<&str>) -> SavedRoute {
    match model_provider(model).as_deref() {
        Some("openrouter") => SavedRoute::Openrouter,
        Some("openai-codex" | "codex" | "openai_codex") => SavedRoute::OpenaiCodex,
        Some("finite-private" | "custom:finite-private") => SavedRoute::FinitePrivate,
        Some("custom") => {
            let identity = model
                .get("base_url")
                .and_then(Value::as_str)
                .and_then(endpoint_identity);
            let is_finite_private = identity.is_some_and(|identity| {
                fp_base_url
                    .into_iter()
                    .chain([
                        FINITE_PRIVATE_PRODUCT_BASE_URL,
                        HISTORICAL_FINITE_PRIVATE_BASE_URL,
                    ])
                    .any(|url| endpoint_identity(url).as_ref() == Some(&identity))
            });
            if is_finite_private {
                SavedRoute::FinitePrivate
            } else {
                SavedRoute::Other
            }
        }
        _ => SavedRoute::Other,
    }
}

/// Hermes resolves provider names after trimming whitespace and lowercasing.
/// Keep classification and legacy credential lookup consistent with that route.
pub(crate) fn model_provider(model: &Value) -> Option<String> {
    model
        .get("provider")
        .and_then(Value::as_str)
        .map(|provider| provider.trim().to_lowercase())
}

/// Endpoint identity (§5.5): scheme and host case-insensitive, port, userinfo,
/// path, query, and fragment exact, and only trailing `/` on the path removed.
/// `None` for an empty URL or one that can't be bound to an endpoint; neither
/// matches anything. Mirrors Python's `urllib.parse.urlsplit`.
#[derive(Debug, PartialEq, Eq)]
struct EndpointIdentity {
    scheme: String,
    host: String,
    port: Option<u16>,
    userinfo: Option<String>,
    path: String,
    query: String,
    fragment: String,
}

fn endpoint_identity(url: &str) -> Option<EndpointIdentity> {
    let url = url.trim();
    let (scheme, rest) = url.split_once(':')?;
    let scheme_valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_valid {
        return None;
    }
    let rest = rest.strip_prefix("//")?;
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = rest
        .find('/')
        .map_or((rest, ""), |index| rest.split_at(index));
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo.to_owned()), host_port),
        None => (None, authority),
    };
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']')?;
        (host, after.strip_prefix(':'))
    } else {
        match host_port.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    let port = match port {
        None | Some("") => None,
        Some(port) => Some(port.parse::<u16>().ok()?),
    };
    if host.is_empty() {
        return None;
    }
    Some(EndpointIdentity {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
        userinfo,
        path: path.trim_end_matches('/').to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    })
}

/// Finite Private settings from agentd's environment, with the launcher's
/// retired-URL and legacy-model rewrites (`run_hermes_gateway.sh`), so status,
/// the helper, and the reconciler agree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FinitePrivateEnv {
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub context_length: Option<u64>,
}

impl FinitePrivateEnv {
    /// `(model, base_url)` when both are configured.
    pub(crate) fn settings(&self) -> Option<(&str, &str)> {
        Some((self.model.as_deref()?, self.base_url.as_deref()?))
    }

    /// `FINITE_CONFIG_FP_*` for the helper; an unset value is an empty string (§8.2).
    pub(crate) fn helper_env(&self) -> [(&'static str, String); 3] {
        [
            (
                "FINITE_CONFIG_FP_MODEL",
                self.model.clone().unwrap_or_default(),
            ),
            (
                "FINITE_CONFIG_FP_BASE_URL",
                self.base_url.clone().unwrap_or_default(),
            ),
            (
                "FINITE_CONFIG_FP_CONTEXT_LENGTH",
                self.context_length
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
        ]
    }
}

pub(crate) fn finite_private_env() -> FinitePrivateEnv {
    finite_private_env_from(|name| std::env::var(name).ok())
}

fn finite_private_env_from(lookup: impl Fn(&str) -> Option<String>) -> FinitePrivateEnv {
    let read = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
    let base_url = read("FINITE_PRIVATE_BASE_URL").map(|url| {
        if url == HISTORICAL_FINITE_PRIVATE_BASE_URL {
            FINITE_PRIVATE_PRODUCT_BASE_URL.to_owned()
        } else {
            url
        }
    });
    let model = read("FINITE_PRIVATE_MODEL").map(|model| {
        if LEGACY_FINITE_PRIVATE_MODELS.contains(&model.as_str())
            && base_url.as_deref() == Some(FINITE_PRIVATE_PRODUCT_BASE_URL)
        {
            CANONICAL_FINITE_PRIVATE_MODEL.to_owned()
        } else {
            model
        }
    });
    let context_length = read("FINITE_PRIVATE_CONTEXT_LENGTH")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    FinitePrivateEnv {
        model,
        base_url,
        context_length,
    }
}

/// The whole `model` value agentd writes for a route (§3.6). `model` is
/// ignored for Finite Private, whose model comes from agentd's environment.
/// Callers validate model names.
pub(crate) fn plan_model_block(
    route: IntentRoute,
    model: Option<&str>,
    fp: &FinitePrivateEnv,
) -> Result<Value, AgentdError> {
    match route {
        IntentRoute::FinitePrivate => {
            let (model, base_url) = fp.settings().ok_or_else(|| {
                AgentdError::Config("Finite Private isn't available on this agent.".to_owned())
            })?;
            let mut value = json!({
                "default": model,
                "provider": "custom",
                "base_url": base_url,
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "api_mode": "chat_completions",
            });
            if let Some(context_length) = fp.context_length {
                value["context_length"] = json!(context_length);
            }
            if model == CANONICAL_FINITE_PRIVATE_MODEL
                && matches!(
                    base_url,
                    FINITE_PRIVATE_PRODUCT_BASE_URL | HISTORICAL_FINITE_PRIVATE_BASE_URL
                )
            {
                value["supports_vision"] = json!(true);
            }
            Ok(value)
        }
        IntentRoute::Openrouter => Ok(json!({
            "default": required_model(model)?,
            "provider": "openrouter",
            "base_url": OPENROUTER_BASE_URL,
            "api_mode": "chat_completions",
        })),
        IntentRoute::OpenaiCodex => Ok(json!({
            "default": required_model(model)?,
            "provider": "openai-codex",
        })),
    }
}

fn required_model(model: Option<&str>) -> Result<&str, AgentdError> {
    model
        .filter(|model| !model.is_empty())
        .ok_or_else(|| AgentdError::InvalidPayload("A model is required".to_owned()))
}

/// The capabilities status advertises (§4.2).
pub(crate) fn capabilities() -> Vec<&'static str> {
    BASE_CAPABILITIES
        .iter()
        .chain(crate::openrouter::CAPABILITIES)
        .chain(crate::codex::CAPABILITIES)
        .copied()
        .collect()
}

/// The additive fields of `inference` in `agent.connections.status` (§3.4).
/// The legacy `profile`/`provider`/`model` fields sit beside them unchanged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct InferenceStatusV2 {
    pub saved: SavedStatus,
    pub routes: RoutesStatus,
    pub fallback: FallbackStatus,
    pub operation: Option<OperationStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SavedStatus {
    pub route: SavedRoute,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RoutesStatus {
    pub finite_private: FinitePrivateRouteStatus,
    pub openrouter: OpenRouterRouteStatus,
    /// Present only when `codex.login.v1` is advertised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_codex: Option<CodexRouteStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FinitePrivateRouteStatus {
    pub state: FinitePrivateState,
    pub reason: Option<FinitePrivateReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinitePrivateState {
    Configured,
    NotConfigured,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinitePrivateReason {
    SettingsMissing,
    CredentialMissing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OpenRouterRouteStatus {
    pub state: OpenRouterKeyState,
    pub key_source: Option<KeySource>,
    /// SHA-256 of the Connections-managed key; `None` for any other source.
    pub key_hash: Option<String>,
    pub hermes_key: HermesKey,
    pub other_pool_keys: PoolEntries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OpenRouterKeyState {
    KeySaved,
    NoKey,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeySource {
    Agent,
    LegacyConfig,
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HermesKey {
    SavedKey,
    OtherKey,
    None,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FallbackStatus {
    pub state: FallbackState,
    pub reason: Option<FallbackReason>,
    pub model: Option<String>,
    pub extra_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FallbackState {
    Configured,
    Unavailable,
    NotConfigured,
    Off,
    Custom,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FallbackReason {
    SettingsMissing,
    CredentialMissing,
    StaleConfig,
}

/// `inference.operation`: the intent record, or the in-memory last result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OperationStatus {
    pub id: String,
    pub kind: IntentKind,
    pub route: IntentRoute,
    pub model: Option<String>,
    pub state: OperationState,
    pub phase: IntentPhase,
    pub error_code: Option<String>,
    pub attempts: u32,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Running,
    Succeeded,
    Failed,
}

impl OperationStatus {
    pub(crate) fn from_record(record: &IntentRecord) -> Self {
        Self {
            id: record.id.clone(),
            kind: record.kind,
            route: record.route,
            model: record.model.clone(),
            state: match record.state {
                IntentState::Running => OperationState::Running,
                IntentState::Failed => OperationState::Failed,
            },
            phase: record.phase,
            error_code: record.error_code.clone(),
            attempts: record.attempts,
            updated_at_ms: record.updated_at_ms,
        }
    }
}

/// Derives the §3.4 fields from agentd's own reads and the helper facts.
/// `model` is the raw `config.yaml` `model` value, and `dotenv_key` the value
/// of the last `OPENROUTER_API_KEY` line in `.env`. `routes.openai_codex` and
/// `operation` are left for the caller to fill.
pub(crate) fn derive_status(
    model: &Value,
    dotenv_key: Option<&str>,
    fp: &FinitePrivateEnv,
    facts: &InferenceFacts,
) -> InferenceStatusV2 {
    let finite_private = finite_private_route(fp, facts);
    InferenceStatusV2 {
        saved: SavedStatus {
            route: classify_saved_route(model, fp.base_url.as_deref()),
            provider: model_string(model, "provider", 128),
            model: model_string(model, "default", 256),
        },
        fallback: fallback_status(&finite_private, facts),
        routes: RoutesStatus {
            openrouter: openrouter_route(model, dotenv_key, facts),
            finite_private,
            openai_codex: None,
        },
        operation: None,
    }
}

fn model_string(model: &Value, field: &str, max_chars: usize) -> Option<String> {
    model
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| value.chars().count() <= max_chars)
        .map(str::to_owned)
}

fn finite_private_route(fp: &FinitePrivateEnv, facts: &InferenceFacts) -> FinitePrivateRouteStatus {
    let (state, reason) = if fp.settings().is_none() {
        (
            FinitePrivateState::NotConfigured,
            Some(FinitePrivateReason::SettingsMissing),
        )
    } else {
        match facts.finite_private.fp_key {
            Tri::Absent => (
                FinitePrivateState::NotConfigured,
                Some(FinitePrivateReason::CredentialMissing),
            ),
            Tri::Unknown => (FinitePrivateState::Unknown, None),
            Tri::Present => (FinitePrivateState::Configured, None),
        }
    };
    FinitePrivateRouteStatus { state, reason }
}

/// A usable stored key: non-empty and not a `${…}` reference.
fn stored_key(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty() && !value.starts_with("${"))
}

fn openrouter_route(
    model: &Value,
    dotenv_key: Option<&str>,
    facts: &InferenceFacts,
) -> OpenRouterRouteStatus {
    let agent_key = stored_key(dotenv_key);
    let legacy_config_key = model
        .as_object()
        .filter(|_| model_provider(model).as_deref() == Some("openrouter"))
        .and_then(|model| stored_key(model.get("api_key").and_then(Value::as_str)));
    let key_source = if agent_key.is_some() {
        Some(KeySource::Agent)
    } else if legacy_config_key.is_some() {
        Some(KeySource::LegacyConfig)
    } else if facts.openrouter.hermes_key == Tri::Present {
        Some(KeySource::Environment)
    } else {
        None
    };
    let state = if key_source.is_some() {
        OpenRouterKeyState::KeySaved
    } else if facts.openrouter.hermes_key == Tri::Unknown {
        OpenRouterKeyState::Unknown
    } else {
        OpenRouterKeyState::NoKey
    };
    let key_hash = agent_key.map(|key| hex_digest(key.as_bytes()));
    let hermes_key = match facts.openrouter.hermes_key {
        Tri::Present
            if key_hash.is_some()
                && facts.openrouter.hermes_key_fingerprint.as_deref() == key_hash.as_deref() =>
        {
            HermesKey::SavedKey
        }
        Tri::Present => HermesKey::OtherKey,
        Tri::Absent => HermesKey::None,
        Tri::Unknown => HermesKey::Unknown,
    };
    OpenRouterRouteStatus {
        state,
        key_source,
        key_hash,
        hermes_key,
        other_pool_keys: facts.openrouter.manual_pool_entries,
    }
}

fn fallback_status(
    finite_private: &FinitePrivateRouteStatus,
    facts: &InferenceFacts,
) -> FallbackStatus {
    let fp_reason = finite_private.reason.map(|reason| match reason {
        FinitePrivateReason::SettingsMissing => FallbackReason::SettingsMissing,
        FinitePrivateReason::CredentialMissing => FallbackReason::CredentialMissing,
    });
    let status = |state, reason, model, extra_entries| FallbackStatus {
        state,
        reason,
        model,
        extra_entries,
    };
    let Some(effective) = facts.fallback.effective.as_deref() else {
        return status(FallbackState::Unknown, None, None, 0);
    };
    let Some(first) = effective.first() else {
        let keys = [
            facts.fallback.fallback_providers,
            facts.fallback.fallback_model,
        ];
        return if keys == [Tri::Absent, Tri::Absent] {
            status(FallbackState::NotConfigured, fp_reason, None, 0)
        } else if keys.contains(&Tri::Unknown) {
            status(FallbackState::Unknown, None, None, 0)
        } else {
            status(FallbackState::Off, None, None, 0)
        };
    };
    let extra_entries = effective.len() - 1;
    match first.owned_canonical {
        YesNo::Yes => {
            let provider_entry = facts.finite_private.provider_entry;
            let (state, reason) = if provider_entry == ProviderEntryFact::Canonical
                && finite_private.state == FinitePrivateState::Configured
            {
                (FallbackState::Configured, None)
            } else if provider_entry == ProviderEntryFact::Unknown
                || finite_private.state == FinitePrivateState::Unknown
            {
                (FallbackState::Unknown, None)
            } else {
                (
                    FallbackState::Unavailable,
                    Some(fp_reason.unwrap_or(FallbackReason::StaleConfig)),
                )
            };
            let model = Some(first.model.clone()).filter(|model| model.chars().count() <= 256);
            status(state, reason, model, extra_entries)
        }
        YesNo::Unknown => status(FallbackState::Unknown, None, None, extra_entries),
        YesNo::No => status(FallbackState::Custom, None, None, extra_entries),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::config::ConfigManager;
    use crate::connections::ConnectionManager;
    use crate::facts::FallbackEntryFact;
    use crate::ledger::Ledger;

    const FP_URL: &str = "https://fp.example.invalid/v1";

    fn fixtures() -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/saved-route");
        let mut cases = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let case = serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap();
                (name, case)
            })
            .collect::<Vec<_>>();
        cases.sort_by(|a, b| a.0.cmp(&b.0));
        cases
    }

    #[test]
    fn t_a1_classifier_matches_every_shared_fixture() {
        let cases = fixtures();
        assert!(cases.len() >= 18, "saved-route fixtures are missing");
        for (name, case) in cases {
            let expected = case["expected"].as_str().unwrap();
            let route = classify_saved_route(&case["model"], case["fp_base_url"].as_str());
            assert_eq!(
                serde_json::to_value(route).unwrap(),
                expected,
                "{name}: {}",
                case["description"]
            );
        }
    }

    #[test]
    fn endpoint_identity_preserves_path_case_and_rejects_uncertain_urls() {
        let same = |a: &str, b: &str| {
            let a = endpoint_identity(a);
            a.is_some() && a == endpoint_identity(b)
        };
        assert!(same(
            "HTTPS://Tenant.Example.INVALID/TenantA/v1/",
            "https://tenant.example.invalid/TenantA/v1"
        ));
        assert!(same(" https://h.invalid/v1 ", "https://h.invalid/v1//"));
        assert!(!same(
            "https://tenant.example.invalid/TenantA/v1",
            "https://tenant.example.invalid/tenanta/v1"
        ));
        assert!(!same("https://h.invalid:8443/v1", "https://h.invalid/v1"));
        assert!(!same("https://h.invalid/v1?x=1", "https://h.invalid/v1"));
        assert!(!same("https://u@h.invalid/v1", "https://h.invalid/v1"));
        assert!(!same("http://h.invalid/v1", "https://h.invalid/v1"));
        assert!(same("https://[::1]:8080/v1", "https://[::1]:8080/v1/"));
        for uncertain in [
            "",
            "h.invalid/v1",
            "https:/h.invalid/v1",
            "https:///v1",
            "https://h.invalid:port/v1",
            "https://h.invalid:70000/v1",
            "https://[::1/v1",
            "1https://h.invalid/v1",
        ] {
            assert_eq!(endpoint_identity(uncertain), None, "{uncertain}");
        }
        // An uncertain saved URL is never Finite Private, even against itself.
        let model = json!({"provider": "custom", "base_url": "https://h.invalid:port/v1"});
        assert_eq!(
            classify_saved_route(&model, Some("https://h.invalid:port/v1")),
            SavedRoute::Other
        );
        let model = json!({"provider": "custom", "base_url": "https://FP.example.invalid/v1/"});
        assert_eq!(
            classify_saved_route(&model, Some(FP_URL)),
            SavedRoute::FinitePrivate
        );
        assert_eq!(classify_saved_route(&model, None), SavedRoute::Other);
    }

    fn env(pairs: &[(&str, &str)]) -> FinitePrivateEnv {
        finite_private_env_from(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    fn fp_env() -> FinitePrivateEnv {
        env(&[
            ("FINITE_PRIVATE_MODEL", "glm-5-3-flash"),
            ("FINITE_PRIVATE_BASE_URL", FINITE_PRIVATE_PRODUCT_BASE_URL),
            ("FINITE_PRIVATE_CONTEXT_LENGTH", "393216"),
        ])
    }

    #[test]
    fn finite_private_env_applies_the_launcher_rewrites() {
        assert_eq!(
            fp_env(),
            FinitePrivateEnv {
                model: Some("glm-5-3-flash".to_owned()),
                base_url: Some(FINITE_PRIVATE_PRODUCT_BASE_URL.to_owned()),
                context_length: Some(393_216),
            }
        );
        let historical = env(&[
            ("FINITE_PRIVATE_MODEL", "glm-5-2"),
            (
                "FINITE_PRIVATE_BASE_URL",
                HISTORICAL_FINITE_PRIVATE_BASE_URL,
            ),
        ]);
        assert_eq!(historical.model.as_deref(), Some("glm-5-3-flash"));
        assert_eq!(
            historical.base_url.as_deref(),
            Some(FINITE_PRIVATE_PRODUCT_BASE_URL)
        );
        assert_eq!(historical.context_length, None);
        // A legacy model on another endpoint is left alone, as the launcher does.
        let custom = env(&[
            ("FINITE_PRIVATE_MODEL", "glm-5-2"),
            ("FINITE_PRIVATE_BASE_URL", FP_URL),
            ("FINITE_PRIVATE_CONTEXT_LENGTH", "not-a-number"),
        ]);
        assert_eq!(custom.model.as_deref(), Some("glm-5-2"));
        assert_eq!(custom.context_length, None);
        let blank = env(&[
            ("FINITE_PRIVATE_MODEL", " "),
            ("FINITE_PRIVATE_BASE_URL", FP_URL),
        ]);
        assert_eq!(blank.settings(), None);
        assert_eq!(
            blank.helper_env(),
            [
                ("FINITE_CONFIG_FP_MODEL", String::new()),
                ("FINITE_CONFIG_FP_BASE_URL", FP_URL.to_owned()),
                ("FINITE_CONFIG_FP_CONTEXT_LENGTH", String::new()),
            ]
        );
        assert_eq!(
            fp_env().helper_env()[2],
            ("FINITE_CONFIG_FP_CONTEXT_LENGTH", "393216".to_owned())
        );
    }

    #[test]
    fn planned_blocks_are_the_documented_values() {
        assert_eq!(
            plan_model_block(IntentRoute::FinitePrivate, None, &fp_env()).unwrap(),
            json!({
                "default": "glm-5-3-flash",
                "provider": "custom",
                "base_url": FINITE_PRIVATE_PRODUCT_BASE_URL,
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "api_mode": "chat_completions",
                "context_length": 393216,
                "supports_vision": true,
            })
        );
        let other = env(&[
            ("FINITE_PRIVATE_MODEL", "future-model"),
            ("FINITE_PRIVATE_BASE_URL", FP_URL),
        ]);
        assert_eq!(
            plan_model_block(IntentRoute::FinitePrivate, None, &other).unwrap(),
            json!({
                "default": "future-model",
                "provider": "custom",
                "base_url": FP_URL,
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "api_mode": "chat_completions",
            })
        );
        let error = plan_model_block(
            IntentRoute::FinitePrivate,
            None,
            &FinitePrivateEnv::default(),
        )
        .unwrap_err();
        assert_eq!(error.public_code(), "config_invalid");
        assert_eq!(
            error.public_message(),
            "Finite Private isn't available on this agent."
        );
        assert_eq!(
            plan_model_block(
                IntentRoute::Openrouter,
                Some("anthropic/claude-sonnet-4.6"),
                &fp_env()
            )
            .unwrap(),
            json!({
                "default": "anthropic/claude-sonnet-4.6",
                "provider": "openrouter",
                "base_url": "https://openrouter.ai/api/v1",
                "api_mode": "chat_completions",
            })
        );
        assert_eq!(
            plan_model_block(IntentRoute::OpenaiCodex, Some("gpt-5.5"), &fp_env()).unwrap(),
            json!({"default": "gpt-5.5", "provider": "openai-codex"})
        );
        for route in [IntentRoute::Openrouter, IntentRoute::OpenaiCodex] {
            assert_eq!(
                plan_model_block(route, None, &fp_env())
                    .unwrap_err()
                    .public_code(),
                "invalid_payload"
            );
        }
        // Every planned block classifies back to its own route.
        for (route, model, saved) in [
            (IntentRoute::FinitePrivate, None, SavedRoute::FinitePrivate),
            (IntentRoute::Openrouter, Some("a/b"), SavedRoute::Openrouter),
            (
                IntentRoute::OpenaiCodex,
                Some("gpt-5.5"),
                SavedRoute::OpenaiCodex,
            ),
        ] {
            let block = plan_model_block(route, model, &other).unwrap();
            assert_eq!(
                classify_saved_route(&block, other.base_url.as_deref()),
                saved
            );
        }
    }

    #[test]
    fn pr1_advertises_only_its_own_capabilities() {
        assert_eq!(
            capabilities(),
            [
                "inference.status.v2",
                "inference.select.v1",
                "inference.disconnect.v1"
            ]
        );
    }

    /// Facts for a healthy agent: FP key present, canonical provider entry and
    /// chain, no OpenRouter key anywhere.
    fn healthy_facts() -> InferenceFacts {
        let mut facts = InferenceFacts::unknown();
        facts.fallback.fallback_providers = Tri::Present;
        facts.fallback.fallback_model = Tri::Absent;
        facts.fallback.effective = Some(vec![canonical_entry()]);
        facts.finite_private.provider_entry = ProviderEntryFact::Canonical;
        facts.finite_private.fp_key = Tri::Present;
        facts.openrouter.hermes_key = Tri::Absent;
        facts.openrouter.dotenv_key = Tri::Absent;
        facts.openrouter.manual_pool_entries = PoolEntries::None;
        facts
    }

    fn canonical_entry() -> FallbackEntryFact {
        FallbackEntryFact {
            provider: "finite-private".to_owned(),
            model: "glm-5-3-flash".to_owned(),
            owned_canonical: YesNo::Yes,
        }
    }

    fn user_entry() -> FallbackEntryFact {
        FallbackEntryFact {
            provider: "anthropic".to_owned(),
            model: "claude-sonnet-4.6".to_owned(),
            owned_canonical: YesNo::No,
        }
    }

    #[test]
    fn t_a3_finite_private_state_covers_every_branch() {
        let route = |fp: &FinitePrivateEnv, fp_key: Tri| {
            let mut facts = healthy_facts();
            facts.finite_private.fp_key = fp_key;
            let route = finite_private_route(fp, &facts);
            (route.state, route.reason)
        };
        let no_model = env(&[("FINITE_PRIVATE_BASE_URL", FP_URL)]);
        let no_url = env(&[("FINITE_PRIVATE_MODEL", "glm-5-3-flash")]);
        for fp in [&no_model, &no_url, &FinitePrivateEnv::default()] {
            // Missing settings win over every credential fact.
            for fp_key in [Tri::Present, Tri::Absent, Tri::Unknown] {
                assert_eq!(
                    route(fp, fp_key),
                    (
                        FinitePrivateState::NotConfigured,
                        Some(FinitePrivateReason::SettingsMissing)
                    )
                );
            }
        }
        assert_eq!(
            route(&fp_env(), Tri::Absent),
            (
                FinitePrivateState::NotConfigured,
                Some(FinitePrivateReason::CredentialMissing)
            )
        );
        assert_eq!(
            route(&fp_env(), Tri::Unknown),
            (FinitePrivateState::Unknown, None)
        );
        assert_eq!(
            route(&fp_env(), Tri::Present),
            (FinitePrivateState::Configured, None)
        );
        // A failed helper leaves the FP route unknown, never configured.
        let status = derive_status(&Value::Null, None, &fp_env(), &InferenceFacts::unknown());
        assert_eq!(
            status.routes.finite_private.state,
            FinitePrivateState::Unknown
        );
    }

    fn fallback_with(change: impl FnOnce(&mut InferenceFacts), fp: &FinitePrivateEnv) -> Value {
        let mut facts = healthy_facts();
        change(&mut facts);
        serde_json::to_value(derive_status(&Value::Null, None, fp, &facts).fallback).unwrap()
    }

    fn fallback(state: &str, reason: Value, model: Value, extra_entries: usize) -> Value {
        json!({"state": state, "reason": reason, "model": model, "extra_entries": extra_entries})
    }

    type FactsChange = Box<dyn FnOnce(&mut InferenceFacts)>;

    #[test]
    fn t_a4_fallback_states() {
        let fp = fp_env();
        let fp_model = json!("glm-5-3-flash");
        let cases: Vec<(&str, FactsChange, FinitePrivateEnv, Value)> = vec![
            (
                "absent",
                Box::new(|facts| {
                    facts.fallback.fallback_providers = Tri::Absent;
                    facts.fallback.effective = Some(vec![]);
                }),
                fp.clone(),
                fallback("not_configured", Value::Null, Value::Null, 0),
            ),
            (
                "absent, FP credential missing",
                Box::new(|facts| {
                    facts.fallback.fallback_providers = Tri::Absent;
                    facts.fallback.effective = Some(vec![]);
                    facts.finite_private.fp_key = Tri::Absent;
                }),
                fp.clone(),
                fallback(
                    "not_configured",
                    json!("credential_missing"),
                    Value::Null,
                    0,
                ),
            ),
            (
                "absent, FP settings missing",
                Box::new(|facts| {
                    facts.fallback.fallback_providers = Tri::Absent;
                    facts.fallback.effective = Some(vec![]);
                }),
                FinitePrivateEnv::default(),
                fallback("not_configured", json!("settings_missing"), Value::Null, 0),
            ),
            (
                "empty list",
                Box::new(|facts| facts.fallback.effective = Some(vec![])),
                fp.clone(),
                fallback("off", Value::Null, Value::Null, 0),
            ),
            (
                "legacy fallback_model only",
                Box::new(|facts| {
                    facts.fallback.fallback_providers = Tri::Absent;
                    facts.fallback.fallback_model = Tri::Present;
                    facts.fallback.effective = Some(vec![user_entry()]);
                }),
                fp.clone(),
                fallback("custom", Value::Null, Value::Null, 0),
            ),
            (
                "empty list plus legacy",
                Box::new(|facts| {
                    facts.fallback.fallback_model = Tri::Present;
                    facts.fallback.effective = Some(vec![]);
                }),
                fp.clone(),
                fallback("off", Value::Null, Value::Null, 0),
            ),
            (
                "malformed chain",
                Box::new(|facts| facts.fallback.effective = Some(vec![])),
                fp.clone(),
                fallback("off", Value::Null, Value::Null, 0),
            ),
            (
                "empty chain with an unknown key",
                Box::new(|facts| {
                    facts.fallback.fallback_model = Tri::Unknown;
                    facts.fallback.effective = Some(vec![]);
                }),
                fp.clone(),
                fallback("unknown", Value::Null, Value::Null, 0),
            ),
            (
                "canonical",
                Box::new(|_| {}),
                fp.clone(),
                fallback("configured", Value::Null, fp_model.clone(), 0),
            ),
            (
                "canonical plus extra",
                Box::new(|facts| {
                    facts.fallback.effective = Some(vec![canonical_entry(), user_entry()]);
                }),
                fp.clone(),
                fallback("configured", Value::Null, fp_model.clone(), 1),
            ),
            (
                "non-canonical FP entry",
                Box::new(|facts| {
                    let mut entry = canonical_entry();
                    entry.owned_canonical = YesNo::No;
                    facts.fallback.effective = Some(vec![entry, user_entry()]);
                }),
                fp.clone(),
                fallback("custom", Value::Null, Value::Null, 1),
            ),
            (
                "stale provider entry",
                Box::new(|facts| {
                    facts.finite_private.provider_entry = ProviderEntryFact::Modified;
                }),
                fp.clone(),
                fallback("unavailable", json!("stale_config"), fp_model.clone(), 0),
            ),
            (
                "missing provider entry",
                Box::new(|facts| {
                    facts.finite_private.provider_entry = ProviderEntryFact::Absent;
                }),
                fp.clone(),
                fallback("unavailable", json!("stale_config"), fp_model.clone(), 0),
            ),
            (
                "FP credential missing",
                Box::new(|facts| facts.finite_private.fp_key = Tri::Absent),
                fp.clone(),
                fallback(
                    "unavailable",
                    json!("credential_missing"),
                    fp_model.clone(),
                    0,
                ),
            ),
            (
                "FP settings missing",
                Box::new(|_| {}),
                FinitePrivateEnv::default(),
                fallback(
                    "unavailable",
                    json!("settings_missing"),
                    fp_model.clone(),
                    0,
                ),
            ),
            (
                "FP credential unknown",
                Box::new(|facts| facts.finite_private.fp_key = Tri::Unknown),
                fp.clone(),
                fallback("unknown", Value::Null, fp_model.clone(), 0),
            ),
            (
                "provider entry unknown",
                Box::new(|facts| {
                    facts.finite_private.provider_entry = ProviderEntryFact::Unknown;
                }),
                fp.clone(),
                fallback("unknown", Value::Null, fp_model.clone(), 0),
            ),
            (
                "owned_canonical unknown",
                Box::new(|facts| {
                    let mut entry = canonical_entry();
                    entry.owned_canonical = YesNo::Unknown;
                    facts.fallback.effective = Some(vec![entry]);
                }),
                fp.clone(),
                fallback("unknown", Value::Null, Value::Null, 0),
            ),
            (
                "effective null",
                Box::new(|facts| facts.fallback.effective = None),
                fp.clone(),
                fallback("unknown", Value::Null, Value::Null, 0),
            ),
        ];
        for (name, change, fp, expected) in cases {
            assert_eq!(fallback_with(change, &fp), expected, "{name}");
        }
        assert_eq!(
            serde_json::to_value(
                derive_status(&Value::Null, None, &fp_env(), &InferenceFacts::unknown()).fallback
            )
            .unwrap(),
            fallback("unknown", Value::Null, Value::Null, 0)
        );
    }

    #[test]
    fn openrouter_key_source_hash_and_hermes_key() {
        let key = "sk-or-v1-synthetic-agent-key";
        let key_hash = hex_digest(key.as_bytes());
        let openrouter_model = json!({"default": "a/b", "provider": "openrouter", "api_key": "sk-or-v1-synthetic-legacy"});
        let route = |model: &Value, dotenv: Option<&str>, change: &dyn Fn(&mut InferenceFacts)| {
            let mut facts = healthy_facts();
            change(&mut facts);
            serde_json::to_value(
                derive_status(model, dotenv, &fp_env(), &facts)
                    .routes
                    .openrouter,
            )
            .unwrap()
        };
        let present_as = |fingerprint: Option<String>| {
            move |facts: &mut InferenceFacts| {
                facts.openrouter.hermes_key = Tri::Present;
                facts.openrouter.hermes_key_fingerprint = fingerprint.clone();
                facts.openrouter.manual_pool_entries = PoolEntries::Present;
            }
        };

        assert_eq!(
            route(&Value::Null, Some(key), &present_as(Some(key_hash.clone()))),
            json!({"state": "key_saved", "key_source": "agent", "key_hash": key_hash,
                   "hermes_key": "saved_key", "other_pool_keys": "present"})
        );
        assert_eq!(
            route(&Value::Null, Some(key), &present_as(Some("ab".repeat(32)))),
            json!({"state": "key_saved", "key_source": "agent", "key_hash": key_hash,
                   "hermes_key": "other_key", "other_pool_keys": "present"})
        );
        // A legacy config key counts only on an OpenRouter model block.
        assert_eq!(
            route(&openrouter_model, None, &|_| {}),
            json!({"state": "key_saved", "key_source": "legacy_config", "key_hash": null,
                   "hermes_key": "none", "other_pool_keys": "none"})
        );
        let mut codex_with_key = openrouter_model.clone();
        codex_with_key["provider"] = json!("openai-codex");
        assert_eq!(route(&codex_with_key, None, &|_| {})["state"], "no_key");
        // A key Hermes sees only from its environment; never hashed.
        assert_eq!(
            route(&Value::Null, None, &present_as(Some(key_hash.clone()))),
            json!({"state": "key_saved", "key_source": "environment", "key_hash": null,
                   "hermes_key": "other_key", "other_pool_keys": "present"})
        );
        // Empty and `${…}` values are not stored keys.
        for dotenv in [Some(""), Some("${OPENROUTER_API_KEY}")] {
            assert_eq!(
                route(&Value::Null, dotenv, &|_| {}),
                json!({"state": "no_key", "key_source": null, "key_hash": null,
                       "hermes_key": "none", "other_pool_keys": "none"})
            );
        }
        let unknown = |facts: &mut InferenceFacts| {
            facts.openrouter.hermes_key = Tri::Unknown;
            facts.openrouter.manual_pool_entries = PoolEntries::Unknown;
        };
        assert_eq!(
            route(&Value::Null, None, &unknown),
            json!({"state": "unknown", "key_source": null, "key_hash": null,
                   "hermes_key": "unknown", "other_pool_keys": "unknown"})
        );
        // agentd's own read still reports the saved key when the helper failed.
        assert_eq!(
            route(&Value::Null, Some(key), &unknown),
            json!({"state": "key_saved", "key_source": "agent", "key_hash": key_hash,
                   "hermes_key": "unknown", "other_pool_keys": "unknown"})
        );
    }

    fn connection_manager(config_yaml: &str) -> (tempfile::TempDir, ConnectionManager) {
        let temp = tempfile::tempdir().unwrap();
        let agent_home = temp.path().join("agent");
        let hermes_home = agent_home.join("hermes-home");
        fs::create_dir_all(&hermes_home).unwrap();
        fs::write(hermes_home.join("config.yaml"), config_yaml).unwrap();
        let ledger = Ledger::open(agent_home.join("agentd/ledger.sqlite3")).unwrap();
        let manager = ConnectionManager::new(
            &agent_home,
            &hermes_home,
            ConfigManager::new(hermes_home.join("config.yaml"), ledger),
        );
        (temp, manager)
    }

    /// Today's legacy `inference` fields, straight from `ConnectionManager`,
    /// as the exact bytes status serializes.
    fn legacy_inference_bytes(model: &Value) -> String {
        let yaml = serde_yaml::to_string(&json!({ "model": model })).unwrap();
        let (_temp, manager) = connection_manager(&yaml);
        serde_json::to_string(&manager.status().unwrap().inference).unwrap()
    }

    fn legacy_inference(model: &Value) -> Value {
        serde_json::from_str(&legacy_inference_bytes(model)).unwrap()
    }

    /// The whole `inference` object as status will compose it: the legacy
    /// fields beside the derived ones.
    fn composed_inference(model: &Value, facts: &InferenceFacts) -> Value {
        let mut inference = legacy_inference(model);
        let derived = serde_json::to_value(derive_status(model, None, &fp_env(), facts)).unwrap();
        inference
            .as_object_mut()
            .unwrap()
            .extend(derived.as_object().unwrap().clone());
        inference
    }

    #[test]
    fn t_a2_status_golden_per_saved_shape() {
        let fp_block = plan_model_block(IntentRoute::FinitePrivate, None, &fp_env()).unwrap();
        let routes = json!({
            "finite_private": {"state": "configured", "reason": null},
            "openrouter": {"state": "no_key", "key_source": null, "key_hash": null,
                           "hermes_key": "none", "other_pool_keys": "none"}
        });
        let fallback = json!({"state": "configured", "reason": null, "model": "glm-5-3-flash", "extra_entries": 0});
        let cases = [
            (
                fp_block,
                json!({"profile": "finite_private", "provider": "custom", "model": "glm-5-3-flash"}),
                json!({"route": "finite_private", "provider": "custom", "model": "glm-5-3-flash"}),
            ),
            (
                json!({"default": "glm-5-3-flash", "provider": "finite-private"}),
                json!({"profile": "finite_private", "provider": "finite-private", "model": "glm-5-3-flash"}),
                json!({"route": "finite_private", "provider": "finite-private", "model": "glm-5-3-flash"}),
            ),
            (
                json!({"default": "anthropic/claude-sonnet-4.6", "provider": "openrouter"}),
                json!({"profile": "openrouter", "provider": "openrouter", "model": "anthropic/claude-sonnet-4.6"}),
                json!({"route": "openrouter", "provider": "openrouter", "model": "anthropic/claude-sonnet-4.6"}),
            ),
            (
                json!({"default": "gpt-5.5", "provider": "openai-codex"}),
                json!({"profile": "finite_private", "provider": "openai-codex", "model": "gpt-5.5"}),
                json!({"route": "openai_codex", "provider": "openai-codex", "model": "gpt-5.5"}),
            ),
            (
                json!({"default": "local-model", "provider": "custom", "base_url": "https://llm.example.invalid/v1"}),
                json!({"profile": "finite_private", "provider": "custom", "model": "local-model"}),
                json!({"route": "other", "provider": "custom", "model": "local-model"}),
            ),
            (
                Value::Null,
                json!({"profile": "finite_private", "provider": "custom", "model": "Unknown model"}),
                json!({"route": "other", "provider": null, "model": null}),
            ),
        ];
        for (model, legacy, saved) in cases {
            let mut expected = legacy.clone();
            expected["saved"] = saved;
            expected["routes"] = routes.clone();
            expected["fallback"] = fallback.clone();
            expected["operation"] = Value::Null;
            let composed = composed_inference(&model, &healthy_facts());
            assert_eq!(composed, expected, "{model}");
            // The legacy fields keep today's names, order, and bytes.
            assert_eq!(
                legacy_inference_bytes(&model),
                format!(
                    "{{\"profile\":{},\"provider\":{},\"model\":{}}}",
                    legacy["profile"], legacy["provider"], legacy["model"]
                )
            );
            // State names are stored facts, never claims that a route works.
            let text = serde_json::to_string(&composed).unwrap();
            for word in ["ready", "valid", "working", "healthy"] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
    }

    #[test]
    fn t_a2_operation_and_codex_fields_serialize_as_documented() {
        let mut record =
            IntentRecord::new(IntentKind::Disconnect, IntentRoute::Openrouter, None).unwrap();
        record.phase = IntentPhase::Cleanup;
        record.state = IntentState::Failed;
        record.error_code = Some("verify_failed".to_owned());
        record.attempts = 3;
        record.updated_at_ms = 1_790_000_000_000;
        let mut status = derive_status(&Value::Null, None, &fp_env(), &healthy_facts());
        status.operation = Some(OperationStatus::from_record(&record));
        let value = serde_json::to_value(&status).unwrap();
        assert!(value["routes"].get("openai_codex").is_none());
        assert_eq!(
            value["operation"],
            json!({"id": record.id, "kind": "disconnect", "route": "openrouter", "model": null,
                   "state": "failed", "phase": "cleanup", "error_code": "verify_failed",
                   "attempts": 3, "updated_at_ms": 1_790_000_000_000_u64})
        );
        status.routes.openai_codex = Some(CodexRouteStatus {
            state: crate::facts::CodexStateFact::SignInRequired,
            quota_reset_at_ms: None,
            reported_quota_reset_at_ms: Some(1_790_000_000_000),
            login: None,
        });
        assert_eq!(
            serde_json::to_value(&status).unwrap()["routes"]["openai_codex"],
            json!({"state": "sign_in_required", "quota_reset_at_ms": null,
                   "reported_quota_reset_at_ms": 1_790_000_000_000_u64, "login": null})
        );
    }

    #[test]
    fn saved_strings_over_their_bound_are_null() {
        let model = json!({"default": "m".repeat(257), "provider": "p".repeat(129)});
        let saved = derive_status(&model, None, &fp_env(), &healthy_facts()).saved;
        assert_eq!(saved.provider, None);
        assert_eq!(saved.model, None);
    }

    #[test]
    fn t_a5_legacy_profile_never_takes_a_new_value() {
        let mut models = fixtures()
            .into_iter()
            .map(|(_, case)| case["model"].clone())
            .collect::<Vec<_>>();
        models.push(json!({"default": "x", "provider": "openai-codex"}));
        models.push(json!({"default": "x", "provider": 7}));
        for model in models {
            let legacy = legacy_inference(&model);
            let expected = if model_provider(&model).as_deref() == Some("openrouter") {
                "openrouter"
            } else {
                "finite_private"
            };
            assert_eq!(legacy["profile"], expected, "{model}");
        }
    }
}
