//! Request validation helpers (blobs, profiles, sync, ids).

use std::collections::{BTreeSet, HashSet};

use axum::http::{HeaderMap, header};
use finitechat_blob::{BLOB_CIPHERTEXT_CONTENT_TYPE, BlobPutRequest};
use finitechat_delivery::{MAX_HTTP_ID_BYTES, MAX_HTTP_SYNC_PAGE_ENTRIES};
use finitechat_http::{
    GetEphemeralActivitiesRequest, NostrProfileRecord, ObserveDeviceLivenessRequest,
    SyncStreamRequest, SyncWaitRequest,
};
use finitechat_proto::{
    AppendEphemeralActivityRequest, AppendEventRequest, LogEntryKind,
    MAX_DEVICE_LIVENESS_EXPIRY_MILLIS, MAX_OBJECT_ID_BYTES, SubmitCommitRequest,
    staged_welcomes_by_id, validate_activity_expiry, validate_bytes_len, validate_bytes_non_empty,
    validate_string_bytes,
};
use finitechat_transport::MemberId;
use sha2::{Digest, Sha256};

use crate::{
    HttpServerConfigurationError, MAX_HTTP_ACCOUNT_ROOM_ID_BYTES,
    MAX_KEY_PACKAGE_AVAILABILITY_BATCH, MAX_NOSTR_PROFILE_ABOUT_BYTES, MAX_NOSTR_PROFILE_BATCH,
    MAX_NOSTR_PROFILE_METADATA_JSON_BYTES, MAX_NOSTR_PROFILE_NAME_BYTES,
    MAX_NOSTR_PROFILE_PICTURE_BYTES, MAX_PUBLIC_IMAGE_BLOB_BYTES, ServerHttpError,
};

pub(crate) fn blob_content_type(headers: &HeaderMap) -> Result<&str, ServerHttpError> {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return Err(ServerHttpError::InvalidBlobRequest {
            reason: "blob upload must include a content-type header".to_owned(),
        });
    };
    let content_type = value
        .to_str()
        .map_err(|_| ServerHttpError::InvalidBlobRequest {
            reason: "blob upload content-type header is not valid UTF-8".to_owned(),
        })?;
    Ok(content_type.split(';').next().unwrap_or_default().trim())
}

pub(crate) fn blob_url(public_url: Option<&str>, headers: &HeaderMap, sha256: &str) -> String {
    if let Some(public_url) = public_url {
        return format!("{public_url}/blobs/{sha256}");
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http" || *value == "https")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("localhost");
    format!("{scheme}://{host}/blobs/{sha256}")
}

pub(crate) fn normalize_public_url(
    public_url: &str,
) -> Result<String, HttpServerConfigurationError> {
    let trimmed = public_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|error| {
        HttpServerConfigurationError::InvalidPublicUrl {
            reason: error.to_string(),
        }
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "scheme must be http or https".to_owned(),
        });
    }
    if parsed.host_str().is_none() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "host is required".to_owned(),
        });
    }
    if parsed.path() != "/" {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "URL must be a bare origin without a path".to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "credentials are not allowed".to_owned(),
        });
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "query and fragment are not allowed".to_owned(),
        });
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

pub(crate) fn normalize_blob_upload_content_type(
    content_type: &str,
) -> Result<&'static str, ServerHttpError> {
    match content_type.trim().to_ascii_lowercase().as_str() {
        BLOB_CIPHERTEXT_CONTENT_TYPE => Ok(BLOB_CIPHERTEXT_CONTENT_TYPE),
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/png" => Ok("image/png"),
        "image/gif" => Ok("image/gif"),
        "image/webp" => Ok("image/webp"),
        other => Err(ServerHttpError::InvalidBlobRequest {
            reason: format!("blob upload content type is not supported: {other}"),
        }),
    }
}

pub(crate) fn validate_blob_upload(
    bytes: &[u8],
    content_type: &str,
) -> Result<(), ServerHttpError> {
    if content_type == BLOB_CIPHERTEXT_CONTENT_TYPE {
        return BlobPutRequest {
            ciphertext: bytes,
            content_type,
        }
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidBlobRequest {
            reason: error.to_string(),
        });
    }
    validate_bytes_non_empty("blob.bytes", bytes.len()).map_err(|error| {
        ServerHttpError::InvalidBlobRequest {
            reason: error.to_string(),
        }
    })?;
    validate_bytes_len(
        "blob.bytes",
        bytes.len(),
        MAX_PUBLIC_IMAGE_BLOB_BYTES as u32,
    )
    .map_err(|error| ServerHttpError::InvalidBlobRequest {
        reason: error.to_string(),
    })?;
    if public_image_blob_magic_matches(bytes, content_type) {
        return Ok(());
    }
    Err(ServerHttpError::InvalidBlobRequest {
        reason: format!("blob bytes do not match {content_type}"),
    })
}

