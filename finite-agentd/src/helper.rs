//! agentd's side of the helper process contract (§8.2): runs
//! `python -m hermes_cli.finite_inference_helper <sub>` inside the Hermes
//! environment and reads its one JSON result from stdout.
//!
//! The helper gets the environment a Hermes process gets from agentd after the
//! §5.6 launch rule, plus `HERMES_HOME` and `FINITE_CONFIG_FP_*`. That
//! environment is built explicitly and the child starts from an empty one, so
//! the four helper test variables can never reach a production helper even
//! when agentd's own environment carries them.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

use crate::AgentdError;
use crate::facts::InferenceFacts;
use crate::hosted_hermes::{CODEX_HOME_DISABLED, openai_key_is_finite_private_alias};
use crate::inference::{FinitePrivateEnv, finite_private_env};
use crate::supervisor::signal_group;

/// Test-only helper variables (§8.2). agentd never passes them on.
pub(crate) const HELPER_TEST_VARIABLES: [&str; 4] = [
    "FINITE_CODEX_AUTH_ISSUER",
    "FINITE_CODEX_LOGIN_DEADLINE_S",
    "FINITE_HELPER_TEST_BARRIER",
    "FINITE_HELPER_TEST_BARRIER_FILE",
];
/// Test-only agentd variables (§12.1) that point agentd at a fake helper.
const PYTHON_OVERRIDE: &str = "FINITE_AGENTD_INFERENCE_HELPER_PYTHON";
const MODULE_OVERRIDE: &str = "FINITE_AGENTD_INFERENCE_HELPER_MODULE";
const HELPER_MODULE: &str = "hermes_cli.finite_inference_helper";
const FACTS_DEADLINE: Duration = Duration::from_secs(10);
/// Longer than Hermes's 15 s auth-store lock.
const CLEAR_DEADLINE: Duration = Duration::from_secs(20);

/// The providers `clear-auth` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only `clear_auth` takes it, and agentd has no caller"
    )
)]
pub(crate) enum HelperProvider {
    Openrouter,
    OpenaiCodex,
}

impl HelperProvider {
    fn arg(self) -> &'static str {
        match self {
            Self::Openrouter => "openrouter",
            Self::OpenaiCodex => "openai-codex",
        }
    }
}

/// The interpreter and module agentd runs.
struct HelperCommand {
    python: OsString,
    module: String,
}

impl HelperCommand {
    fn from_env() -> Self {
        Self {
            python: std::env::var_os(PYTHON_OVERRIDE).unwrap_or_else(|| "python".into()),
            module: std::env::var(MODULE_OVERRIDE).unwrap_or_else(|_| HELPER_MODULE.to_owned()),
        }
    }
}

/// Read-only redacted facts (§8.2.1).
pub(crate) async fn inference_facts(hermes_home: &Path) -> Result<InferenceFacts, AgentdError> {
    let value = run_helper(
        &HelperCommand::from_env(),
        helper_env(std::env::vars_os(), hermes_home, &finite_private_env()),
        &["inference-facts"],
        FACTS_DEADLINE,
    )
    .await?;
    parse_facts(value)
}

/// Upstream `clear_provider_auth` for one provider. `true` if anything was cleared.
#[expect(
    dead_code,
    reason = "no agentd caller: the launcher's pending-disconnect step clears (§3.7)"
)]
pub(crate) async fn clear_auth(
    hermes_home: &Path,
    provider: HelperProvider,
) -> Result<bool, AgentdError> {
    let value = run_helper(
        &HelperCommand::from_env(),
        helper_env(std::env::vars_os(), hermes_home, &finite_private_env()),
        &["clear-auth", "--provider", provider.arg()],
        CLEAR_DEADLINE,
    )
    .await?;
    parse_cleared_auth(&value)
}

