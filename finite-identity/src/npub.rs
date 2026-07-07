//! NIP-19 `npub` encoding. Tools display identity as `npub` but store hex;
//! this matches the encoders in `finitesites-proto` and `finitechat-proto`
//! byte for byte (plain bech32, HRP `npub`, 32-byte payload).

use bech32::{Bech32, Hrp};

const NPUB_HRP: &str = "npub";

/// Encode a 32-byte x-only public key as an `npub1...` string.
pub fn encode(public_key: &[u8; 32]) -> String {
    let hrp = Hrp::parse(NPUB_HRP).expect("static hrp is valid");
    bech32::encode::<Bech32>(hrp, public_key).expect("32-byte payload is within bech32 limits")
}

/// Decode an `npub1...` string back to the 32-byte x-only public key.
pub fn decode(npub: &str) -> Result<[u8; 32], String> {
    let (hrp, bytes) = bech32::decode(npub).map_err(|error| error.to_string())?;
    if hrp.as_str() != NPUB_HRP {
        return Err(format!("expected hrp {NPUB_HRP:?}, got {:?}", hrp.as_str()));
    }
    bytes
        .try_into()
        .map_err(|_| "npub must decode to 32 bytes".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Canonical NIP-19 test vector (fiatjaf's pubkey), cross-checked against
    // the fixtures in finitesites-proto::npub.
    const HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    const NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    fn hex32(hex: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn matches_nip19_test_vector() {
        assert_eq!(encode(&hex32(HEX)), NPUB);
        assert_eq!(decode(NPUB).unwrap(), hex32(HEX));
    }

    #[test]
    fn rejects_wrong_hrp_and_length() {
        assert!(decode("nsec1qqqq").is_err());
        assert!(decode("not bech32").is_err());
        let short = bech32::encode::<Bech32>(Hrp::parse("npub").unwrap(), &[0u8; 20]).unwrap();
        assert!(decode(&short).is_err());
    }
}
