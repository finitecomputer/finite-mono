//! Pure computation of report rows and coverage from one authority snapshot.
//! No I/O happens here; descriptions are attached afterwards for the page only.

use std::collections::{BTreeMap, BTreeSet};

use finite_brain_core::{BrainKind, Folder, FolderAccessMode, FolderId, UserId};
use finite_brain_store::{
    AccessReportAuthority, AccessReportGrant, AccessReportSnapshot, FolderAccessSource,
    IncomingMount, MemberProvenance,
};

use super::contracts::*;
use super::cursor;
use finite_nostr::NostrPublicKey;

/// Default and maximum identities per page.
pub(crate) const DEFAULT_PAGE_LIMIT: usize = 50;
pub(crate) const MAX_PAGE_LIMIT: usize = finite_brain_store::MAX_ACCESS_REPORT_EVIDENCE_KEYS;
/// Folder entries one page may carry; a page always carries at least one row.
pub(crate) const MAX_PAGE_FOLDER_ENTRIES: usize = 5_000;

/// How one explicit access row is sourced, from `folder_access_sources`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum ExplicitKind {
    /// Admin-written or invitation-written Guest access, or unsourced.
    Guest,
    /// Only active Mount sources.
    ActiveMount,
    /// Only Mount sources whose connection is no longer active.
    InactiveMount,
}

/// Precomputed lookup sets so each entitlement check is logarithmic.
pub(crate) struct AuthorityIndex<'a> {
    authority: &'a AccessReportAuthority,
    owner: Option<&'a UserId>,
    personal_agent: Option<&'a UserId>,
    admins: BTreeSet<&'a UserId>,
    members: BTreeSet<&'a UserId>,
    current_grants: BTreeMap<(&'a str, &'a UserId), &'a AccessReportGrant>,
    active_outgoing: BTreeSet<&'a str>,
}

impl<'a> AuthorityIndex<'a> {
    pub(crate) fn new(authority: &'a AccessReportAuthority) -> Self {
        Self {
            authority,
            owner: authority.brain.owner_user_id.as_ref(),
            personal_agent: authority
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
            admins: authority.brain.admins.iter().collect(),
            members: authority
                .brain
                .members
                .iter()
                .map(|member| &member.user_id)
                .collect(),
            current_grants: authority
                .current_grants
                .iter()
                .map(|grant| ((grant.folder_id.as_str(), &grant.recipient_npub), grant))
                .collect(),
            active_outgoing: authority
                .outgoing_connections
                .iter()
                .map(|connection| connection.id.as_str())
                .collect(),
        }
    }

    fn explicit_access(&self, folder: &Folder, key: &UserId) -> bool {
        self.authority
            .folder_access
            .get(&folder.id)
            .is_some_and(|users| users.contains(key))
    }