/// Clears conversation overrides naming any of `providers`. Only safe with no
/// gateway running (X7); the launcher's pending-disconnect step is the normal
/// caller.
#[expect(
    dead_code,
    reason = "no agentd caller: the launcher's pending-disconnect step clears (§3.7)"
)]
pub(crate) async fn clear_session_overrides(
    hermes_home: &Path,
    providers: &[&str],
) -> Result<u32, AgentdError> {
    let mut args = vec!["clear-session-overrides", "--provider"];
    args.extend_from_slice(providers);
    let value = run_helper(
        &HelperCommand::from_env(),
        helper_env(std::env::vars_os(), hermes_home, &finite_private_env()),
        &args,
        CLEAR_DEADLINE,
    )
    .await?;
    parse_cleared_count(&value)
}

/// agentd's environment after the §5.6 launch rule, without the helper test
/// variables, plus `HERMES_HOME` and `FINITE_CONFIG_FP_*`.
fn helper_env(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    hermes_home: &Path,
    fp: &FinitePrivateEnv,
) -> BTreeMap<OsString, OsString> {
    let mut environment = base.into_iter().collect::<BTreeMap<_, _>>();
    for name in HELPER_TEST_VARIABLES {
        environment.remove(&OsString::from(name));
    }
    if openai_key_is_finite_private_alias(|name| environment.get(&OsString::from(name)).cloned()) {
        environment.remove(&OsString::from("OPENAI_API_KEY"));
    }
    environment.insert("CODEX_HOME".into(), CODEX_HOME_DISABLED.into());
    environment.insert("HERMES_HOME".into(), hermes_home.into());
    for (name, value) in fp.helper_env() {
        environment.insert(name.into(), value.into());
    }
    environment
}

async fn run_helper(
    command: &HelperCommand,
    environment: BTreeMap<OsString, OsString>,
    args: &[&str],
    deadline: Duration,
) -> Result<Value, AgentdError> {
    let child = Command::new(&command.python)
        .arg("-m")
        .arg(&command.module)
        .args(args)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let pid = child.id();
    let output = match tokio::time::timeout(deadline, child.wait_with_output()).await {
        Ok(output) => output.map_err(|_| unavailable())?,
        Err(_) => {
            // The direct child is killed on drop; sweep its group too.
            if let Some(pid) = pid {
                signal_group(pid, rustix::process::Signal::KILL);
            }
            return Err(unavailable());
        }
    };
    if !output.status.success() {
        return Err(unavailable());
    }
    let stdout = String::from_utf8(output.stdout).map_err(|_| unavailable())?;
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(unavailable)?;
    serde_json::from_str(line).map_err(|_| unavailable())
}

fn parse_facts(value: Value) -> Result<InferenceFacts, AgentdError> {
    serde_json::from_value(value).map_err(|_| unavailable())
}

fn parse_cleared_auth(value: &Value) -> Result<bool, AgentdError> {
    match value.get("cleared").and_then(Value::as_str) {
        Some("yes") => Ok(true),
        Some("no") => Ok(false),
        _ => Err(unavailable()),
    }
}

