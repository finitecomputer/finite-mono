//! Page cursors bound to the authority they were issued under:
//! `<authorityFingerprint>.<npub of the last row>`. A cursor from a different
//! authority is refused rather than stitched onto newer facts.

use finite_brain_core::UserId;
use finite_nostr::NostrPublicKey;

const FINGERPRINT_HEX_LEN: usize = 64;

pub(crate) fn encode(fingerprint: &str, npub: &str) -> String {
    format!("{fingerprint}.{npub}")
}

/// Parse a cursor into its authority fingerprint and canonical npub.
pub(crate) fn decode(cursor: &str) -> Option<(String, UserId)> {
    let (fingerprint, npub) = cursor.split_once('.')?;
    let fingerprint_ok = fingerprint.len() == FINGERPRINT_HEX_LEN
        && fingerprint
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !fingerprint_ok {
        return None;
    }
    let canonical = NostrPublicKey::parse(npub)
        .and_then(|key| key.to_npub())
        .ok()?;
    if canonical != npub {
        return None;
    }
    Some((fingerprint.to_owned(), UserId::new(canonical).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_and_reject_foreign_shapes() {
        let npub = NostrPublicKey::from_protocol(nostr::Keys::generate().public_key())
            .to_npub()
            .unwrap();
        let fingerprint = "a".repeat(64);
        let (decoded_fingerprint, decoded_npub) = decode(&encode(&fingerprint, &npub)).unwrap();
        assert_eq!(decoded_fingerprint, fingerprint);
        assert_eq!(decoded_npub.as_str(), npub);
        for bad in [
            npub.clone(),
            format!("{}.{npub}", "A".repeat(64)),
            format!("{}.{npub}", "a".repeat(63)),
            format!("{fingerprint}.not-an-npub"),
            format!("{fingerprint}.{}", npub.to_uppercase()),
            format!("{fingerprint}."),
        ] {
            assert!(decode(&bad).is_none(), "{bad}");
        }
    }
}
