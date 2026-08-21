use std::error::Error;
use std::net::SocketAddr;

/// Parse `FINITE_BRAIN_PROTECTED_RATE_LIMIT` as `max_requests:window_seconds`.
/// Unset or empty keeps the production defaults; malformed values fail closed.
fn protected_rate_limit_override(value: Option<String>) -> Result<Option<(u32, u64)>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let (max_requests, window_seconds) = value
        .split_once(':')
        .ok_or_else(|| "expected \"max_requests:window_seconds\"".to_owned())?;
    let max_requests = max_requests
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid max_requests: {error}"))?;
    let window_seconds = window_seconds
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid window_seconds: {error}"))?;
    if max_requests == 0 || window_seconds == 0 {
        return Err("max_requests and window_seconds must be at least 1".to_owned());
    }
    Ok(Some((max_requests, window_seconds)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args()
        .nth(1)
        .is_some_and(|arg| matches!(arg.as_str(), "version" | "--version" | "-V"))
    {
        println!("finite-brain {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let address = std::env::var("FINITE_BRAIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3015".to_owned())
        .parse::<SocketAddr>()?;
    let public_base_url = std::env::var("FINITE_BRAIN_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("http://{address}"));
    let database_path =
        std::env::var("FINITE_BRAIN_DB").unwrap_or_else(|_| "finite-brain.sqlite3".to_owned());
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("FiniteBrain smoke server listening on http://{address}");

    let mut state =
        finite_brain_server::server_state_with_sqlite_path(database_path, public_base_url)?;
    if let Some((max_requests, window_seconds)) =
        protected_rate_limit_override(std::env::var("FINITE_BRAIN_PROTECTED_RATE_LIMIT").ok())
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid FINITE_BRAIN_PROTECTED_RATE_LIMIT: {error}"),
                )
            })?
    {
        state = state.with_rate_limit(max_requests, window_seconds);
    }
    if let Ok(mailer) = std::env::var("FINITE_BRAIN_INVITE_MAILER") {
        match mailer.trim() {
            "" | "none" => {}
            "dev" => {
                state = state.with_dev_invite_mailer();
            }
            "resend" => {
                let api_key = std::env::var("RESEND_API_KEY")?;
                let from = std::env::var("FINITE_BRAIN_INVITE_MAIL_FROM")?;
                state = state.with_resend_invite_mailer(api_key, from);
            }
            other => {
                return Err(format!(
                    "unsupported FINITE_BRAIN_INVITE_MAILER value: {other} (none|dev|resend)"
                )
                .into());
            }
        }
    }
    let router = finite_brain_server::router_with_state(state);
    axum::serve(listener, router).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::protected_rate_limit_override;

    #[test]
    fn protected_rate_limit_unset_or_empty_keeps_defaults() {
        assert_eq!(protected_rate_limit_override(None).unwrap(), None);
        assert_eq!(
            protected_rate_limit_override(Some(String::new())).unwrap(),
            None
        );
        assert_eq!(
            protected_rate_limit_override(Some("   ".to_owned())).unwrap(),
            None
        );
    }

    #[test]
    fn protected_rate_limit_parses_max_requests_and_window() {
        assert_eq!(
            protected_rate_limit_override(Some("10000:60".to_owned())).unwrap(),
            Some((10000, 60))
        );
        assert_eq!(
            protected_rate_limit_override(Some(" 120 : 30 ".to_owned())).unwrap(),
            Some((120, 30))
        );
    }

    #[test]
    fn protected_rate_limit_rejects_malformed_values() {
        for value in [
            "10000",
            "10000:",
            ":60",
            "abc:60",
            "10000:abc",
            "10000:60:5",
            "0:60",
            "10000:0",
            "-1:60",
        ] {
            assert!(
                protected_rate_limit_override(Some(value.to_owned())).is_err(),
                "expected {value:?} to be rejected"
            );
        }
    }
}
