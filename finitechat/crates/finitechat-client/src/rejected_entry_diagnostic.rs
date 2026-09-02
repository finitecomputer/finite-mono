//! Operator-only rejected-entry classifier (Track B incident diagnostic).
//!
//! A hosted device can sit exactly one application delivery behind in one
//! room: its durable sync cursor refuses to advance past one rejected log
//! entry while every other room progresses independently. Core records only
//! the failed room id in `CoreSyncProjection.room_sync_failures`, which is
//! enough to quarantine the room but not to distinguish *why* the entry is
//! rejected: a malformed entry (every device would choke) or a diverged
//! local MLS state (only this device chokes). The production repair depends
//! on that classification.
//!
//! This module is the diagnostic that makes the distinction possible. It
//! runs against a byte copy of a client store (never the live store, never
//! a live server) and replays a captured room log through the production
//! bounded sync tick, then attributes the exact rejected entry on a pristine
//! in-memory device. It is classification only: it never skips an entry,
//! never edits a cursor, never replaces device state, and never selects a
//! repair.
//!
//! Privacy contract for the emitted record: no room/account/device/project/
//! runtime identifiers (only a caller-supplied incident alias), no
//! plaintext, no ciphertext bytes (only their SHA-256), no filenames or
//! attachment metadata, no emails/display names/URLs/tokens/keys, and no
//! raw exception strings.

use std::fs;
use std::path::{Path, PathBuf};

use finitechat_proto::{
    ClaimKeyPackageResult, CommitAccepted, DeviceRef, KeyPackageInventory, ListAccountRoomsPage,
    ListAccountRoomsRequest, LogEntryKind, RoomLogEntry, SubmitCommitRequest, SyncEventsPage,
    UploadKeyPackageRequest, WelcomeRecord,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    ClientError, ClientStoreError, FiniteChatDeviceConfig, RuntimeDelivery, RuntimeSyncOptions,
    RuntimeWorkerError, SqliteClientStore, SqliteClientStoreOptions, apply_log_entry_in_memory,
    hex_lower, run_room_sync_tick,
};

/// Schema version of [`RejectedEntryDiagnostic`]. Bump on any field change.
pub const REJECTED_ENTRY_DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;

/// How many captured entries one replayed sync page carries. Matches the
/// production HTTP page cap so replayed paging is faithful.
const REPLAY_PAGE_ENTRIES: usize = finitechat_delivery::MAX_HTTP_SYNC_PAGE_ENTRIES;

/// Upper bound on replayed pages per room, matching the production bounded
/// tick. A capture that does not fit is rejected instead of truncated.
const REPLAY_MAX_SYNC_PAGES_PER_ROOM: u32 = 64;

const MAX_INCIDENT_ALIAS_BYTES: usize = 128;
const REPLAY_STORE_COPY_NAME: &str = "rejected-entry-replay-store.sqlite3";

/// Coarse error class for a rejected entry. Stable strings; these are the
/// only failure detail that may leave the operator's machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectedEntryErrorClass {
    /// Transport/delivery failure (unreachable in a captured replay; kept
    /// for the production-path mapping).
    TransportDelivery,
    /// Local encrypted-store failure (sqlite/encryption layer).
    LocalEncryptedStore,
    /// Protocol/envelope parsing or shape validation failure.
    ProtocolEnvelopeParsing,
    /// MLS application/ciphertext failure on an application entry.
    MlsApplicationCiphertext,
    /// MLS epoch or local group-state mismatch ("only this device chokes").
    MlsEpochOrStateMismatch,
    /// Commit/proposal/membership processing failure.
    CommitProposalMembership,
    /// Anything else. Always stops; never selects a repair.
    UnsupportedUnclassified,
}

/// Kind of the rejected log entry, coarsened to the incident vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectedEntryKind {
    Application,
    Commit,
    Other,
}

/// What the classification implies for any later repair. This tool never
/// skips, never moves a cursor, and never selects a repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairDisposition {
    /// A Commit/membership failure is never skippable and must never enter
    /// an application-gap path.
    NonSkippableCommit,
    /// Unsupported/unclassified failure: stop; no repair may be selected.
    StopUnclassified,
    /// Classified application-layer failure. Classification only; the
    /// repair decision belongs to a later phase.
    ClassificationOnly,
}