    fn recorded_sources(&self, folder_id: &FolderId, key: &UserId) -> Vec<&'a FolderAccessSource> {
        self.authority
            .folder_access_sources
            .get(&(folder_id.clone(), key.clone()))
            .into_iter()
            .flatten()
            .collect()
    }

    /// Labels for one explicit access row, from its recorded sources only.
    fn explicit_source_labels(&self, folder: &Folder, key: &UserId) -> Vec<String> {
        let sources = self.recorded_sources(&folder.id, key);
        if sources.is_empty() {
            return vec!["explicitAccessUnsourced".to_owned()];
        }
        sources
            .into_iter()
            .map(|source| match source.kind.as_str() {
                "direct" => "explicitAccess".to_owned(),
                "invitation" => "invitationAccess".to_owned(),
                "mount" if self.active_outgoing.contains(source.id.as_str()) => {
                    format!("mount:{}", source.id)
                }
                "mount" => format!("inactiveMount:{}", source.id),
                other => format!("unknownSource:{other}"),
            })
            .collect()
    }

    fn explicit_kind(&self, folder_id: &FolderId, key: &UserId) -> ExplicitKind {
        let sources = self.recorded_sources(folder_id, key);
        if sources.is_empty() || sources.iter().any(|source| source.kind != "mount") {
            ExplicitKind::Guest
        } else if sources
            .iter()
            .any(|source| self.active_outgoing.contains(source.id.as_str()))
        {
            ExplicitKind::ActiveMount
        } else {
            ExplicitKind::InactiveMount
        }
    }

    /// Entitlement sources; empty means not entitled. Mirrors the server's
    /// `folder_visible` rule exactly (tests pin the equivalence).
    pub(crate) fn entitlement_sources(&self, folder: &Folder, key: &UserId) -> Vec<String> {
        let mut sources = Vec::new();
        let is_owner = self.owner == Some(key);
        let is_admin = self.admins.contains(key);
        let is_member = self.members.contains(key);
        if self.personal_agent == Some(key) {
            sources.push("personalAgent".to_owned());
        }
        if self.explicit_access(folder, key) {
            sources.extend(self.explicit_source_labels(folder, key));
        }
        let native = match folder.access {
            FolderAccessMode::Owner => is_owner.then_some("brainOwner"),
            FolderAccessMode::AdminOnly | FolderAccessMode::Restricted => {
                if is_owner {
                    Some("brainOwner")
                } else if is_admin {
                    Some("brainAdmin")
                } else {
                    None
                }
            }
            FolderAccessMode::AllMembers => {
                if is_owner {
                    Some("brainOwner")
                } else if is_admin {
                    Some("brainAdmin")
                } else if is_member {
                    Some("allMembers")
                } else {
                    None
                }
            }
        };
        sources.extend(native.map(ToOwned::to_owned));
        sources
    }

    pub(crate) fn brain_role(&self, key: &UserId) -> &'static str {
        if self.owner == Some(key) {
            return "owner";
        }
        if self.personal_agent == Some(key) {
            return "personalAgent";
        }
        if self.admins.contains(key) {
            return "admin";
        }
        if self.members.contains(key) {
            return "member";
        }
        let kinds = self
            .authority
            .folder_access
            .iter()
            .filter(|(_, users)| users.contains(key))
            .map(|(folder_id, _)| self.explicit_kind(folder_id, key))
            .collect::<BTreeSet<_>>();
        if kinds.contains(&ExplicitKind::Guest) {
            "guest"
        } else if kinds.contains(&ExplicitKind::ActiveMount) {
            "mountParticipant"
        } else if kinds.contains(&ExplicitKind::InactiveMount) {
            "retainedMountAccess"
        } else {
            "noCurrentRole"
        }
    }

    fn role_sources(&self, key: &UserId, incoming: &[IncomingMountEntry]) -> Vec<String> {
        let mut sources = Vec::new();
        if self.owner == Some(key) {
            sources.push("brainOwner".to_owned());
        }
        if self.personal_agent == Some(key) {
            sources.push("personalAgent".to_owned());
        }
        if self.admins.contains(key) {
            sources.push("brainAdmin".to_owned());
        }
        if self.members.contains(key) {
            sources.push("brainMember".to_owned());
        }
        let has_folder_access = self
            .authority
            .folder_access
            .values()
            .any(|users| users.contains(key));
        if has_folder_access && !self.members.contains(key) {
            sources.push("folderAccess".to_owned());
        }
        sources.extend(
            self.authority
                .outgoing_connections
                .iter()
                .filter(|connection| connection.member_npubs.contains(key))
                .map(|connection| {
                    format!(
                        "mount:{}@{}",
                        connection.id, connection.destination_brain_id
                    )
                }),
        );
        sources.extend(
            incoming
                .iter()
                .map(|entry| format!("incomingMount:{}", entry.mount_id)),
        );
        if sources.is_empty() {
            sources.push("currentGrantOnly".to_owned());
        }
        sources
    }

    /// Count coverage without materializing rows or grant evidence.
    fn totals(&self, identities: usize) -> AccessTotals {
        let mut totals = AccessTotals {
            identities,
            revocation_incomplete: 0,
            grants_missing: 0,
        };
        for folder in &self.authority.brain.folders {
            let mut entitled = BTreeSet::new();
            entitled.extend(self.personal_agent);
            entitled.extend(self.owner);
            if folder.access != FolderAccessMode::Owner {
                entitled.extend(self.admins.iter().copied());
            }
            if folder.access == FolderAccessMode::AllMembers {
                entitled.extend(self.members.iter().copied());
            }
            entitled.extend(
                self.authority
                    .folder_access
                    .get(&folder.id)
                    .into_iter()
                    .flatten(),
            );
            totals.grants_missing += entitled.len();
        }
        let folders = self
            .authority
            .brain
            .folders
            .iter()
            .map(|folder| (&folder.id, folder))
            .collect::<BTreeMap<_, _>>();
        for grant in &self.authority.current_grants {
            let Some(folder) = folders.get(&grant.folder_id) else {
                continue;
            };
            if self.has_entitlement(folder, &grant.recipient_npub) {
                totals.grants_missing -= 1;
            } else {
                totals.revocation_incomplete += 1;
            }
        }
        totals
    }

    fn has_entitlement(&self, folder: &Folder, key: &UserId) -> bool {
        self.personal_agent == Some(key)
            || self.owner == Some(key)
            || self.explicit_access(folder, key)
            || (folder.access != FolderAccessMode::Owner && self.admins.contains(key))
            || (folder.access == FolderAccessMode::AllMembers && self.members.contains(key))
    }

    fn folder_entry_count(&self, key: &UserId) -> usize {
        self.authority
            .brain
            .folders
            .iter()
            .filter(|folder| {
                self.has_entitlement(folder, key)
                    || self.current_grants.contains_key(&(folder.id.as_str(), key))
            })
            .count()
    }

    fn folder_entries(
        &self,
        key: &UserId,
        evidence: &mut impl FnMut(&str, &AccessReportGrant) -> GrantEvidence,
    ) -> Vec<FolderAccessEntry> {
        let brain_id = self.authority.brain.id.as_str();
        let mut entries = Vec::new();
        for folder in &self.authority.brain.folders {
            let sources = self.entitlement_sources(folder, key);
            let grant = self.current_grants.get(&(folder.id.as_str(), key)).copied();
            let entitled = !sources.is_empty();
            if !entitled && grant.is_none() {
                continue;
            }
            let state = match (entitled, grant.is_some()) {
                (true, true) => "ready",
                (true, false) => "grantMissing",
                _ => "revocationIncomplete",
            };
            entries.push(FolderAccessEntry {
                folder_id: folder.id.to_string(),
                path: folder.path.to_string(),
                access_mode: access_mode_name(folder.access).to_owned(),
                key_version: folder.current_key_version,
                entitled,
                entitlement_sources: sources,
                current_grant: if grant.is_some() {
                    "present"
                } else {
                    "missing"
                }
                .to_owned(),
                state: state.to_owned(),
                grant: grant.map(|grant| evidence(brain_id, grant)),
            });
        }
        entries
    }
}

