use super::*;

/// Frozen minimal shape of a protected inbox holding a prepared wrapper v2.
/// A zero watermark is valid: the room had no accepted own send yet.
const PREPARED_V2: &str = include_str!("fixtures/prepared-refusal-v2.json");
const OLD_CAPABLE_READER: &str = "refusal-v1";

fn prepared(fields: &str) -> String {
    format!(
        r#"{{"events":[{{"lease":{{"state":"refusal_v1","operation":{{"prepared":{fields}}}}}}}]}}"#
    )
}

#[test]
fn legacy_and_drained_inboxes_need_no_new_reader() {
    for raw in [
        r#"{}"#,
        r#"{"events":[]}"#,
        r#"{"events":[{"lease":{"state":"pending"}},{"lease":{"state":"leased"}},{"text":"old"}]}"#,
    ] {
        for reader in [None, Some(OLD_CAPABLE_READER), Some(REFUSAL_V2_READER)] {
            assert!(check(raw.as_bytes(), reader).is_ok(), "{raw} {reader:?}");
        }
    }
}

#[test]
fn frozen_prepared_v2_inbox_needs_the_v2_reader() {
    assert!(check(PREPARED_V2.as_bytes(), Some(REFUSAL_V2_READER)).is_ok());
    for reader in [None, Some(OLD_CAPABLE_READER), Some("refusal-v3")] {
        assert!(check(PREPARED_V2.as_bytes(), reader).is_err(), "{reader:?}");
    }
}

#[test]
fn unprepared_protected_leases_need_the_v2_reader() {
    for raw in [
        r#"{"events":[{"lease":{"state":"refusal_v1"}}]}"#.to_owned(),
        prepared("null"),
    ] {
        assert!(
            check(raw.as_bytes(), Some(REFUSAL_V2_READER)).is_ok(),
            "{raw}"
        );
        assert!(
            check(raw.as_bytes(), Some(OLD_CAPABLE_READER)).is_err(),
            "{raw}"
        );
        assert!(check(raw.as_bytes(), None).is_err(), "{raw}");
    }
}

#[test]
fn prepared_wrappers_other_than_v2_with_a_u64_watermark_fail_closed() {
    assert!(
        check(
            prepared(r#"{"version":2,"own_send_high_water_seq":18446744073709551615}"#).as_bytes(),
            Some(REFUSAL_V2_READER)
        )
        .is_ok()
    );
    for fields in [
        r#"{"version":1,"own_send_high_water_seq":4}"#,
        r#"{"version":1}"#,
        r#"{"version":3,"own_send_high_water_seq":4}"#,
        r#"{"own_send_high_water_seq":4}"#,
        r#"{"version":2}"#,
        r#"{"version":2,"own_send_high_water_seq":null}"#,
        r#"{"version":2,"own_send_high_water_seq":-1}"#,
        r#"{"version":2,"own_send_high_water_seq":1.5}"#,
        r#"{"version":2,"own_send_high_water_seq":"4"}"#,
        r#"{"version":2,"own_send_high_water_seq":18446744073709551616}"#,
    ] {
        let error = check(prepared(fields).as_bytes(), Some(REFUSAL_V2_READER)).unwrap_err();
        assert!(
            error.contains("runtime replacement blocked"),
            "{fields}: {error}"
        );
    }
}

#[test]
fn unknown_and_unreadable_inboxes_fail_closed() {
    for raw in [
        r#"{"events":[{"lease":{"state":"refusal_v2"}}]}"#,
        "invalid",
        r#"{"events":null}"#,
    ] {
        assert!(
            check(raw.as_bytes(), Some(REFUSAL_V2_READER)).is_err(),
            "{raw}"
        );
    }
}
