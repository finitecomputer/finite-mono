//! Fetch public runtime flags before the canonical image entrypoint starts any
//! child. Command::envs avoids mutating a multithreaded process's environment.
use crate::AgentdError;

#[cfg(unix)]
pub async fn bootstrap() -> Result<(), AgentdError> {
    use std::os::unix::process::CommandExt;
    // Substrate warms every template in a temporary golden actor. Never let
    // that actor enroll, connect chat, or write a real agent's identity.
    let atespace = std::fs::read_to_string("/run/ate/atespace")?;
    if atespace == "ate-golden" {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
        axum::serve(
            listener,
            axum::Router::new().route("/healthz", axum::routing::get(|| async { "ready" })),
        )
        .await?;
        return Ok(());
    }
    let mut environment: std::collections::BTreeMap<String, String> =
        match std::env::var("FINITE_BOOTSTRAP_ENV_JSON") {
            Ok(value) if value.len() <= 32768 => serde_json::from_str(&value)
                .map_err(|_| AgentdError::Config("invalid bootstrap environment".into()))?,
            Ok(_) => {
                return Err(AgentdError::Config(
                    "bootstrap environment exceeds its limit".into(),
                ));
            }
            Err(std::env::VarError::NotPresent) => Default::default(),
            Err(_) => return Err(AgentdError::Config("invalid bootstrap environment".into())),
        };
    if environment
        .keys()
        .any(|key| key.starts_with("FINITE_CORE_"))
    {
        return Err(AgentdError::Config(
            "bootstrap cannot replace Core authority".into(),
        ));
    }
    environment.remove("FINITE_BOOTSTRAP_INFERENCE_KEY_ID");
    environment.extend(crate::hosted_hermes_pull::boot_environment().await?);
    if environment
        .get("FINITE_DESKTOP_ENABLED")
        .is_some_and(|value| value == "1")
    {
        environment.insert("DISPLAY".into(), ":99".into());
    }
    let error = std::process::Command::new("/opt/agent-entrypoint.sh")
        .args(["/runtime/bin/finite-agentd", "serve"])
        .env_remove("FINITE_BOOTSTRAP_ENV_JSON")
        .envs(environment)
        .exec();
    Err(error.into())
}