fn parse_cleared_count(value: &Value) -> Result<u32, AgentdError> {
    value
        .get("cleared")
        .and_then(Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(unavailable)
}

/// Helper failures never carry helper output, which could echo Hermes state.
fn unavailable() -> AgentdError {
    AgentdError::ProviderUnavailable("The agent's inference helper is unavailable.".to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    use super::*;
    use crate::facts::{FactsCache, Tri};

    const FP_KEY: &str = "synthetic-finite-private-key";

    fn fp() -> FinitePrivateEnv {
        FinitePrivateEnv {
            model: Some("glm-5-3-flash".to_owned()),
            base_url: Some("https://fp.example.invalid/v1".to_owned()),
            context_length: Some(393_216),
        }
    }

    /// agentd's own environment in a test: a PATH, the FP key, its alias, and
    /// every helper test variable.
    fn agentd_env(openai_key: &str) -> Vec<(OsString, OsString)> {
        let mut environment = vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("FINITE_PRIVATE_API_KEY".into(), FP_KEY.into()),
            ("OPENAI_API_KEY".into(), openai_key.into()),
            ("FINITE_AGENTD_SUPERVISED".into(), "1".into()),
            ("CODEX_HOME".into(), "/root/.codex".into()),
        ];
        for name in HELPER_TEST_VARIABLES {
            environment.push((name.into(), "synthetic-test-override".into()));
        }
        environment
    }

    /// A fake helper: records its argv and environment, then prints `reply`.
    fn fake_helper(dir: &Path, body: &str) -> HelperCommand {
        // A fresh file each time: rewriting a script that just ran can fail
        // with ETXTBSY on Linux.
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let script = dir.join(format!("fake-helper-{count}"));
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        HelperCommand {
            python: script.into(),
            module: "fake_helper_module".to_owned(),
        }
    }

    fn recorded_env(path: &Path) -> BTreeMap<String, String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    #[tokio::test]
    async fn t_a13_helper_env_is_the_launch_env_without_test_variables() {
        let temp = tempfile::tempdir().unwrap();
        let dump = temp.path().join("env");
        let args = temp.path().join("args");
        let command = fake_helper(
            temp.path(),
            &format!(
                "env > '{}'\necho \"$@\" > '{}'\necho '{{\"cleared\": 2}}'",
                dump.display(),
                args.display()
            ),
        );
        let hermes_home = temp.path().join("hermes-home");

        let value = run_helper(
            &command,
            helper_env(agentd_env(FP_KEY), &hermes_home, &fp()),
            &["clear-session-overrides", "--provider", "openrouter"],
            CLEAR_DEADLINE,
        )
        .await
        .unwrap();
        assert_eq!(parse_cleared_count(&value).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(&args).unwrap().trim(),
            "-m fake_helper_module clear-session-overrides --provider openrouter"
        );
        let seen = recorded_env(&dump);
        for name in HELPER_TEST_VARIABLES {
            assert!(!seen.contains_key(name), "{name} reached the helper");
        }
        assert!(!seen.contains_key("OPENAI_API_KEY"));
        let expected = BTreeMap::from([
            ("PATH", "/usr/bin:/bin".to_owned()),
            ("FINITE_PRIVATE_API_KEY", FP_KEY.to_owned()),
            ("FINITE_AGENTD_SUPERVISED", "1".to_owned()),
            ("CODEX_HOME", CODEX_HOME_DISABLED.to_owned()),
            ("HERMES_HOME", hermes_home.display().to_string()),
            ("FINITE_CONFIG_FP_MODEL", "glm-5-3-flash".to_owned()),
            (
                "FINITE_CONFIG_FP_BASE_URL",
                "https://fp.example.invalid/v1".to_owned(),
            ),
            ("FINITE_CONFIG_FP_CONTEXT_LENGTH", "393216".to_owned()),
        ]);
        // `sh` may add its own PWD/SHLVL/_; everything else is exactly ours.
        let ours = seen
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "PWD" | "SHLVL" | "_" | "OLDPWD"))
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ours, expected);
    }

    #[test]
    fn a_users_own_openai_key_reaches_the_helper() {
        let env = helper_env(agentd_env("sk-user-own-key"), Path::new("/h"), &fp());
        assert_eq!(
            env.get(&OsString::from("OPENAI_API_KEY")),
            Some(&OsString::from("sk-user-own-key"))
        );
        // An empty FP key never matches an empty alias.
        let mut base = agentd_env("");
        base.retain(|(name, _)| name != "FINITE_PRIVATE_API_KEY");
        base.push(("FINITE_PRIVATE_API_KEY".into(), "".into()));
        let env = helper_env(base, Path::new("/h"), &fp());
        assert_eq!(
            env.get(&OsString::from("OPENAI_API_KEY")),
            Some(&OsString::new())
        );
        let unset = helper_env(Vec::new(), Path::new("/h"), &FinitePrivateEnv::default());
        assert_eq!(
            unset.get(&OsString::from("FINITE_CONFIG_FP_MODEL")),
            Some(&OsString::new())
        );
    }

    #[tokio::test]
    async fn helper_output_is_the_last_json_line() {
        let temp = tempfile::tempdir().unwrap();
        let command = fake_helper(
            temp.path(),
            "echo 'import noise'\necho '{\"cleared\": \"yes\"}'\necho",
        );
        let value = run_helper(&command, BTreeMap::new(), &["clear-auth"], CLEAR_DEADLINE)
            .await
            .unwrap();
        assert!(parse_cleared_auth(&value).unwrap());
        assert_eq!(HelperProvider::OpenaiCodex.arg(), "openai-codex");
        assert_eq!(HelperProvider::Openrouter.arg(), "openrouter");
    }

    #[tokio::test]
    async fn helper_failures_are_unavailable_and_never_echo_output() {
        let temp = tempfile::tempdir().unwrap();
        for body in [
            "echo '{\"cleared\": \"yes\"}'\nexit 2",
            "echo 'sk-or-v1-synthetic-secret'",
            "exit 0",
            "echo '{\"cleared\": \"maybe\"}'",
        ] {
            let command = fake_helper(temp.path(), body);
            let error = match run_helper(&command, BTreeMap::new(), &["clear-auth"], CLEAR_DEADLINE)
                .await
            {
                Ok(value) => parse_cleared_auth(&value).unwrap_err(),
                Err(error) => error,
            };
            assert_eq!(error.public_code(), "provider_unavailable");
            assert!(!error.public_message().contains("sk-or"));
        }
        let missing = HelperCommand {
            python: temp.path().join("no-such-python").into(),
            module: HELPER_MODULE.to_owned(),
        };
        assert!(
            run_helper(
                &missing,
                BTreeMap::new(),
                &["inference-facts"],
                FACTS_DEADLINE
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_helper_past_its_deadline_is_killed_with_its_group() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("grandchild.pid");
        let command = fake_helper(
            temp.path(),
            &format!("sleep 60 &\necho $! > '{}'\nwait", pid_file.display()),
        );
        let started = Instant::now();
        let result = run_helper(
            &command,
            BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            &["inference-facts"],
            // Long enough for a loaded machine to start the script.
            Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
        let grandchild = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while rustix::process::test_kill_process(
                rustix::process::Pid::from_raw(grandchild).unwrap(),
            )
            .is_ok()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the helper's process group must be killed");
    }

    #[tokio::test]
    async fn t_a17_helper_failure_serves_unknown_facts() {
        let temp = tempfile::tempdir().unwrap();
        let cache = FactsCache::default();
        let failing = fake_helper(temp.path(), "exit 1");
        let facts = cache
            .get_or_fetch(temp.path(), || async {
                let value = run_helper(
                    &failing,
                    BTreeMap::new(),
                    &["inference-facts"],
                    FACTS_DEADLINE,
                )
                .await?;
                parse_facts(value)
            })
            .await;
        assert_eq!(facts, InferenceFacts::unknown());
        // Status still derives every field, with unknown, never a guess.
        let status = crate::inference::derive_status(&Value::Null, None, &fp(), &facts);
        assert_eq!(
            serde_json::to_value(&status.fallback).unwrap()["state"],
            "unknown"
        );
        assert_eq!(
            serde_json::to_value(&status.routes.finite_private).unwrap()["state"],
            "unknown"
        );

        // A well-formed reply parses into facts.
        let reply = serde_json::json!({
            "v": 1, "saved_route": "finite_private",
            "fallback": {"fallback_providers": "absent", "fallback_model": "absent", "effective": []},
            "finite_private": {"provider_entry": "canonical", "fp_key": "present"},
            "openrouter": {"hermes_key": "absent", "hermes_key_fingerprint": null,
                           "dotenv_key": "absent", "manual_pool_entries": "none"},
            "codex": {"state": "not_signed_in", "quota_reset_at": null, "reported_quota_reset_at": null},
            "session_overrides": {"openrouter": "absent", "openai_codex": "unknown"},
            "alias_present": "no", "codex_home_neutral": "yes"
        });
        let working = fake_helper(temp.path(), &format!("echo '{reply}'"));
        let value = run_helper(
            &working,
            BTreeMap::new(),
            &["inference-facts"],
            FACTS_DEADLINE,
        )
        .await
        .unwrap();
        let facts = parse_facts(value).unwrap();
        assert_eq!(facts.finite_private.fp_key, Tri::Present);
        assert_eq!(facts.session_overrides.openai_codex, Tri::Unknown);
    }
}
