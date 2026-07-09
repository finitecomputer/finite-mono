pub const NOSTR_NPUB_HRP: &str = "npub";
pub const NOSTR_NPROFILE_HRP: &str = "nprofile";
pub const NOSTR_NSEC_HRP: &str = "nsec";

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex value must have even length".to_owned());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "invalid hex".to_owned())
        })
        .collect()
}

/// NIP-19 npub display form of a 64-hex account id.
pub fn npub_encode(account_id_hex: &str) -> Result<String, String> {
    let bytes = decode_hex(account_id_hex)?;
    if bytes.len() != 32 {
        return Err("account id must be 32 bytes of hex".to_owned());
    }
    let hrp = bech32::Hrp::parse(NOSTR_NPUB_HRP).expect("static hrp");
    bech32::encode::<bech32::Bech32>(hrp, &bytes).map_err(|error| error.to_string())
}

/// Decode an npub back to the 64-hex account id.
pub fn npub_decode(npub: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(npub).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NPUB_HRP {
        return Err(format!("expected {NOSTR_NPUB_HRP}, got {hrp}"));
    }
    if bytes.len() != 32 {
        return Err("npub must decode to 32 bytes".to_owned());
    }
    Ok(hex_lower(&bytes))
}

/// Decode a NIP-19 nprofile back to the embedded 64-hex account id.
pub fn nprofile_decode(nprofile: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(nprofile).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NPROFILE_HRP {
        return Err(format!("expected {NOSTR_NPROFILE_HRP}, got {hrp}"));
    }

    let mut offset = 0;
    while offset < bytes.len() {
        if offset + 2 > bytes.len() {
            return Err("nprofile contains truncated TLV header".to_owned());
        }
        let tag = bytes[offset];
        let length = bytes[offset + 1] as usize;
        offset += 2;
        if offset + length > bytes.len() {
            return Err("nprofile contains truncated TLV value".to_owned());
        }
        let value = &bytes[offset..offset + length];
        offset += length;
        if tag == 0 {
            if value.len() != 32 {
                return Err("nprofile pubkey TLV must be 32 bytes".to_owned());
            }
            return Ok(hex_lower(value));
        }
    }

    Err("nprofile is missing pubkey TLV".to_owned())
}

/// NIP-19 nsec display form of a 32-byte account secret.
pub fn nsec_encode(secret_hex: &str) -> Result<String, String> {
    let bytes = decode_hex(secret_hex)?;
    if bytes.len() != 32 {
        return Err("account secret must be 32 bytes of hex".to_owned());
    }
    let hrp = bech32::Hrp::parse(NOSTR_NSEC_HRP).expect("static hrp");
    bech32::encode::<bech32::Bech32>(hrp, &bytes).map_err(|error| error.to_string())
}

/// Decode an nsec back to the 32-byte account secret hex.
pub fn nsec_decode(nsec: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(nsec).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NSEC_HRP {
        return Err(format!("expected {NOSTR_NSEC_HRP}, got {hrp}"));
    }
    if bytes.len() != 32 {
        return Err("nsec must decode to 32 bytes".to_owned());
    }
    Ok(hex_lower(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npub_and_nsec_round_trip_hex_material() {
        let account_id = hex_lower(&[0xab; 32]);
        let npub = npub_encode(&account_id).unwrap();
        assert!(npub.starts_with("npub1"));
        assert_eq!(npub_decode(&npub).unwrap(), account_id);

        let secret = hex_lower(&[0xcd; 32]);
        let nsec = nsec_encode(&secret).unwrap();
        assert!(nsec.starts_with("nsec1"));
        assert_eq!(nsec_decode(&nsec).unwrap(), secret);
    }
}