fn public_image_blob_magic_matches(bytes: &[u8], content_type: &str) -> bool {
    match content_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

pub(crate) fn validate_blob_sha256(sha256: &str) -> Result<(), ServerHttpError> {
    if sha256.len() != 64 {
        return Err(ServerHttpError::InvalidBlobRequest {
            reason: format!(
                "blob sha256 must be 64 lowercase hex chars, got {}",
                sha256.len()
            ),
        });
    }
    if sha256
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(());
    }
    Err(ServerHttpError::InvalidBlobRequest {
        reason: "blob sha256 must use lowercase hex".to_owned(),
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn validate_submit_commit_request(
    request: &SubmitCommitRequest,
) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        })?;
    let message_id =
        request
            .envelope
            .message_id()
            .map_err(|error| ServerHttpError::InvalidCommitRequest {
                reason: error.to_string(),
            })?;
    if request.envelope.kind != LogEntryKind::Commit {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: "commit request envelope must be a commit".to_owned(),
        });
    }
    if request.envelope.room_id != request.room_id {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "commit envelope room_id {} does not match request room_id {}",
                request.envelope.room_id, request.room_id
            ),
        });
    }
    if request.envelope.epoch != request.expected_epoch {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "commit envelope epoch {} does not match expected epoch {}",
                request.envelope.epoch, request.expected_epoch
            ),
        });
    }
    if request.envelope.sender != request.sender {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: "commit envelope sender does not match request sender".to_owned(),
        });
    }
    request
        .membership_delta
        .validate_structure(request.expected_epoch, &message_id)
        .map_err(|error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        })?;
    staged_welcomes_by_id(&request.membership_delta, &request.staged_welcomes).map_err(
        |error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        },
    )?;
    Ok(())
}

pub(crate) fn validate_append_event_request(
    request: &AppendEventRequest,
) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidEventRequest {
            reason: error.to_string(),
        })?;
    if request.envelope.kind == LogEntryKind::Commit {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: "commit events must be submitted through /commits".to_owned(),
        });
    }
    if request.envelope.room_id != request.room_id {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: format!(
                "event envelope room_id {} does not match request room_id {}",
                request.envelope.room_id, request.room_id
            ),
        });
    }
    if request.envelope.sender != request.sender {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: "event envelope sender does not match request sender".to_owned(),
        });
    }
    request
        .envelope
        .message_id()
        .map_err(|error| ServerHttpError::InvalidEventRequest {
            reason: error.to_string(),
        })?;
    Ok(())
}

pub(crate) fn validate_append_ephemeral_activity_request(
    request: &AppendEphemeralActivityRequest,
) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })?;
    validate_activity_expiry(request.received_at_ms, request.expires_at_ms).map_err(|error| {
        ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        }
    })
}

pub(crate) fn validate_get_ephemeral_activities_request(
    request: &GetEphemeralActivitiesRequest,
) -> Result<(), ServerHttpError> {
    validate_string_bytes("activity.room_id", &request.room_id, MAX_OBJECT_ID_BYTES).map_err(
        |error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        },
    )?;
    if let Some(conversation_id) = &request.conversation_id {
        validate_bytes_non_empty("activity.conversation_id", conversation_id.len()).map_err(
            |error| ServerHttpError::InvalidActivityRequest {
                reason: error.to_string(),
            },
        )?;
        validate_string_bytes(
            "activity.conversation_id",
            conversation_id,
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })?;
    }
    request
        .requester
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })
}

pub(crate) fn validate_device_liveness_request(
    request: &ObserveDeviceLivenessRequest,
) -> Result<(), ServerHttpError> {
    request.device.validate_limits().map_err(|error| {
        ServerHttpError::InvalidDeviceLivenessRequest {
            reason: error.to_string(),
        }
    })?;
    if request.expires_at_ms <= request.observed_at_ms {
        return Err(ServerHttpError::InvalidDeviceLivenessRequest {
            reason:
                "device_liveness.expires_at_ms must be greater than device_liveness.observed_at_ms"
                    .to_owned(),
        });
    }
    let window = request.expires_at_ms - request.observed_at_ms;
    if window > MAX_DEVICE_LIVENESS_EXPIRY_MILLIS {
        return Err(ServerHttpError::InvalidDeviceLivenessRequest {
            reason: format!(
                "device_liveness.expiry_window_millis has {window} ms, max {MAX_DEVICE_LIVENESS_EXPIRY_MILLIS}"
            ),
        });
    }
    Ok(())
}

