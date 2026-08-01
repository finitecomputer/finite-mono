use finitesites_proto::npub;

use crate::CliError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxAddress(String);

impl MailboxAddress {
    pub fn parse(value: &str, flag: &str) -> Result<Self, CliError> {
        let normalized = normalize_address(value)
            .ok_or_else(|| CliError::Usage(format!("{flag} needs a valid mailbox address")))?;
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip05Name(String);

impl Nip05Name {
    pub fn parse(value: &str, flag: &str) -> Result<Self, CliError> {
        let normalized = normalize_address(value)
            .filter(|value| {
                value
                    .split_once('@')
                    .is_some_and(|(localpart, _)| valid_nip05_localpart(localpart))
            })
            .ok_or_else(|| CliError::Usage(format!("{flag} needs a valid NIP-05 name")))?;
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeNpub(String);

impl NativeNpub {
    pub fn parse(value: &str, flag: &str) -> Result<Self, CliError> {
        let pubkey = npub::decode_npub(value.trim())
            .map_err(|error| CliError::Usage(format!("invalid {flag}: {error}")))?;
        let canonical = npub::encode_npub(&pubkey)
            .map_err(|error| CliError::Usage(format!("invalid {flag}: {error}")))?;
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

fn normalize_address(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.is_ascii() {
        return None;
    }
    let normalized = value.to_ascii_lowercase();
    let (localpart, domain) = normalized.split_once('@')?;
    if localpart.is_empty()
        || localpart.len() > 128
        || localpart.starts_with('.')
        || localpart.ends_with('.')
        || localpart.contains("..")
        || !localpart.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'_'
                    | b'%'
                    | b'+'
                    | b'-'
            )
        })
        || domain.is_empty()
        || domain.len() > 253
        || domain.contains('@')
        || !domain.contains('.')
        || !domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        })
    {
        return None;
    }
    Some(normalized)
}

fn valid_nip05_localpart(localpart: &str) -> bool {
    localpart
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    #[test]
    fn typed_targets_normalize_and_reject_wrong_shapes() {
        assert_eq!(
            MailboxAddress::parse(" Paul@Finite.VIP ", "--email")
                .unwrap()
                .as_str(),
            "paul@finite.vip"
        );
        assert_eq!(
            Nip05Name::parse(" Agent.Name@Finite.VIP ", "--nip05")
                .unwrap()
                .as_str(),
            "agent.name@finite.vip"
        );
        assert_eq!(NativeNpub::parse(NPUB, "--npub").unwrap().as_str(), NPUB);
        assert!(MailboxAddress::parse("npub1not-an-email", "--email").is_err());
        assert!(Nip05Name::parse("plus+tag@finite.vip", "--nip05").is_err());
        assert!(NativeNpub::parse("paul@finite.vip", "--npub").is_err());
    }
}
