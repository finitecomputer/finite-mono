//! The `finitechat` binary's stderr contract for machine callers: with
//! `--json` anywhere on the command line a failure is one JSON line carrying
//! the same `error_kind` / `retryable` fields as the resident service's HTTP
//! error body; without it the human-readable message is unchanged.

use serde_json::Value;
use std::process::Command;

fn finitechat(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .args(args)
        .output()
        .expect("finitechat binary runs")
}

#[test]
fn json_flag_turns_cli_errors_into_structured_stderr() {
    let output = finitechat(&["hermes", "--json", "no-such-action"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "usage errors keep exit code 2"
    );
    assert!(output.stdout.is_empty(), "errors never go to stdout");

    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    let lines = stderr.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "exactly one stderr line: {stderr:?}");
    let body: Value = serde_json::from_str(lines[0]).expect("stderr line is JSON");

    assert_eq!(body["ok"], Value::Bool(false));
    assert_eq!(body["status"], "error");
    assert_eq!(body["error_kind"], "usage");
    assert_eq!(body["retryable"], Value::Bool(false));
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|text| text.contains("no-such-action")),
        "error text names the bad argument: {body}"
    );
}

#[test]
fn without_json_flag_cli_errors_stay_human_readable() {
    let output = finitechat(&["hermes", "no-such-action"]);
    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    assert!(stderr.contains("no-such-action"), "{stderr:?}");
    assert!(
        serde_json::from_str::<Value>(stderr.trim()).is_err(),
        "plain mode must not emit JSON: {stderr:?}"
    );
}

#[test]
fn json_errors_requested_matches_only_the_exact_flag() {
    assert!(finitechat_cli::json_errors_requested([
        "hermes",
        "--agent-home",
        "/h",
        "send",
        "--json"
    ]));
    assert!(!finitechat_cli::json_errors_requested([
        "hermes",
        "send",
        "--request-json",
        "{}"
    ]));
    assert!(!finitechat_cli::json_errors_requested::<[&str; 0], &str>([]));
}
