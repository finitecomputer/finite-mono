//! Stable digest of current Brain authority. Computed on read and never
//! stored. Content record sequences are informational and excluded: ordinary
//! edits do not change access. Access changes without a Brain record (Invite
//! redemption, invitation acceptance, Mount participation) are included here.

use super::{AccessReportAuthority, FolderAccessSource};
use crate::*;

const FINGERPRINT_VERSION: &str = "finite-brain-access-authority-v1";

/// Length-prefixed fields so no value can forge a field boundary.
struct Digest(String);

impl Digest {
    fn field(&mut self, value: &str) -> &mut Self {
        self.0.push_str(&value.len().to_string());
        self.0.push(':');
        self.0.push_str(value);
        self
    }

    fn end(&mut self) {
        self.0.push('\n');
    }
}

pub(super) fn authority_fingerprint(authority: &AccessReportAuthority) -> String {
    let mut digest = Digest(String::new());
    let brain = &authority.brain;
    digest
        .field(FINGERPRINT_VERSION)
        .field(brain.id.as_str())
        .field(match brain.kind {
            BrainKind::Personal => "personal",
            BrainKind::Organization => "organization",
        })
        .field(brain.name.as_str())
        .field(&authority.pending_mount_source_repairs.to_string())
        .end();
    digest
        .field("owner")
        .field(brain.owner_user_id.as_ref().map_or("", UserId::as_str))
        .field(
            authority
                .personal_agent
                .as_ref()
                .map_or("", |relationship| relationship.agent_npub.as_str()),
        )
        .end();
    for admin in &brain.admins {
        digest.field("admin").field(admin.as_str()).end();
    }
    for member in &brain.members {
        digest.field("member").field(member.user_id.as_str()).end();
    }
    for folder in &brain.folders {
        digest
            .field("folder")
            .field(folder.id.as_str())
            .field(folder.path.as_str())
            .field(&format!("{:?}", folder.access))
            .field(&folder.current_key_version.to_string())
            .end();
    }
    for (folder_id, users) in &authority.folder_access {
        for user in users {
            digest
                .field("access")
                .field(folder_id.as_str())
                .field(user.as_str());
            let key = (folder_id.clone(), user.clone());
            for FolderAccessSource { kind, id } in authority
                .folder_access_sources
                .get(&key)
                .into_iter()
                .flatten()
            {
                digest.field(kind).field(id);
            }
            digest.end();
        }
    }
    for grant in &authority.current_grants {
        digest
            .field("grant")
            .field(&grant.id)
            .field(grant.folder_id.as_str())
            .field(&grant.key_version.to_string())
            .field(grant.issuer_npub.as_str())
            .field(grant.recipient_npub.as_str())
            .field(&grant.created_at)
            .field(
                grant
                    .provenance
                    .delegated_by_npub
                    .as_ref()
                    .map_or("", UserId::as_str),
            )
            .field(grant.provenance.origin_kind.as_str())
            .field(grant.provenance.origin_ref.as_deref().unwrap_or(""))
            .field(&format!("{:?}", grant.provenance.roster_revision))
            .field(
                &grant
                    .access_change_event_json
                    .as_deref()
                    .map(finite_brain_core::sha256_hex)
                    .unwrap_or_default(),
            )
            .end();
    }
    for connection in &authority.outgoing_connections {
        digest
            .field("outgoing")
            .field(&connection.id)
            .field(connection.source_folder_id.as_str())
            .field(connection.destination_brain_id.as_str());
        for member in &connection.member_npubs {
            digest.field(member.as_str());
        }
        digest.end();
    }
    for mount in &authority.incoming_mounts {
        digest
            .field("incoming")
            .field(&mount.mount_id)
            .field(&mount.connection_id)
            .field(&mount.connection_status)
            .field(mount.source_brain_id.as_str())
            .field(mount.source_folder_id.as_str())
            .field(&mount.display_name)
            .field(&format!("{:?}", mount.source_key_version));
        match &mount.participants {
            None => {
                digest.field("withheld");
            }
            Some(participants) => {
                for participant in participants {
                    digest
                        .field(participant.npub.as_str())
                        .field(if participant.mount_access {
                            "access"
                        } else {
                            "-"
                        })
                        .field(if participant.current_grant {
                            "grant"
                        } else {
                            "-"
                        });
                }
            }
        }
        digest.end();
    }
    finite_brain_core::sha256_hex(digest.0)
}