fn incoming_entries(mounts: &[IncomingMount], key: &UserId) -> Vec<IncomingMountEntry> {
    let mut entries = Vec::new();
    for mount in mounts.iter().filter(|mount| mount.is_active()) {
        let Some(participant) = mount
            .participants
            .iter()
            .flatten()
            .find(|participant| participant.npub == *key)
        else {
            continue;
        };
        let state = match (participant.mount_access, participant.current_grant) {
            (true, true) => "ready",
            (true, false) => "grantMissing",
            (false, _) => "mountAccessMissing",
        };
        entries.push(IncomingMountEntry {
            mount_id: mount.mount_id.clone(),
            display_name: mount.display_name.clone(),
            source_brain_id: mount.source_brain_id.to_string(),
            source_folder_id: mount.source_folder_id.to_string(),
            mount_access: participant.mount_access,
            current_grant: if participant.current_grant {
                "present"
            } else {
                "missing"
            }
            .to_owned(),
            state: state.to_owned(),
        });
    }
    entries
}

pub(crate) fn incoming_mount_views(authority: &AccessReportAuthority) -> Vec<IncomingMountView> {
    authority
        .incoming_mounts
        .iter()
        .map(|mount| IncomingMountView {
            mount_id: mount.mount_id.clone(),
            display_name: mount.display_name.clone(),
            source_brain_id: mount.source_brain_id.to_string(),
            source_folder_id: mount.source_folder_id.to_string(),
            connection_status: mount.connection_status.clone(),
            participant_detail: if mount.participants.is_some() {
                "complete"
            } else {
                "withheld"
            }
            .to_owned(),
            participants: mount.participants.as_ref().map_or(0, Vec::len),
        })
        .collect()
}

