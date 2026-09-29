use super::*;

#[test]
fn legacy_and_drained_inboxes_need_no_new_reader() {
    for raw in [
        r#"{}"#,
        r#"{"events":[]}"#,
        r#"{"events":[{"lease":{"state":"pending"}},{"lease":{"state":"leased"}},{"text":"old"}]}"#,
    ] {
        assert!(check(raw.as_bytes(), None).is_ok());
    }
}

#[test]
fn protected_unknown_and_unreadable_inboxes_fail_closed() {
    let pending = br#"{"events":[{"lease":{"state":"refusal_v1"}}]}"#;
    assert!(check(pending, None).is_err());
    assert!(check(pending, Some(REFUSAL_V1_READER)).is_ok());
    for raw in [
        r#"{"events":[{"lease":{"state":"refusal_v2"}}]}"#,
        "invalid",
        r#"{"events":null}"#,
    ] {
        assert!(check(raw.as_bytes(), Some(REFUSAL_V1_READER)).is_err());
    }
}
