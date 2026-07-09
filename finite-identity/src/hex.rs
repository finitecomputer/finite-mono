//! Lowercase hex helpers for Finite Identity public wire values.

/// Encode bytes as lowercase hex.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble < 16"));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble < 16"));
    }
    out
}

/// Decode a hex string into bytes.
pub fn decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        out.push(u8::from_str_radix(&hex[index..index + 2], 16).ok()?);
    }
    Some(out)
}

/// Decode a 32-byte lowercase or uppercase hex string.
pub fn decode32(hex: &str) -> Option<[u8; 32]> {
    let bytes = decode(hex)?;
    bytes.try_into().ok()
}

/// Decode a 64-byte lowercase or uppercase hex string.
pub fn decode64(hex: &str) -> Option<[u8; 64]> {
    let bytes = decode(hex)?;
    bytes.try_into().ok()
}

/// True when the string is exactly 32 bytes encoded as hex.
pub fn is_hex32(hex: &str) -> bool {
    decode32(hex).is_some()
}
