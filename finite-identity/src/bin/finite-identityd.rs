use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use finite_identity::authority::{
    AuthorityConfig, AuthorityState, DevMailer, HttpMailer, IdentityStore, SystemClock,
    public_router, router,
};

#[tokio::main]
async fn main() {
    if let Err(message) = run(std::env::args().skip(1).collect()).await {
        eprintln!("finite-identityd: {message}");
        std::process::exit(2);
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("serve") {
        return Err(usage());
    }
    let data = flag_value(&args, "--data").ok_or_else(usage)?;
    let listen = flag_value(&args, "--listen").unwrap_or_else(|| "127.0.0.1:8790".to_owned());
    let public_listen =
        flag_value(&args, "--public-listen").unwrap_or_else(|| "127.0.0.1:8791".to_owned());
    let external_base_url =
        flag_value(&args, "--external-base-url").ok_or("--external-base-url URL is required")?;
    let mailer = configure_mailer(&args)?;
    let finite_vip_domain =
        flag_value(&args, "--finite-vip-domain").unwrap_or_else(|| "finite.vip".to_owned());
    let operator_token =
        configured_operator_token(&args, std::env::var("FINITE_IDENTITY_OPERATOR_TOKEN").ok());
    let name_lookup_token = configured_name_lookup_token(
        std::env::var("FINITE_IDENTITY_NAME_LOOKUP_TOKEN").ok(),
        operator_token.as_deref(),
    )?;
    let address: SocketAddr = listen
        .parse()
        .map_err(|error| format!("invalid --listen address: {error}"))?;
    require_loopback_for_name_lookup(address, name_lookup_token.is_some())?;
    let public_address: SocketAddr = public_listen
        .parse()
        .map_err(|error| format!("invalid --public-listen address: {error}"))?;
    let data_dir = PathBuf::from(data);
    let store = IdentityStore::open(data_dir.join("identity.db"))
        .map_err(|error| format!("cannot open identity store: {error}"))?;
    let state = AuthorityState::new(
        store,
        mailer,
        SystemClock,
        AuthorityConfig {
            external_base_url,
            finite_vip_domain,
            email_challenge_ttl_seconds: 15 * 60,
            operator_token,
        },
    )
    .with_name_lookup_token(name_lookup_token);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("cannot bind {address}: {error}"))?;
    let public_listener = tokio::net::TcpListener::bind(public_address)
        .await
        .map_err(|error| format!("cannot bind {public_address}: {error}"))?;
    let full_server = axum::serve(listener, router(state.clone()));
    let public_server = axum::serve(public_listener, public_router(state));
    let (full_result, public_result) = tokio::join!(async move { full_server.await }, async move {
        public_server.await
    },);
    full_result
        .and(public_result)
        .map_err(|error| format!("server error: {error}"))
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == name).then(|| window[1].clone()))
}

