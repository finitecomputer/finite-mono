use super::*;
use serde_json::json;

const HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

fn name(first: Value, last: Value) -> Option<String> {
    workos_display_name(Some(&first), Some(&last))
}

#[test]
fn workos_names_join_present_parts_and_never_guess() {
    assert_eq!(
        name(json!(" Ada "), json!("Lovelace")).as_deref(),
        Some("Ada Lovelace")
    );
    assert_eq!(
        name(json!("Mary  Ann"), json!(null)).as_deref(),
        Some("Mary Ann")
    );
    assert_eq!(name(json!(""), json!("Hopper")).as_deref(), Some("Hopper"));
    assert_eq!(workos_display_name(None, None), None);
    assert_eq!(name(json!("  "), json!(null)), None);
    assert_eq!(
        name(json!("名前"), json!("田中")).as_deref(),
        Some("名前 田中")
    );
}

#[test]
fn malformed_or_oversized_workos_names_are_unusable_not_fatal() {
    for (first, last) in [
        (json!(42), json!("Lovelace")),
        (json!(["Ada"]), json!(null)),
        (json!("Ada"), json!({"x": 1})),
        (json!("Ada\u{0000}"), json!(null)),
        (json!("Ada\nLovelace"), json!(null)),
        (json!("Ada\u{202E}ecalevol"), json!(null)),
        (json!("A".repeat(MAX_NAME_PART_CHARS + 1)), json!(null)),
        // Two in-bound parts whose UTF-8 total exceeds the 200-byte wire cap.
        (json!("é".repeat(60)), json!("é".repeat(60))),
    ] {
        assert_eq!(name(first.clone(), last.clone()), None, "{first} {last}");
    }
    assert!(name(json!("A".repeat(MAX_NAME_PART_CHARS)), json!(null)).is_some());
}

#[test]
fn aliases_are_trimmed_bounded_and_printable() {
    assert_eq!(
        normalize_alias("  Maya   (design) ").as_deref(),
        Some("Maya (design)")
    );
    assert_eq!(
        normalize_alias(&"x".repeat(MAX_ALIAS_CHARS))
            .as_deref()
            .map(str::len),
        Some(MAX_ALIAS_CHARS)
    );
    for rejected in [
        String::new(),
        "   ".to_string(),
        "x".repeat(MAX_ALIAS_CHARS + 1),
        "tab\there".to_string(),
        "bell\u{7}".to_string(),
        "\u{2066}hidden".to_string(),
    ] {
        assert_eq!(normalize_alias(&rejected), None, "{rejected:?}");
    }
    // Multi-byte characters count as characters, within the byte cap.
    assert!(normalize_alias(&"👩🏽‍💻".repeat(9)).is_some());
}

#[test]
fn public_keys_canonicalize_from_npub_or_hex_and_must_be_curve_points() {
    assert_eq!(canonical_public_key(NPUB).as_deref(), Some(HEX));
    assert_eq!(canonical_public_key(HEX).as_deref(), Some(HEX));
    assert_eq!(
        canonical_public_key(&HEX.to_uppercase()).as_deref(),
        Some(HEX)
    );
    for rejected in [
        String::new(),
        "f".repeat(64),
        HEX[..62].to_string(),
        format!("{HEX}00"),
        NPUB.to_uppercase(),
        NPUB.replacen("npub", "nsec", 1),
        NPUB[..NPUB.len() - 1].to_string(),
        "npub1".to_string(),
    ] {
        assert_eq!(canonical_public_key(&rejected), None, "{rejected}");
    }
}