/// Parsed page request. `fingerprint` is the authority a cursor was issued
/// under; a later page must come from the same authority.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PageRequest {
    pub(crate) limit: usize,
    pub(crate) after: Option<UserId>,
    pub(crate) fingerprint: Option<String>,
}

/// Rows for one page plus totals over every identity.
pub(crate) struct ReportRows {
    pub(crate) rows: Vec<AccessIdentityRow>,
    pub(crate) totals: AccessTotals,
    pub(crate) next: Option<String>,
}

/// Build the page's rows in stable npub order. Totals cover every identity
/// so a page never implies the Brain has fewer gaps than it does.
pub(crate) fn build_rows(snapshot: &AccessReportSnapshot, page: &PageRequest) -> ReportRows {
    build_rows_with_grant_evidence(snapshot, page, grant_evidence)
}

fn build_rows_with_grant_evidence(
    snapshot: &AccessReportSnapshot,
    page: &PageRequest,
    mut evidence: impl FnMut(&str, &AccessReportGrant) -> GrantEvidence,
) -> ReportRows {
    let authority = &snapshot.authority;
    let index = AuthorityIndex::new(authority);
    let keys = authority.candidate_keys();
    let totals = index.totals(keys.len());
    let mut rows = Vec::new();
    let mut entries_on_page = 0usize;
    let mut next = None;
    for key in keys
        .iter()
        .filter(|key| page.after.as_ref().is_none_or(|after| *key > after))
    {
        let count = index.folder_entry_count(key);
        if !rows.is_empty()
            && (rows.len() >= page.limit
                || !snapshot.evidence_keys.contains(key)
                || entries_on_page + count > MAX_PAGE_FOLDER_ENTRIES)
        {
            next = rows
                .last()
                .map(|row: &AccessIdentityRow| cursor::encode(&authority.fingerprint, &row.npub));
            break;
        }
        // Only returned rows incur allocations and grant evidence.
        let entries = index.folder_entries(key, &mut evidence);
        let revocation_incomplete = entries
            .iter()
            .filter(|entry| entry.state == "revocationIncomplete")
            .map(|entry| entry.folder_id.clone())
            .collect();
        let missing_current_grants = entries
            .iter()
            .filter(|entry| entry.state == "grantMissing")
            .map(|entry| entry.folder_id.clone())
            .collect();
        entries_on_page += entries.len();
        let incoming = incoming_entries(&authority.incoming_mounts, key);
        rows.push(AccessIdentityRow {
            npub: key.to_string(),
            hex: NostrPublicKey::parse(key.as_str())
                .map(|key| key.to_hex())
                .unwrap_or_default(),
            brain_role: index.brain_role(key).to_owned(),
            role_sources: index.role_sources(key, &incoming),
            membership: snapshot.member_provenance.get(key).map(member_view),
            participation: snapshot.verified_participation.get(key).map(|evidence| {
                ParticipationView {
                    kind: evidence.kind.as_str().to_owned(),
                    recorded_at: evidence.recorded_at.clone(),
                }
            }),
            description: IdentityDescriptionView::withheld("descriptions not yet attached"),
            stored_nip05: None,
            folders: entries,
            incoming_mounts: incoming,
            missing_current_grants,
            revocation_incomplete,
        });
    }
    ReportRows { rows, totals, next }
}

pub(crate) fn acting_role(authority: &AccessReportAuthority, actor: &UserId) -> String {
    AuthorityIndex::new(authority).brain_role(actor).to_owned()
}

