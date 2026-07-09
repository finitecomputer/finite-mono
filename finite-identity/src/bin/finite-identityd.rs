use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use finite_identity::authority::{
    AuthorityConfig, AuthorityState, DevMailer, HttpMailer, IdentityStore, MailProvider,
    SystemClock, router,
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
    let external_base_url =
        flag_value(&args, "--external-base-url").ok_or("--external-base-url URL is required")?;
    let mailer = configure_mailer(&args)?;
    let finite_vip_domain =
        flag_value(&args, "--finite-vip-domain").unwrap_or_else(|| "finite.vip".to_owned());
    let operator_token = flag_value(&args, "--operator-token");
    let address: SocketAddr = listen
        .parse()
        .map_err(|error| format!("invalid --listen address: {error}"))?;
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
    );
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("cannot bind {address}: {error}"))?;
    axum::serve(listener, router(state))
        .await
        .map_err(|error| format!("server error: {error}"))
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == name).then(|| window[1].clone()))
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
        raw => {
            let provider = MailProvider::parse(raw)
                .ok_or_else(|| format!("unknown --mailer `{raw}` (dev|resend|postmark)"))?;
            if flag_value(args, "--dev-print-email-tokens").is_some() {
                return Err("--dev-print-email-tokens is only valid with --mailer dev".to_owned());
            }
            let from_address = flag_value(args, "--mail-from")
                .ok_or("--mailer resend|postmark requires --mail-from ADDR")?;
            let env_var = provider.api_key_env_var();
            let api_key = std::env::var(env_var).map_err(|_| {
                format!("--mailer {raw} requires the {env_var} environment variable")
            })?;
            Ok(Arc::new(HttpMailer::new(provider, api_key, from_address)))
        }
    }
}

fn usage() -> String {
    "usage: finite-identityd serve --data DIR --external-base-url URL [--listen 127.0.0.1:8790] [--finite-vip-domain finite.vip] [--operator-token TOKEN] [--mailer dev --dev-print-email-tokens yes | --mailer resend|postmark --mail-from ADDR]".to_owned()
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

        let error = match configure_mailer(&args(&["serve", "--mailer", "postmark"])) {
            Ok(_) => panic!("expected production mailer without sender to fail"),
            Err(error) => error,
        };
        assert!(error.contains("--mail-from"));
    }
}
