//! Classification and canonicalisation of human-typed identity inputs.
//!
//! Every FiniteBrain surface that accepts "a person or an agent" as text
//! (admin targets, invitation targets, Personal Agent mailboxes, departure
//! facts) classifies the text the same way. Which authority answers for each
//! class, and in which order, is the caller's resolution policy; this module
//! only decides what kind of thing was typed.

use std::error::Error;
use std::fmt;

use finite_nostr::NostrPublicKey;

/// Mailbox domain the Finite identity authority serves itself: Managed Agent
/// mailboxes and finite.vip NIP-05 names live here, so no account roster is
/// consulted for them.
pub const FINITE_VIP_EMAIL_DOMAIN: &str = "finite.vip";

const MAX_EMAIL_LEN: usize = 320;

/// Why a string is not an email address FiniteBrain can canonicalise.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum EmailInputError {
    /// No `local@domain` shape.
    NotAnEmail,
    /// Empty local part or domain, control characters, or over 320 bytes.
    NotPrintable,
}

impl fmt::Display for EmailInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnEmail => f.write_str("must be an email address"),
            Self::NotPrintable => f.write_str("must be a printable email address"),
        }
    }
}

impl Error for EmailInputError {}

/// Trimmed, ASCII-lowercased `local@domain` form used wherever FiniteBrain
/// stores or compares an email address.
pub fn canonical_email(value: &str) -> Result<String, EmailInputError> {
    let value = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = value.split_once('@') else {
        return Err(EmailInputError::NotAnEmail);
    };
    if local.is_empty()
        || domain.is_empty()
        || value.len() > MAX_EMAIL_LEN
        || value.chars().any(|c| c == '\0' || c.is_control())
    {
        return Err(EmailInputError::NotPrintable);
    }
    Ok(value)
}

/// Whether the text has the `local@domain.tld` shape of an email address.
/// Unlike [`canonical_email`] this requires a dotted domain, so bare NIP-05
/// root names such as `example.com` and `_@localhost` are not email-like.
pub fn email_like(value: &str) -> bool {
    let value = value.trim();
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

/// Whether the text is a mailbox on [`FINITE_VIP_EMAIL_DOMAIN`].
pub fn finite_vip_email(value: &str) -> bool {
    canonical_email(value)
        .map(|email| {
            email
                .rsplit_once('@')
                .is_some_and(|(_, domain)| domain == FINITE_VIP_EMAIL_DOMAIN)
        })
        .unwrap_or(false)
}

/// A human-typed identity input, classified by the kind of authority that
/// can answer for it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum IdentityInput {
    /// A Nostr public key in npub, nprofile, `nostr:` URI, or hex form.
    Npub(NostrPublicKey),
    /// A mailbox on the identity authority's own domain, in canonical form.
    FiniteVipEmail(String),
    /// Any other email-shaped text, in canonical form; it may bind to a
    /// Finite account whose npub only the account authorities know.
    AccountEmail(String),
    /// Anything else is a NIP-05 identifier (root-name domain shorthand
    /// included), trimmed but otherwise as typed.
    Nip05(String),
}

/// Why a string cannot be classified as an identity input.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum IdentityInputError {
    /// Nothing was typed.
    Empty,
    /// The text looked like an email but cannot be canonicalised.
    Email(EmailInputError),
}

impl fmt::Display for IdentityInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("identity input is required"),
            Self::Email(error) => write!(f, "email target {error}"),
        }
    }
}

impl Error for IdentityInputError {}

impl From<EmailInputError> for IdentityInputError {
    fn from(error: EmailInputError) -> Self {
        Self::Email(error)
    }
}

impl IdentityInput {
    /// Classify one trimmed identity input.
    pub fn parse(value: &str) -> Result<Self, IdentityInputError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(IdentityInputError::Empty);
        }
        if let Ok(public_key) = NostrPublicKey::parse(value) {
            return Ok(Self::Npub(public_key));
        }
        if email_like(value) {
            let email = canonical_email(value)?;
            return Ok(if finite_vip_email(&email) {
                Self::FiniteVipEmail(email)
            } else {
                Self::AccountEmail(email)
            });
        }
        Ok(Self::Nip05(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_email_lowercases_and_bounds() {
        assert_eq!(
            canonical_email("  Alice@Example.COM "),
            Ok("alice@example.com".to_owned())
        );
        assert_eq!(canonical_email("alice"), Err(EmailInputError::NotAnEmail));
        assert_eq!(canonical_email("@x.y"), Err(EmailInputError::NotPrintable));
        assert_eq!(canonical_email("a@"), Err(EmailInputError::NotPrintable));
        assert_eq!(
            canonical_email("a\u{7}@example.com"),
            Err(EmailInputError::NotPrintable)
        );
        let long = format!("{}@example.com", "a".repeat(MAX_EMAIL_LEN));
        assert_eq!(canonical_email(&long), Err(EmailInputError::NotPrintable));
    }

    #[test]
    fn finite_vip_email_matches_only_the_exact_domain() {
        assert!(finite_vip_email("Agent@Finite.VIP"));
        assert!(!finite_vip_email("agent@notfinite.vip"));
        assert!(!finite_vip_email("agent@finite.vip.example.com"));
        assert!(!finite_vip_email("finite.vip"));
    }

    #[test]
    fn email_like_requires_a_dotted_domain() {
        assert!(email_like("alice@example.com"));
        assert!(!email_like("alice@localhost"));
        assert!(!email_like("example.com"));
        assert!(!email_like("@example.com"));
    }

    #[test]
    fn parse_classifies_each_input_kind() {
        let hex = "77".repeat(32);
        assert!(matches!(
            IdentityInput::parse(&hex),
            Ok(IdentityInput::Npub(key)) if key.to_hex() == hex
        ));
        assert_eq!(
            IdentityInput::parse(" Agent@Finite.vip "),
            Ok(IdentityInput::FiniteVipEmail("agent@finite.vip".to_owned()))
        );
        assert_eq!(
            IdentityInput::parse("Alice@Example.com"),
            Ok(IdentityInput::AccountEmail("alice@example.com".to_owned()))
        );
        assert_eq!(
            IdentityInput::parse(" example.com "),
            Ok(IdentityInput::Nip05("example.com".to_owned()))
        );
        assert_eq!(
            IdentityInput::parse("alice@localhost"),
            Ok(IdentityInput::Nip05("alice@localhost".to_owned()))
        );
        assert_eq!(IdentityInput::parse("  "), Err(IdentityInputError::Empty));
        assert_eq!(
            IdentityInput::parse("a\u{7}@example.com"),
            Err(IdentityInputError::Email(EmailInputError::NotPrintable))
        );
    }

    #[test]
    fn error_messages_keep_the_server_wording() {
        assert_eq!(
            IdentityInputError::Empty.to_string(),
            "identity input is required"
        );
        assert_eq!(
            IdentityInputError::Email(EmailInputError::NotPrintable).to_string(),
            "email target must be a printable email address"
        );
    }
}