/// Coverage per scope. A scope is `complete` only when every current fact
/// in it was read and is attributable; anything else names its reason.
pub(crate) fn coverage(authority: &AccessReportAuthority) -> AccessCoverage {
    let state = |state: &str, reason: Option<String>| ScopeCoverage {
        state: state.to_owned(),
        reason,
    };
    AccessCoverage {
        members: state("complete", None),
        guests: state(
            "complete",
            (authority.brain.kind == BrainKind::Personal)
                .then(|| "Personal Brain Folder Guests from explicit Folder access".to_owned()),
        ),
        mounts: mount_coverage(authority),
        current_grants: state(
            "complete",
            Some(
                "a present current grant means a wrapped key exists for that key; it does not prove the key decrypted it"
                    .to_owned(),
            ),
        ),
        access_history: state(
            "unsupported",
            Some(
                "past access changes are not reconstructed; grant issuer and time come from stored grant provenance"
                    .to_owned(),
            ),
        ),
    }
}

/// Mounts are complete only when outgoing Mount access is fully sourced and
/// active, and nothing is mounted in from another Brain.
fn mount_coverage(authority: &AccessReportAuthority) -> ScopeCoverage {
    let active_outgoing = authority
        .outgoing_connections
        .iter()
        .map(|connection| connection.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut reasons = Vec::new();
    if authority.pending_mount_source_repairs > 0 {
        reasons.push(format!(
            "{} Mount access row(s) await legacy source repair; their Mount attribution is unverified",
            authority.pending_mount_source_repairs
        ));
    }
    let retained = authority
        .folder_access_sources
        .values()
        .flatten()
        .filter(|source| source.kind == "mount" && !active_outgoing.contains(source.id.as_str()))
        .count();
    if retained > 0 {
        reasons.push(format!(
            "{retained} Folder access row(s) are still sourced by a Mount that is no longer active (shown as inactiveMount)"
        ));
    }
    let active_incoming = authority
        .incoming_mounts
        .iter()
        .filter(|mount| mount.is_active())
        .collect::<Vec<_>>();
    if !active_incoming.is_empty() {
        let mounts = active_incoming
            .iter()
            .map(|mount| {
                format!(
                    "{} from {}/{}",
                    mount.mount_id, mount.source_brain_id, mount.source_folder_id
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        reasons.push(format!(
            "{} Folder(s) are mounted into this Brain ({mounts}); participants' Mount access and current grants are shown, but access to those source Folders by other routes is reported only by their source Brains",
            active_incoming.len()
        ));
    }
    let withheld = active_incoming
        .iter()
        .filter(|mount| mount.participants.is_none())
        .count();
    if withheld > 0 {
        reasons.push(format!(
            "participant detail for {withheld} incoming Mount(s) exceeded the report bound and was not checked"
        ));
    }
    if reasons.is_empty() {
        ScopeCoverage {
            state: "complete".to_owned(),
            reason: Some(
                "covers Folders this Brain shares out; no Folder is mounted into this Brain"
                    .to_owned(),
            ),
        }
    } else {
        ScopeCoverage {
            state: "unverified".to_owned(),
            reason: Some(reasons.join("; ")),
        }
    }
}

pub(crate) fn current_access_complete(coverage: &AccessCoverage) -> bool {
    [
        &coverage.members,
        &coverage.guests,
        &coverage.mounts,
        &coverage.current_grants,
    ]
    .iter()
    .all(|scope| scope.state == "complete")
}

fn member_view(provenance: &MemberProvenance) -> ProvenanceView {
    ProvenanceView {
        origin: provenance.origin_kind.as_str().to_owned(),
        delegated_by: provenance
            .delegated_by_npub
            .as_ref()
            .map(ToString::to_string),
        origin_ref: provenance.origin_ref.clone(),
    }
}

fn grant_evidence(_brain_id: &str, grant: &AccessReportGrant) -> GrantEvidence {
    GrantEvidence {
        grant_id: grant.id.clone(),
        issued_by: grant.issuer_npub.to_string(),
        issued_at: grant.created_at.clone(),
        provenance: ProvenanceView {
            origin: grant.provenance.origin_kind.as_str().to_owned(),
            delegated_by: grant
                .provenance
                .delegated_by_npub
                .as_ref()
                .map(ToString::to_string),
            origin_ref: grant.provenance.origin_ref.clone(),
        },
    }
}

pub(crate) fn access_mode_name(mode: FolderAccessMode) -> &'static str {
    match mode {
        FolderAccessMode::Owner => "owner",
        FolderAccessMode::AdminOnly => "admin_only",
        FolderAccessMode::AllMembers => "all_members",
        FolderAccessMode::Restricted => "restricted",
    }
}

impl IdentityDescriptionView {
    /// A row-only state with a reason and no description fields.
    pub(crate) fn bare(state: &str, reason: Option<&str>) -> Self {
        Self {
            state: state.to_owned(),
            reason: reason.map(ToOwned::to_owned),
            kind: None,
            display_name: None,
            account_email: None,
            lifecycle: None,
            responsible_account: None,
            source: None,
        }
    }

    /// Brain withholds private description for this key.
    pub(crate) fn withheld(reason: &str) -> Self {
        Self::bare("notShared", Some(reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::{
        Brain, BrainId, BrainMember, DisplayName, FolderRole, SafeRelativePath,
    };
    use finite_brain_store::GrantProvenance;

    #[test]
    fn page_reads_grant_evidence_only_for_returned_rows_while_totals_cover_off_page_gaps() {
        let keys = (0..100)
            .map(|index| UserId::new(format!("key-{index:03}")).unwrap())
            .collect::<Vec<_>>();
        let folder = Folder {
            id: FolderId::new("notes").unwrap(),
            name: DisplayName::new("name", "Notes").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::AllMembers,
            parent_folder_id: None,
            path: SafeRelativePath::new("path", "Notes").unwrap(),
            current_key_version: 1,
        };
        // Last key is removed but retains a current grant; the penultimate Member lacks one.
        let grants = keys
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 98)
            .map(|(index, key)| AccessReportGrant {
                id: format!("grant-{index}"),
                folder_id: folder.id.clone(),
                key_version: 1,
                issuer_npub: keys[0].clone(),
                recipient_npub: key.clone(),
                access_change_event_json: None,
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                provenance: GrantProvenance::direct(),
            })
            .collect();
        let snapshot = AccessReportSnapshot {
            authority: AccessReportAuthority {
                brain: Brain {
                    id: BrainId::new("synthetic").unwrap(),
                    kind: BrainKind::Organization,
                    name: DisplayName::new("name", "Synthetic").unwrap(),
                    owner_user_id: None,
                    folders: vec![folder],
                    admins: vec![keys[0].clone()],
                    members: keys[..99]
                        .iter()
                        .map(|key| BrainMember {
                            user_id: key.clone(),
                            folder_access: BTreeSet::new(),
                        })
                        .collect(),
                },
                personal_agent: None,
                folder_access: BTreeMap::new(),
                folder_access_sources: BTreeMap::new(),
                current_grants: grants,
                outgoing_connections: vec![],
                incoming_mounts: vec![],
                pending_mount_source_repairs: 0,
                latest_sequence: 0,
                fingerprint: "a".repeat(64),
            },
            evidence_keys: BTreeSet::from([keys[0].clone()]),
            member_provenance: BTreeMap::new(),
            verified_participation: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        let mut audited = Vec::new();
        let report = build_rows_with_grant_evidence(
            &snapshot,
            &PageRequest {
                limit: 1,
                after: None,
                fingerprint: None,
            },
            |brain, grant| {
                audited.push(grant.recipient_npub.clone());
                grant_evidence(brain, grant)
            },
        );
        assert_eq!(report.rows.len(), 1);
        assert_eq!(audited, vec![keys[0].clone()]);
        assert_eq!(report.totals.identities, 100);
        assert_eq!(report.totals.revocation_incomplete, 1);
        assert_eq!(report.totals.grants_missing, 1);
        assert!(report.next.is_some());
    }
}
