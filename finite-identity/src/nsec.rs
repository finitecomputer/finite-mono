//! NIP-19 `nsec` encoding for secret keys (plain bech32, HRP `nsec`,
//! 32-byte payload), mirroring the [`npub`](crate::npub) module.
//!
//! These functions handle secret material. Never log, display, or
//! `Debug`-print an `nsec` string or the bytes it decodes to; the decode
//! error strings deliberately never echo the input.

use bech32::{Bech32, Hrp};

const NSEC_HRP: &str = "nsec";

/// Encode a 32-byte secret key as an `nsec1...` string.
///
/// The returned string is secret material; treat it exactly like the raw
/// bytes (never log it, never store it outside the identity file).
pub fn encode(secret: &[u8; 32]) -> String {
    let hrp = Hrp::parse(NSEC_HRP).expect("static hrp is valid");
    bech32::encode::<Bech32>(hrp, secret).expect("32-byte payload is within bech32 limits")
}

/// Decode an `nsec1...` string back to the 32-byte secret key.
///
/// Error strings never include the input (a mistyped secret must not leak
/// into logs via the error message).
pub fn decode(nsec: &str) -> Result<[u8; 32], String> {
    let (hrp, bytes) = bech32::decode(nsec).map_err(|_| "not valid bech32".to_owned())?;
    if hrp.as_str() != NSEC_HRP {
        return Err(format!("expected hrp {NSEC_HRP:?}, got {:?}", hrp.as_str()));
    }
    bytes
        .try_into()
        .map_err(|_| "nsec must decode to 32 bytes".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Canonical NIP-19 test vector for nsec (from the NIP-19 spec).
    const HEX: &str = "67dea2ed018072d675f5415ecfaed7d2597555e202d85b3d65ea4e58d2d92ffa";
    const NSEC: &str = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";

    fn hex32(hex: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn matches_nip19_test_vector() {
        assert_eq!(encode(&hex32(HEX)), NSEC);
        assert_eq!(decode(NSEC).unwrap(), hex32(HEX));
    }

    #[test]
    fn rejects_wrong_hrp_length_and_garbage() {
        // npub (a public key) must never be accepted as a secret.
        assert!(
            decode("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6")
                .unwrap_err()
                .contains("hrp")
        );
        assert!(decode("not bech32").is_err());
        let short = bech32::encode::<Bech32>(Hrp::parse("nsec").unwrap(), &[0u8; 20]).unwrap();
        assert_eq!(decode(&short).unwrap_err(), "nsec must decode to 32 bytes");
    }

    // The decode error path must never echo the input string.
    #[test]
    fn decode_errors_never_echo_input() {
        let almost = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe4";
        let error = decode(almost).unwrap_err();
        assert!(
            !error.contains("vl029"),
            "error must not echo input: {error}"
        );
    }
}
