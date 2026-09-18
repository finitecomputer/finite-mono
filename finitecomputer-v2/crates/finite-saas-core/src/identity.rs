//! Canonical identities, opaque IDs, input normalization, and timestamps.

use crate::{CoreError, CoreResult};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn normalize_owner_email(value: Option<&str>) -> Option<String> {
    let email = value?.trim().to_lowercase();
    if email.is_empty() { None } else { Some(email) }
}

/// The natural key for a runtime: `source_host_id:source_machine_id`
/// (UNIQUE on `agent_runtimes.source_import_key`). The name is an artifact of
/// the deleted existing-host import bridge, but the key itself is live
/// identity machinery — every registration resolves runtimes through it.
/// Renaming the column would be schema surgery for zero behavior change, so
/// the legacy name stays.
pub fn source_import_key(source_host_id: &str, source_machine_id: &str) -> String {
    format!(
        "{}:{}",
        normalize_id_part(source_host_id),
        normalize_id_part(source_machine_id)
    )
}

pub fn normalize_source_host_id(value: &str) -> CoreResult<String> {
    let source_host_id = value.trim().to_lowercase();
    if source_host_id.is_empty() {
        return Err(CoreError::MissingSourceHostId);
    }
    if !source_host_id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(CoreError::InvalidSourceHostId);
    }
    if source_host_id.starts_with('-') || source_host_id.ends_with('-') {
        return Err(CoreError::InvalidSourceHostId);
    }
    Ok(source_host_id)
}

pub(crate) fn valid_agent_npub(value: &str) -> bool {
    value.starts_with("npub1")
        && value.len() <= 256
        && value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

pub(crate) fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn trim_or_fallback(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn trim_to_option(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn normalize_idempotency_key(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(128).collect())
    }
}

pub(crate) fn normalize_profile_picture_url(value: Option<&str>) -> CoreResult<Option<String>> {
    let Some(value) = trim_to_option(value) else {
        return Ok(None);
    };
    let valid_scheme = value.starts_with("https://") || value.starts_with("http://");
    if !valid_scheme
        || value.len() > 2_048
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(CoreError::InvalidAgentProfilePictureUrl);
    }
    Ok(Some(value))
}

/// The dashboard submits the hosted-device `identity.account_id`, which is the
/// account's 64-hex public key; the Hermes adapter's `user_id` and the chat
/// sidecar Welcome allowlist both consume that same hex form, so Core stores
/// the canonical lowercase hex and accepts nothing else.
pub(crate) fn normalize_owner_chat_account_id(value: Option<&str>) -> CoreResult<Option<String>> {
    let Some(value) = trim_to_option(value) else {
        return Ok(None);
    };
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 64
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CoreError::InvalidOwnerChatAccountId);
    }
    Ok(Some(normalized))
}

pub(crate) fn current_time_iso() -> CoreResult<String> {
    // Truncate to microseconds: TIMESTAMPTZ stores exactly six fractional
    // digits, so a nanosecond-precision stamp would round on write and stop
    // round-tripping byte-for-byte (macOS clocks tick in microseconds, which
    // hid this; Linux exposes it).
    let now = OffsetDateTime::now_utc();
    let now = now
        .replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .expect("truncating nanoseconds cannot leave the valid range");
    Ok(now.format(&Rfc3339)?)
}

pub(crate) fn parse_time(value: &str) -> CoreResult<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| CoreError::InvalidTimestamp)
}

pub(crate) fn normalize_id_part(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(crate) fn id_from_parts(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hasher.update([0]);
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(10)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}_{hex}")
}

/// Generate an opaque surrogate id: `<prefix>_<20 hex chars of CSPRNG>`.
///
/// Surrogate ids are minted at insert time and are the ONLY way we assign a
/// primary key for a root entity (user, org, agent-creation request, project,
/// runtime). They are NEVER derived from PII or request inputs — that coupling
/// (`user_id = f(email)`) is exactly what let a wiped+recreated same-email
/// account collide with orphans (PERSISTENCE.md anti-pattern #5). Randomness
/// comes from `getrandom` (the OS CSPRNG), the same source the API-key
/// generator uses; this is the server crate, so the workflow-script
/// Math.random/Date.now constraints do not apply.
pub(crate) fn generate_surrogate_id(prefix: &str) -> CoreResult<String> {
    let mut bytes = [0_u8; 10];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        CoreError::Store(format!("failed to generate {prefix} surrogate id: {error}"))
    })?;
    let mut id = String::with_capacity(prefix.len() + 1 + bytes.len() * 2);
    id.push_str(prefix);
    id.push('_');
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02x}").map_err(|error| {
            CoreError::Store(format!("failed to render {prefix} surrogate id: {error}"))
        })?;
    }
    Ok(id)
}

pub(crate) fn new_user_id() -> CoreResult<String> {
    generate_surrogate_id("user")
}

pub(crate) fn new_customer_org_id() -> CoreResult<String> {
    generate_surrogate_id("org")
}

pub(crate) fn new_agent_runtime_id() -> CoreResult<String> {
    generate_surrogate_id("runtime")
}

pub(crate) fn agent_creation_entitlement_id_for(customer_org_id: &str) -> String {
    id_from_parts("agent_entitlement", &[customer_org_id])
}

pub(crate) fn new_agent_creation_request_id() -> CoreResult<String> {
    generate_surrogate_id("agent_request")
}

pub(crate) fn new_self_service_project_id() -> CoreResult<String> {
    generate_surrogate_id("project")
}

/// Mint the stable, collision-resistant Finite VIP address shown to humans.
/// The readable prefix comes from the chosen agent name; the opaque project
/// suffix keeps duplicate names from competing for the same global identity.
pub fn canonical_agent_email(display_name: &str, project_id: &str) -> String {
    let mut slug = String::with_capacity(display_name.len());
    let mut previous_was_separator = false;
    for character in display_name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !slug.is_empty() && !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("agent");
    }
    slug.truncate(40);
    while slug.ends_with('-') {
        slug.pop();
    }

    let mut suffix = project_id
        .strip_prefix("project_")
        .unwrap_or(project_id)
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase();
    if suffix.len() < 16 {
        suffix = Sha256::digest(project_id.as_bytes())
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
    }
    format!("{slug}-{suffix}@finite.vip")
}

pub(crate) fn project_runtime_link_id_for(project_id: &str, agent_runtime_id: &str) -> String {
    id_from_parts("runtime_link", &[project_id, agent_runtime_id])
}

pub(crate) fn chat_identity_id_for_user(user_id: &str) -> String {
    id_from_parts("chat_identity", &[user_id, "hosted_web"])
}

pub(crate) fn project_room_membership_id_for(project_id: &str, chat_identity_id: &str) -> String {
    id_from_parts("room_member", &[project_id, chat_identity_id])
}