/// Outcome of replaying the target room against the captured log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOutcome {
    /// The replay rejected an entry and failed closed.
    Rejected,
    /// The replay advanced the durable cursor on the throwaway copy.
    Advanced,
    /// The replay succeeded but had nothing new to apply.
    Unchanged,
}

/// Binding for the exact rejected entry: sequence, kind, and the SHA-256 of
/// the exact opaque envelope ciphertext bytes (binding only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedEntryRecord {
    pub seq: u64,
    pub kind: RejectedEntryKind,
    /// Lowercase hex SHA-256 over `entry.envelope.payload`.
    pub sha256: String,
}

/// The structured diagnostic record. Contains no identifiers, no plaintext,
/// no ciphertext, and no raw error strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedEntryDiagnostic {
    pub schema_version: u32,
    /// Caller-supplied incident-scoped alias (validated charset).
    pub incident_alias: String,
    /// Durable target-room cursor before replay.
    pub cursor_before: u64,
    /// Durable target-room cursor after the replay attempt.
    pub cursor_after: u64,
    pub replay_outcome: ReplayOutcome,
    /// Exact rejected entry binding; `None` when no single entry could be
    /// attributed (success, or a tick-scoped failure such as the atomic
    /// persistence boundary).
    pub rejected: Option<RejectedEntryRecord>,
    pub error_class: Option<RejectedEntryErrorClass>,
    pub repair_disposition: Option<RepairDisposition>,
    /// Whether the target-room replay attempt persisted a device-state
    /// candidate. Fail-closed replay keeps this `false` on rejection.
    /// (Other rooms may legitimately advance on the throwaway copy; that
    /// is reported separately.)
    pub device_state_candidate_persisted: bool,
    pub other_rooms_attempted: u32,
    pub other_rooms_advanced: u32,
    /// `true` only when at least one other room was replayed and every
    /// other room advanced its durable cursor independently.
    pub later_rooms_continued: bool,
}

/// A captured room log: the ordered entries of one room, copied out of band
/// (e.g. from a restored snapshot). Input only; room ids never leave the
/// operator's machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedRoomLog {
    pub room_id: String,
    pub entries: Vec<RoomLogEntry>,
}

/// File format for the captured logs consumed by the operator CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedRoomLogFile {
    pub target_room_id: String,
    pub rooms: Vec<CapturedRoomLog>,
}

/// Everything the diagnostic needs. All inputs are local copies.
#[derive(Debug)]
pub struct RejectedEntryDiagnosticRequest {
    /// Path to the source client store sqlite file. Only ever read; the
    /// diagnostic byte-copies it into `work_dir` before opening sqlite.
    pub source_db_path: PathBuf,
    /// Scratch directory for the throwaway store copy.
    pub work_dir: PathBuf,
    /// Device config (account secret + device id + fixed clock) used to
    /// decrypt the copied store. Never appears in the record.
    pub config: FiniteChatDeviceConfig,
    /// Incident-scoped alias supplied by the operator.
    pub incident_alias: String,
    /// Captured log of the room whose cursor refuses to advance.
    pub target: CapturedRoomLog,
    /// Captured logs of other rooms, used to prove independent progress.
    pub other_rooms: Vec<CapturedRoomLog>,
}

#[derive(Debug, thiserror::Error)]
pub enum RejectedEntryDiagnosticError {
    #[error(
        "incident alias must be 1..=128 bytes of [A-Za-z0-9._:-] (no paths, urls, or identifiers)"
    )]
    InvalidIncidentAlias,
    #[error("captured log entry room does not match its capture")]
    CaptureRoomMismatch,
    #[error("captured log has duplicate or out-of-order entries")]
    CaptureNotOrdered,
    #[error("target room is not present in the copied store")]
    TargetRoomMissing,
    #[error("captured log exceeds the bounded replay limit")]
    ReplayBoundExceeded,
    #[error("source store path resolves to the replay copy; pass the original")]
    SourceIsReplayCopy,
    #[error("store copy I/O failed: {reason}")]
    Io { reason: String },
    #[error(transparent)]
    Store(#[from] ClientStoreError),
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Debug, thiserror::Error)]
enum CapturedLogDeliveryError {
    #[error("captured replay cannot submit commits")]
    SubmitCommitUnsupported,
}