pub(crate) fn normalize_nostr_profile_record(
    mut incoming: NostrProfileRecord,
    existing: Option<&NostrProfileRecord>,
) -> Result<NostrProfileRecord, ServerHttpError> {
    if incoming.bot.is_none() {
        incoming.bot = existing.and_then(|record| record.bot);
    }
    if incoming.finite_role.is_none() {
        incoming.finite_role = existing.and_then(|record| record.finite_role.clone());
    }
    incoming.metadata_json = Some(patched_nostr_profile_metadata_json(&incoming, existing)?);
    Ok(incoming)
}

fn patched_nostr_profile_metadata_json(
    incoming: &NostrProfileRecord,
    existing: Option<&NostrProfileRecord>,
) -> Result<String, ServerHttpError> {
    let mut object = existing
        .and_then(|record| record.metadata_json.as_deref())
        .or(incoming.metadata_json.as_deref())
        .map(nostr_profile_metadata_object)
        .transpose()?
        .unwrap_or_default();

    patch_json_string_field(&mut object, "name", incoming.name.as_deref());
    patch_json_string_field(
        &mut object,
        "display_name",
        incoming.display_name.as_deref(),
    );
    object.remove("displayName");
    patch_json_string_field(&mut object, "about", incoming.about.as_deref());
    patch_json_string_field(&mut object, "picture", incoming.picture.as_deref());
    object.remove("picture_url");
    if let Some(bot) = incoming.bot {
        object.insert("bot".to_owned(), serde_json::Value::Bool(bot));
    }
    patch_json_string_field(&mut object, "finite_role", incoming.finite_role.as_deref());
    object.remove("finiteRole");

    let encoded = serde_json::to_string(&serde_json::Value::Object(object)).map_err(|error| {
        ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("profile.metadata_json could not be encoded: {error}"),
        }
    })?;
    validate_bytes_len(
        "profile.metadata_json",
        encoded.len(),
        MAX_NOSTR_PROFILE_METADATA_JSON_BYTES as u32,
    )
    .map_err(|error| ServerHttpError::InvalidNostrProfileRequest {
        reason: error.to_string(),
    })?;
    Ok(encoded)
}

fn nostr_profile_metadata_object(
    metadata_json: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, ServerHttpError> {
    let value: serde_json::Value = serde_json::from_str(metadata_json).map_err(|error| {
        ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("profile.metadata_json must be valid JSON: {error}"),
        }
    })?;
    match value {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.metadata_json must be a JSON object".to_owned(),
        }),
    }
}

fn patch_json_string_field(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        object.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    } else {
        object.remove(key);
    }
}

pub(crate) fn validate_nostr_profile_record(
    record: &NostrProfileRecord,
) -> Result<(), ServerHttpError> {
    validate_nostr_account_id(&record.account_id)?;
    validate_optional_profile_text(
        "profile.name",
        record.name.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.display_name",
        record.display_name.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.about",
        record.about.as_deref(),
        MAX_NOSTR_PROFILE_ABOUT_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.picture",
        record.picture.as_deref(),
        MAX_NOSTR_PROFILE_PICTURE_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.finite_role",
        record.finite_role.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    if let Some(picture) = &record.picture
        && !picture.starts_with("http://")
        && !picture.starts_with("https://")
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.picture must be http(s)".to_owned(),
        });
    }
    if record.expires_at_ms <= record.fetched_at_ms {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.expires_at_ms must be greater than profile.fetched_at_ms".to_owned(),
        });
    }
    validate_nostr_profile_metadata_json(record.metadata_json.as_deref())?;
    Ok(())
}

// Only the byte cap is re-checked here: every caller passes a record fresh
// from `normalize_nostr_profile_record`, which already parsed, patched, and
// length-checked the metadata before re-serializing it.
fn validate_nostr_profile_metadata_json(
    metadata_json: Option<&str>,
) -> Result<(), ServerHttpError> {
    let Some(metadata_json) = metadata_json else {
        return Ok(());
    };
    validate_bytes_len(
        "profile.metadata_json",
        metadata_json.len(),
        MAX_NOSTR_PROFILE_METADATA_JSON_BYTES as u32,
    )
    .map_err(|error| ServerHttpError::InvalidNostrProfileRequest {
        reason: error.to_string(),
    })
}

