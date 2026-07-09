use std::error::Error;
use std::net::SocketAddr;

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
    let identity_authority_url = std::env::var("FINITE_IDENTITY_AUTHORITY")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let smoke_nip07_secret = std::env::var("FINITE_BRAIN_SMOKE_NIP07_SECRET")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let smoke_email_proofs = std::env::var("FINITE_BRAIN_SMOKE_EMAIL_PROOFS")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if smoke_email_proofs.is_some() && identity_authority_url.is_some() {
        return Err(
            "FINITE_BRAIN_SMOKE_EMAIL_PROOFS cannot be combined with FINITE_IDENTITY_AUTHORITY"
                .into(),
        );
    }
    if smoke_email_proofs.is_some() && smoke_nip07_secret.is_none() {
        return Err(
            "FINITE_BRAIN_SMOKE_EMAIL_PROOFS requires FINITE_BRAIN_SMOKE_NIP07_SECRET".into(),
        );
    }
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("FiniteBrain smoke server listening on http://{address}");

    let mut state =
        finite_brain_server::server_state_with_sqlite_path(database_path, public_base_url)?;
    if let Some(url) = identity_authority_url {
        state = state.with_identity_authority_url(url);
    }
    if let Some(secret) = smoke_nip07_secret {
        state = state.with_smoke_nip07_signer(secret).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid FINITE_BRAIN_SMOKE_NIP07_SECRET: {error}"),
            )
        })?;
    }
    if let Some(email_proofs) = smoke_email_proofs {
        state = state
            .with_smoke_email_proofs(email_proofs)
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid FINITE_BRAIN_SMOKE_EMAIL_PROOFS: {error}"),
                )
            })?;
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
            "postmark" => {
                let token = std::env::var("POSTMARK_SERVER_TOKEN")?;
                let from = std::env::var("FINITE_BRAIN_INVITE_MAIL_FROM")?;
                state = state.with_postmark_invite_mailer(token, from);
            }
            other => {
                return Err(
                    format!("unsupported FINITE_BRAIN_INVITE_MAILER value: {other}").into(),
                );
            }
        }
    }
    let router = finite_brain_server::router_with_state(state);
    axum::serve(listener, router).await?;

    Ok(())
}