/// Serves one captured room log through the production `RuntimeDelivery`
/// interface, one bounded page at a time. No network, no server.
struct CapturedLogDelivery<'a> {
    log: &'a CapturedRoomLog,
}

impl RuntimeDelivery for CapturedLogDelivery<'_> {
    type Error = CapturedLogDeliveryError;

    fn key_package_inventory(
        &mut self,
        owner: &DeviceRef,
    ) -> Result<KeyPackageInventory, Self::Error> {
        Ok(KeyPackageInventory {
            owner: owner.clone(),
            available: 0,
            leased: 0,
        })
    }

    fn upload_key_package(&mut self, _request: UploadKeyPackageRequest) -> Result<(), Self::Error> {
        Ok(())
    }

    fn claim_key_package_for_device(
        &mut self,
        _owner: &DeviceRef,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
        Ok(None)
    }

    fn claim_key_package_for_account(
        &mut self,
        _account_id: &str,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
        Ok(None)
    }

    fn submit_commit(
        &mut self,
        _request: SubmitCommitRequest,
    ) -> Result<CommitAccepted, Self::Error> {
        Err(CapturedLogDeliveryError::SubmitCommitUnsupported)
    }

    fn list_account_rooms(
        &mut self,
        _request: ListAccountRoomsRequest,
    ) -> Result<ListAccountRoomsPage, Self::Error> {
        Ok(ListAccountRoomsPage {
            rooms: Vec::new(),
            next_after_room_id: None,
            has_more: false,
        })
    }

    fn claim_welcomes(&mut self, _device: &DeviceRef) -> Result<Vec<WelcomeRecord>, Self::Error> {
        Ok(Vec::new())
    }

    fn ack_welcome(&mut self, _welcome_id: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn sync_events(
        &mut self,
        room_id: &str,
        _requester: &DeviceRef,
        after_seq: u64,
    ) -> Result<SyncEventsPage, Self::Error> {
        debug_assert_eq!(room_id, self.log.room_id);
        let page = self
            .log
            .entries
            .iter()
            .filter(|entry| entry.seq > after_seq)
            .take(REPLAY_PAGE_ENTRIES + 1)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = page.len() > REPLAY_PAGE_ENTRIES;
        let page = page
            .into_iter()
            .take(REPLAY_PAGE_ENTRIES)
            .collect::<Vec<_>>();
        let next_after_seq = page.last().map(|entry| entry.seq).unwrap_or(after_seq);
        Ok(SyncEventsPage {
            entries: page,
            next_after_seq,
            has_more,
        })
    }
}