fn configured_operator_token(args: &[String], environment: Option<String>) -> Option<String> {
    flag_value(args, "--operator-token")
        .or(environment)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The read-only name-lookup credential comes only from the service
/// environment and must differ from the operator credential, so a product
/// holding it can never reach operator endpoints.
fn configured_name_lookup_token(
    environment: Option<String>,
    operator_token: Option<&str>,
) -> Result<Option<String>, String> {
    let token = environment
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if token.is_some() && token.as_deref() == operator_token {
        return Err(
            "FINITE_IDENTITY_NAME_LOOKUP_TOKEN must differ from the operator token".to_owned(),
        );
    }
    Ok(token)
}

/// Exact-key name lookup is served only on the private `--listen` router.
/// With the capability enabled that listener must be a loopback address, so
/// the credential-gated route is never reachable off-host by misconfiguration.
fn require_loopback_for_name_lookup(
    address: SocketAddr,
    name_lookup_enabled: bool,
) -> Result<(), String> {
    if name_lookup_enabled && !address.ip().is_loopback() {
        return Err(format!(
            "FINITE_IDENTITY_NAME_LOOKUP_TOKEN requires a loopback --listen address; got {address}"
        ));
    }
    Ok(())
}

fn configure_mailer(
    args: &[String],
) -> Result<Arc<dyn finite_identity::authority::Mailer>, String> {
    let mailer = flag_value(args, "--mailer").unwrap_or_else(|| "dev".to_owned());
    match mailer.as_str() {
        "dev" => {
            if flag_value(args, "--dev-print-email-tokens").as_deref() != Some("yes") {
                return Err(
                    "--mailer dev requires --dev-print-email-tokens yes so token-printing is explicit"
                        .to_owned(),
                );
            }
            Ok(Arc::new(DevMailer))
        }
        "resend" => {
            if flag_value(args, "--dev-print-email-tokens").is_some() {
                return Err("--dev-print-email-tokens is only valid with --mailer dev".to_owned());
            }
            let from_address = flag_value(args, "--mail-from")
                .ok_or("--mailer resend requires --mail-from ADDR")?;
            let env_var = finite_mail::RESEND_API_KEY_ENV_VAR;
            let api_key = std::env::var(env_var).map_err(|_| {
                format!("--mailer resend requires the {env_var} environment variable")
            })?;
            Ok(Arc::new(HttpMailer::new(api_key, from_address)))
        }
        raw => Err(format!("unknown --mailer `{raw}` (dev|resend)")),
    }
}

fn usage() -> String {
    "usage: finite-identityd serve --data DIR --external-base-url URL [--listen 127.0.0.1:8790] [--public-listen 127.0.0.1:8791] [--finite-vip-domain finite.vip] [--operator-token TOKEN] [--mailer dev --dev-print-email-tokens yes | --mailer resend --mail-from ADDR]".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn dev_mailer_requires_explicit_token_printing() {
        let error = match configure_mailer(&args(&["serve"])) {
            Ok(_) => panic!("expected dev mailer without explicit token printing to fail"),
            Err(error) => error,
        };
        assert!(error.contains("--dev-print-email-tokens yes"));
        assert!(
            configure_mailer(&args(&[
                "serve",
                "--mailer",
                "dev",
                "--dev-print-email-tokens",
                "yes",
            ]))
            .is_ok()
        );
    }

    #[test]
    fn production_mailer_rejects_dev_token_printing_and_requires_sender() {
        let error = match configure_mailer(&args(&[
            "serve",
            "--mailer",
            "resend",
            "--dev-print-email-tokens",
            "yes",
        ])) {
            Ok(_) => panic!("expected production mailer with dev token printing to fail"),
            Err(error) => error,
        };
        assert!(error.contains("only valid with --mailer dev"));

        let error = match configure_mailer(&args(&["serve", "--mailer", "resend"])) {
            Ok(_) => panic!("expected production mailer without sender to fail"),
            Err(error) => error,
        };
        assert!(error.contains("--mail-from"));

        let error = match configure_mailer(&args(&["serve", "--mailer", "postmark"])) {
            Ok(_) => panic!("expected removed provider to be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("unknown --mailer `postmark` (dev|resend)"));
    }

    #[test]
    fn operator_token_prefers_explicit_flag_and_accepts_secret_environment() {
        assert_eq!(
            configured_operator_token(
                &args(&["serve", "--operator-token", "flag-token"]),
                Some("environment-token".to_string()),
            )
            .as_deref(),
            Some("flag-token")
        );
        assert_eq!(
            configured_operator_token(&args(&["serve"]), Some(" environment-token ".to_string()))
                .as_deref(),
            Some("environment-token")
        );
        assert_eq!(configured_operator_token(&args(&["serve"]), None), None);
    }

    #[test]
    fn name_lookup_token_is_optional_and_never_the_operator_token() {
        assert_eq!(configured_name_lookup_token(None, Some("op")), Ok(None));
        assert_eq!(
            configured_name_lookup_token(Some("  ".to_owned()), Some("op")),
            Ok(None)
        );
        assert_eq!(
            configured_name_lookup_token(Some(" lookup ".to_owned()), Some("op")),
            Ok(Some("lookup".to_owned()))
        );
        assert!(configured_name_lookup_token(Some("op".to_owned()), Some("op")).is_err());
    }

    #[test]
    fn name_lookup_requires_a_loopback_private_listener() {
        for loopback in ["127.0.0.1:8790", "127.0.0.2:8790", "[::1]:8790"] {
            assert!(require_loopback_for_name_lookup(loopback.parse().unwrap(), true).is_ok());
        }
        for external in [
            "0.0.0.0:8790",
            "10.0.0.5:8790",
            "[::]:8790",
            "[2001:db8::1]:8790",
        ] {
            let address = external.parse().unwrap();
            assert!(
                require_loopback_for_name_lookup(address, true).is_err(),
                "{external}"
            );
            // Without the capability the existing listener contract is unchanged.
            assert!(require_loopback_for_name_lookup(address, false).is_ok());
        }
    }
}
