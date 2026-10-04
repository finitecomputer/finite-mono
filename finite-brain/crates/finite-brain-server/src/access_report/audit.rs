//! Signed audit for one current grant: the access-change event stored with
//! the grant counts as `verified` only when it is a validly signed,
//! canonical `grant-folder-access` change for exactly this Brain, Folder,
//! recipient, and key version, signed by the admin it names.

use finite_brain_core::{AdminAccessAction, AdminAccessChangePayload};
use finite_brain_store::AccessReportGrant;
use finite_nostr::{NostrPublicKey, verify_event_integrity};
use nostr::Event;

use crate::{APP_SPECIFIC_KIND, format_unix_timestamp};

const ACCESS_CHANGE_VERSION: &str = "finite-brain-admin-access-change-v1";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum SignedAudit {
    NotStored,
    /// Not parseable, not canonical, or the signature does not verify.
    Unreadable,
    /// Validly signed, but not a grant of exactly this Folder Key version to
    /// this recipient in this Brain.
    Mismatched {
        signer: String,
        at: Option<String>,
    },
    Verified {
        signer: String,
        at: Option<String>,
    },
}

impl SignedAudit {
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::NotStored => "notStored",
            Self::Unreadable => "unreadable",
            Self::Mismatched { .. } => "mismatched",
            Self::Verified { .. } => "verified",
        }
    }
}

pub(crate) fn signed_grant_audit(brain_id: &str, grant: &AccessReportGrant) -> SignedAudit {
    let Some(json) = grant.access_change_event_json.as_deref() else {
        return SignedAudit::NotStored;
    };
    let Ok(event) = Event::from_json(json) else {
        return SignedAudit::Unreadable;
    };
    if event.kind.as_u16() != APP_SPECIFIC_KIND || verify_event_integrity(&event).is_err() {
        return SignedAudit::Unreadable;
    }
    let Ok(payload) = serde_json::from_str::<AdminAccessChangePayload>(&event.content) else {
        return SignedAudit::Unreadable;
    };
    if payload.canonical_json() != event.content {
        return SignedAudit::Unreadable;
    }
    let Ok(signer) = NostrPublicKey::from_protocol(event.pubkey).to_npub() else {
        return SignedAudit::Unreadable;
    };
    let at = format_unix_timestamp(event.created_at.as_secs());
    let matches = payload.version == ACCESS_CHANGE_VERSION
        && payload.admin_npub == signer
        && (grant.issuer_npub.as_str() == signer
            || grant
                .provenance
                .delegated_by_npub
                .as_ref()
                .is_some_and(|key| key.as_str() == signer))
        && payload.brain_id == brain_id
        && payload.action == AdminAccessAction::GrantFolderAccess.as_str()
        && payload.folder_id.as_deref() == Some(grant.folder_id.as_str())
        && payload.target_npub.as_deref() == Some(grant.recipient_npub.as_str())
        && payload.key_version == Some(grant.key_version);
    if matches {
        SignedAudit::Verified { signer, at }
    } else {
        SignedAudit::Mismatched { signer, at }
    }
}