/// Run the rejected-entry diagnostic against a copied store. Classification
/// only; never writes the source store, never touches a server.
pub fn run_rejected_entry_diagnostic(
    request: &RejectedEntryDiagnosticRequest,
) -> Result<RejectedEntryDiagnostic, RejectedEntryDiagnosticError> {
    validate_incident_alias(&request.incident_alias)?;
    validate_capture(&request.target)?;
    for room in &request.other_rooms {
        validate_capture(room)?;
    }

    let copy_path = copy_store_for_replay(&request.source_db_path, &request.work_dir)?;
    let mut store = SqliteClientStore::open(
        &copy_path,
        SqliteClientStoreOptions::from_nostr_secret(
            &request.config.account_secret_key,
            &request.config.device_id,
        )?,
    )?;

    let mut device = store.load_device(request.config.clone())?;
    let state_before = device.export_state()?;
    let cursor_before = device
        .last_applied_seq(&request.target.room_id)
        .map_err(|_| RejectedEntryDiagnosticError::TargetRoomMissing)?;

    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: REPLAY_MAX_SYNC_PAGES_PER_ROOM,
    };

    // Phase A: replay the target room through the unmodified production
    // bounded tick against the captured log. On failure the tick persists
    // nothing (fail closed), exactly like the production Core path.
    let tick = {
        let mut delivery = CapturedLogDelivery {
            log: &request.target,
        };
        run_room_sync_tick(
            &mut store,
            &mut device,
            &mut delivery,
            &options,
            &request.target.room_id,
        )
    };

    // Read the durable state back from the copy; never trust the in-memory
    // candidate after a failed tick.
    let reloaded = store.load_device(request.config.clone())?;
    let state_after = reloaded.export_state()?;
    let cursor_after = reloaded
        .last_applied_seq(&request.target.room_id)
        .map_err(|_| RejectedEntryDiagnosticError::TargetRoomMissing)?;
    let device_state_candidate_persisted = state_after != state_before;

    let (replay_outcome, rejected, error_class, repair_disposition) = match tick {
        Ok(_) => {
            if let Some(max_seq) = request.target.entries.iter().map(|entry| entry.seq).max()
                && max_seq > cursor_after
            {
                // The bounded tick finished without consuming the capture:
                // the replay bound was hit instead of the log end. Report
                // nothing rather than a truncated replay.
                return Err(RejectedEntryDiagnosticError::ReplayBoundExceeded);
            }
            let outcome = if cursor_after > cursor_before {
                ReplayOutcome::Advanced
            } else {
                ReplayOutcome::Unchanged
            };
            (outcome, None, None, None)
        }
        Err(worker_error) => {
            let tick_class = classify_worker_error(None, &worker_error);
            // Phase B: attribute the exact rejected entry on a pristine
            // in-memory device loaded from the unchanged durable state.
            // Entries are applied with the production per-entry function;
            // the first entry that fails is the rejected one (never
            // inferred as cursor + 1). Nothing here is persisted.
            let mut probe = store.load_device(request.config.clone())?;
            let mut rejected = None;
            let mut class = tick_class;
            for entry in &request.target.entries {
                if entry.seq <= cursor_before {
                    continue;
                }
                if let Err(store_error) =
                    apply_log_entry_in_memory(&mut probe, &request.target.room_id, entry)
                {
                    class = classify_store_error(Some(entry.kind), &store_error);
                    rejected = Some(RejectedEntryRecord {
                        seq: entry.seq,
                        kind: rejected_entry_kind(entry.kind),
                        sha256: sha256_hex(&entry.envelope.payload),
                    });
                    break;
                }
            }
            let rejected_kind = rejected.as_ref().map(|record| record.kind);
            let disposition = repair_disposition(class, rejected_kind);
            (
                ReplayOutcome::Rejected,
                rejected,
                Some(class),
                Some(disposition),
            )
        }
    };

    // Other rooms replay independently, mirroring the production Core loop:
    // each room starts from a fresh durable load, so the failed target room
    // cannot poison them.
    let mut other_rooms_attempted = 0u32;
    let mut other_rooms_advanced = 0u32;
    for room in &request.other_rooms {
        other_rooms_attempted = other_rooms_attempted.saturating_add(1);
        let Ok(mut room_device) = store.load_device(request.config.clone()) else {
            continue;
        };
        let Ok(before) = room_device.last_applied_seq(&room.room_id) else {
            continue;
        };
        let tick = {
            let mut delivery = CapturedLogDelivery { log: room };
            run_room_sync_tick(
                &mut store,
                &mut room_device,
                &mut delivery,
                &options,
                &room.room_id,
            )
        };
        if tick.is_err() {
            continue;
        }
        let advanced = store
            .load_device(request.config.clone())
            .and_then(|device| device.last_applied_seq(&room.room_id).map_err(Into::into))
            .map(|after| after > before)
            .unwrap_or(false);
        if advanced {
            other_rooms_advanced = other_rooms_advanced.saturating_add(1);
        }
    }
    let later_rooms_continued =
        other_rooms_attempted > 0 && other_rooms_advanced == other_rooms_attempted;

    Ok(RejectedEntryDiagnostic {
        schema_version: REJECTED_ENTRY_DIAGNOSTIC_SCHEMA_VERSION,
        incident_alias: request.incident_alias.clone(),
        cursor_before,
        cursor_after,
        replay_outcome,
        rejected,
        error_class,
        repair_disposition,
        device_state_candidate_persisted,
        other_rooms_attempted,
        other_rooms_advanced,
        later_rooms_continued,
    })
}

