//! `fbrain brain delete` offers no bypass of its preview, and only the Brain
//! server's deleted-Brain answer makes a Working Tree terminal.

use super::*;

#[test]
fn brain_delete_has_no_flag_that_skips_the_preview() {
    let tmp = TempDir::new().unwrap();
    for flag in ["--force", "--yes", "-y"] {
        let mut output = Vec::new();
        let error = run_with_env(
            [
                "brain",
                "delete",
                "acme",
                flag,
                "--server",
                "http://127.0.0.1:9",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(&format!("has no {flag}")),
            "{error}"
        );
        assert!(output.is_empty());
    }
    let mut output = Vec::new();
    let error = run_with_env(["brain", "delete"], env_for(&tmp), &mut output).unwrap_err();
    assert!(
        matches!(error, CliError::MissingArgument("brain-id")),
        "{error}"
    );
}

#[test]
fn only_the_deleted_brain_answer_is_terminal() {
    let deleted = CliError::SyncStage {
        stage: "fetch".to_owned(),
        root: PathBuf::from("/tree"),
        source: Box::new(CliError::HttpStatus {
            status: 410,
            body: serde_json::json!({
                "error": "Brain acme was permanently deleted at 2026-10-25T00:00:00Z",
                "code": "brain_deleted",
                "brainDeleted": {"brainId": "acme", "deletedAt": "2026-10-25T00:00:00Z"}
            })
            .to_string(),
        }),
    };
    assert_eq!(
        brain_deleted_notice(&deleted),
        Some(("acme".to_owned(), "2026-10-25T00:00:00Z".to_owned()))
    );
    for other in [
        // An expired sync cursor is also 410, but asks for a rebootstrap.
        CliError::HttpStatus {
            status: 410,
            body: r#"{"error":"rebootstrap required: retention floor 7"}"#.to_owned(),
        },
        // An unknown Brain is never mistaken for a deleted one.
        CliError::HttpStatus {
            status: 404,
            body: r#"{"error":"missing brain: acme"}"#.to_owned(),
        },
        CliError::Http("connection refused".to_owned()),
    ] {
        assert_eq!(brain_deleted_notice(&other), None, "{other}");
    }
}

#[test]
fn the_terminal_error_names_kept_unsynced_work_without_listing_everything() {
    let state = BrainDeletedState {
        deleted_at: "2026-10-25T00:00:00Z".to_owned(),
        detected_at: "2026-10-25T00:01:00Z".to_owned(),
        unsynced_paths: (0..25)
            .map(|index| format!("Notes/{index:02}.md"))
            .collect(),
    };
    let message = brain_deleted_error("acme", &state).to_string();
    assert!(
        message.starts_with(
            "Brain acme was permanently deleted on the server at 2026-10-25T00:00:00Z"
        )
    );
    assert!(
        message.contains("25 unsynced local change(s) were kept here and not uploaded anywhere")
    );
    assert!(message.contains("Notes/19.md, ..."));
    assert!(!message.contains("Notes/20.md"));
}