pub(crate) fn validate_nostr_profile_batch(account_ids: &[String]) -> Result<(), ServerHttpError> {
    if account_ids.is_empty() || account_ids.len() > MAX_NOSTR_PROFILE_BATCH {
        return Err(ServerHttpError::InvalidNostrProfileBatch {
            actual: account_ids.len(),
            max: MAX_NOSTR_PROFILE_BATCH,
        });
    }
    let mut seen = BTreeSet::new();
    for account_id in account_ids {
        validate_nostr_account_id(account_id)?;
        if !seen.insert(account_id) {
            return Err(ServerHttpError::InvalidNostrProfileRequest {
                reason: format!("duplicate profile account_id {account_id}"),
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_key_package_availability_batch(
    account_ids: &[String],
) -> Result<(), ServerHttpError> {
    if account_ids.is_empty() || account_ids.len() > MAX_KEY_PACKAGE_AVAILABILITY_BATCH {
        return Err(ServerHttpError::InvalidKeyPackageAvailabilityBatch {
            actual: account_ids.len(),
            max: MAX_KEY_PACKAGE_AVAILABILITY_BATCH,
        });
    }
    for account_id in account_ids {
        validate_key_package_availability_account_id(account_id)?;
    }
    Ok(())
}

fn validate_nostr_account_id(account_id: &str) -> Result<(), ServerHttpError> {
    if account_id.len() != 64
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.account_id must be 64 lowercase hex characters".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_key_package_availability_account_id(
    account_id: &str,
) -> Result<(), ServerHttpError> {
    if account_id.len() != 64
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ServerHttpError::InvalidKeyPackageAvailabilityRequest {
            reason: "key_package_availability.account_id must be 64 lowercase hex characters"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_optional_profile_text(
    field: &'static str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), ServerHttpError> {
    if let Some(value) = value
        && value.len() > max_bytes
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("{field} must be at most {max_bytes} bytes"),
        });
    }
    Ok(())
}

pub(crate) const MAX_SYNC_WAIT_MILLIS: u64 = 25_000;
const MAX_SYNC_WAIT_ROOMS: usize = 256;
pub(crate) const DEFAULT_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 15_000;
pub(crate) const MIN_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 1_000;
pub(crate) const MAX_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 60_000;

pub(crate) fn validate_sync_wait_request(request: &SyncWaitRequest) -> Result<(), ServerHttpError> {
    validate_sync_watch_bounds(&request.rooms, "sync_wait")
}

pub(crate) fn validate_sync_stream_request(
    request: &SyncStreamRequest,
) -> Result<(), ServerHttpError> {
    validate_sync_watch_bounds(&request.rooms, "sync_stream")?;
    if let Some(inbox) = &request.inbox {
        let recipient_len = inbox.recipient.as_slice().len();
        if recipient_len == 0 || recipient_len > MAX_HTTP_ID_BYTES {
            return Err(ServerHttpError::InvalidSyncRequest {
                reason: format!(
                    "sync_stream inbox recipient must contain 1..={MAX_HTTP_ID_BYTES} bytes"
                ),
            });
        }
    }
    Ok(())
}

fn validate_sync_watch_bounds(
    rooms: &[finitechat_http::SyncWaitRoom],
    route: &str,
) -> Result<(), ServerHttpError> {
    if rooms.len() > MAX_SYNC_WAIT_ROOMS {
        return Err(ServerHttpError::InvalidSyncRequest {
            reason: format!("{route} watches at most {MAX_SYNC_WAIT_ROOMS} rooms"),
        });
    }
    for room in rooms {
        validate_sync_room_id(&room.room_id)?;
    }
    Ok(())
}

fn validate_sync_room_id(room_id: &str) -> Result<(), ServerHttpError> {
    validate_string_bytes("sync.room_id", room_id, MAX_OBJECT_ID_BYTES).map_err(|error| {
        ServerHttpError::InvalidSyncRequest {
            reason: error.to_string(),
        }
    })
}

pub(crate) fn validate_account_room_id(
    field: &'static str,
    value: &str,
) -> Result<(), ServerHttpError> {
    if value.is_empty() || value.len() > MAX_HTTP_ACCOUNT_ROOM_ID_BYTES {
        return Err(ServerHttpError::InvalidAccountRoomRequest {
            reason: format!(
                "{field} must contain between 1 and {MAX_HTTP_ACCOUNT_ROOM_ID_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

pub(crate) fn validate_key_package_claim_batch(owners: &[MemberId]) -> Result<(), ServerHttpError> {
    if owners.is_empty() || owners.len() > MAX_HTTP_SYNC_PAGE_ENTRIES {
        return Err(ServerHttpError::InvalidKeyPackageClaimBatch {
            actual: owners.len(),
            max: MAX_HTTP_SYNC_PAGE_ENTRIES,
        });
    }

    let mut seen = HashSet::new();
    for owner in owners {
        if !seen.insert(owner) {
            return Err(ServerHttpError::DuplicateKeyPackageClaimOwner {
                owner: owner.clone(),
            });
        }
    }
    Ok(())
}

pub(crate) fn usize_to_u32(field: &'static str, value: usize) -> Result<u32, ServerHttpError> {
    u32::try_from(value)
        .map_err(|_| ServerHttpError::KeyPackageInventoryCountOverflow { field, value })
}