fn validate_incident_alias(alias: &str) -> Result<(), RejectedEntryDiagnosticError> {
    let valid = !alias.is_empty()
        && alias.len() <= MAX_INCIDENT_ALIAS_BYTES
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'));
    if valid {
        Ok(())
    } else {
        Err(RejectedEntryDiagnosticError::InvalidIncidentAlias)
    }
}

fn validate_capture(log: &CapturedRoomLog) -> Result<(), RejectedEntryDiagnosticError> {
    let mut previous = None;
    for entry in &log.entries {
        if entry.room_id != log.room_id || entry.envelope.room_id != log.room_id {
            return Err(RejectedEntryDiagnosticError::CaptureRoomMismatch);
        }
        if let Some(previous) = previous
            && entry.seq <= previous
        {
            return Err(RejectedEntryDiagnosticError::CaptureNotOrdered);
        }
        previous = Some(entry.seq);
    }
    Ok(())
}

/// Byte-copy the source store (plus WAL sidecars, if present) into the work
/// directory. The source is only ever read through `fs::copy`, never opened
/// by sqlite, so no diagnostic operation can write it.
fn copy_store_for_replay(
    source_db_path: &Path,
    work_dir: &Path,
) -> Result<PathBuf, RejectedEntryDiagnosticError> {
    let io = |operation: &str, source: std::io::Error| RejectedEntryDiagnosticError::Io {
        reason: format!("{operation}: {source}"),
    };
    let source_canonical =
        fs::canonicalize(source_db_path).map_err(|error| io("canonicalize source", error))?;
    fs::create_dir_all(work_dir).map_err(|error| io("create work dir", error))?;
    let destination = work_dir.join(REPLAY_STORE_COPY_NAME);
    if destination.exists()
        && fs::canonicalize(&destination).map_err(|error| io("canonicalize destination", error))?
            == source_canonical
    {
        return Err(RejectedEntryDiagnosticError::SourceIsReplayCopy);
    }
    fs::copy(&source_canonical, &destination).map_err(|error| io("copy store", error))?;
    // A WAL-mode store may have sidecars holding not-yet-checkpointed
    // frames; copy them so the replay sees the same durable bytes. Remove
    // stale sidecars from earlier runs first.
    for suffix in ["-wal", "-shm"] {
        let source_sidecar = sidecar_path(&source_canonical, suffix);
        let destination_sidecar = sidecar_path(&destination, suffix);
        if destination_sidecar.exists() {
            fs::remove_file(&destination_sidecar)
                .map_err(|error| io("remove stale sidecar", error))?;
        }
        if source_sidecar.exists() {
            fs::copy(&source_sidecar, &destination_sidecar)
                .map_err(|error| io("copy sidecar", error))?;
        }
    }
    Ok(destination)
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Classify one rejected entry the way the production replay does: the
/// entry's kind (coarsened to the incident vocabulary) and the error class
/// of its failure.
pub fn classify_rejected_entry(
    kind: LogEntryKind,
    error: &ClientStoreError,
) -> (RejectedEntryKind, RejectedEntryErrorClass) {
    (
        rejected_entry_kind(kind),
        classify_store_error(Some(kind), error),
    )
}

/// The one skip rule, shared by `finitechat repair skip-entry` and the
/// rekey's pre-commit backlog replay: an entry may be skipped only when it
/// is an application entry whose rejection classifies as an MLS
/// application-ciphertext failure. Nothing else is ever skippable.
pub fn is_skippable_rejection(
    kind: RejectedEntryKind,
    error_class: RejectedEntryErrorClass,
) -> bool {
    kind == RejectedEntryKind::Application
        && error_class == RejectedEntryErrorClass::MlsApplicationCiphertext
}

fn rejected_entry_kind(kind: LogEntryKind) -> RejectedEntryKind {
    match kind {
        LogEntryKind::Application => RejectedEntryKind::Application,
        LogEntryKind::Commit => RejectedEntryKind::Commit,
        LogEntryKind::Proposal => RejectedEntryKind::Other,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn repair_disposition(
    class: RejectedEntryErrorClass,
    kind: Option<RejectedEntryKind>,
) -> RepairDisposition {
    if class == RejectedEntryErrorClass::UnsupportedUnclassified {
        return RepairDisposition::StopUnclassified;
    }
    if kind == Some(RejectedEntryKind::Commit)
        || class == RejectedEntryErrorClass::CommitProposalMembership
    {
        return RepairDisposition::NonSkippableCommit;
    }
    RepairDisposition::ClassificationOnly
}

fn classify_worker_error<E>(
    kind: Option<LogEntryKind>,
    error: &RuntimeWorkerError<E>,
) -> RejectedEntryErrorClass {
    match error {
        RuntimeWorkerError::Delivery(_) => RejectedEntryErrorClass::TransportDelivery,
        RuntimeWorkerError::Client(client) => classify_client_error(kind, client),
        RuntimeWorkerError::ClientStore(store) => classify_store_error(kind, store),
    }
}

fn classify_store_error(
    kind: Option<LogEntryKind>,
    error: &ClientStoreError,
) -> RejectedEntryErrorClass {
    match error {
        ClientStoreError::Client(client) => classify_client_error(kind, client),
        _ => RejectedEntryErrorClass::LocalEncryptedStore,
    }
}

/// The FiniteChatCoreError → class mapping. Anything not listed here is
/// `unsupported_unclassified` and stops without selecting a repair.
fn classify_client_error(
    kind: Option<LogEntryKind>,
    error: &ClientError,
) -> RejectedEntryErrorClass {
    use ClientError as E;
    match error {
        // Protocol/envelope parsing and log-entry shape validation.
        E::ProtocolLimit(_)
        | E::ParseProtocolMessage
        | E::EnvelopeMessageId(_)
        | E::MlsGroupIdNotUtf8
        | E::LogEntryRoomMismatch { .. }
        | E::LogEntryEnvelopeRoomMismatch { .. }
        | E::LogEntryKindMismatch { .. }
        | E::LogEntryEnvelopeKindMismatch { .. }
        | E::LogEntryMessageIdMismatch { .. }
        | E::LogEntrySenderMismatch
        | E::LogEntryEpochMismatch { .. } => RejectedEntryErrorClass::ProtocolEnvelopeParsing,
        // MLS epoch or local group-state mismatch: the device's durable MLS
        // state diverged from the log's expectation.
        E::ActivityEpochMismatch { .. }
        | E::UnexpectedCommitEpoch { .. }
        | E::UnexpectedPostCommitEpoch { .. }
        | E::AppliedSeqRegression { .. }
        | E::InvalidClientState(_)
        | E::GroupNotFound(_)
        | E::MissingGroupState(_)
        | E::LoadGroupState(_)
        | E::PersistedGroupIdMismatch(_) => RejectedEntryErrorClass::MlsEpochOrStateMismatch,
        // Application ciphertext construction/decryption failures.
        E::CreateApplicationMessage | E::ActivityCiphertext => {
            RejectedEntryErrorClass::MlsApplicationCiphertext
        }
        // Raw MLS message processing: the class depends on the entry kind.
        E::ProcessMessage { .. } | E::UnexpectedMessage => match kind {
            Some(LogEntryKind::Commit) => RejectedEntryErrorClass::CommitProposalMembership,
            Some(LogEntryKind::Application) => RejectedEntryErrorClass::MlsApplicationCiphertext,
            _ => RejectedEntryErrorClass::UnsupportedUnclassified,
        },
        // Commit/proposal/membership processing, including Welcome
        // activation and member credential verification.
        E::MergePendingCommit
        | E::ClearPendingCommit
        | E::MergeStagedCommit
        | E::AddMember
        | E::RemoveMember
        | E::SelfUpdate
        | E::CannotRemoveSelf
        | E::UnexpectedWelcomeForNonAddCommit
        | E::OwnCommitWithoutPendingState(_)
        | E::PendingCommitExists(_)
        | E::PendingCommitMustBeMerged(_)
        | E::MissingPendingCommit(_)
        | E::PendingCommitNotObserved(_)
        | E::MemberCredentialMissing(_)
        | E::MlsCredential(_)
        | E::ParseWelcome
        | E::StageWelcome
        | E::ActivateWelcome => RejectedEntryErrorClass::CommitProposalMembership,
        _ => RejectedEntryErrorClass::UnsupportedUnclassified,
    }
}
