use std::env;

use anyhow::Result;
use devfinity::run_devfinity_test;

#[test]
#[ignore = "starts a devfinity stack; run `just dev rust-smoke`"]
fn backend_services_are_ready() -> Result<()> {
    run_devfinity_test(|stack| {
        assert!(stack.paths().ready_file.exists());

        let env = DevfinityEnv::from_env()?;
        assert_eq!(env.core_url, stack.core_url());
        assert_eq!(env.finitechat_url, stack.finitechat_url());
        assert_eq!(env.finitesites_api_url, stack.finitesites_api_url());

        assert_http_contains("core", &format!("{}/healthz", env.core_url), "\"ok\":true")?;
        assert_http_contains(
            "finitechat",
            &format!("{}/health", env.finitechat_url),
            "\"status\":\"ok\"",
        )?;
        assert_http_contains(
            "finitesites",
            &format!("{}/api/v1/healthz", env.finitesites_api_url),
            "\"ok\":true",
        )?;

        println!("devfinity backend rust smoke ok");
        Ok(())
    })
}

struct DevfinityEnv {
    core_url: String,
    finitechat_url: String,
    finitesites_api_url: String,
}

impl DevfinityEnv {
    fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            core_url: trim_trailing_slash(env::var("FC_CORE_URL")?),
            finitechat_url: trim_trailing_slash(env::var("FINITECHAT_SERVER_URL")?),
            finitesites_api_url: trim_trailing_slash(env::var("FINITE_SITES_API")?),
        })
    }
}

fn assert_http_contains(name: &str, url: &str, expected: &str) -> Result<()> {
    let body = ureq::get(url).call()?.into_string()?;
    assert!(
        body.contains(expected),
        "{name} response from {url} did not contain {expected:?}: {body}"
    );
    Ok(())
}

fn trim_trailing_slash(value: String) -> String {
    value.trim_end_matches('/').to_string()
}
