//! The `finitechat repair` subcommand family: operator-only, fail-closed
//! repairs. Unlike `finitechat diagnose` (local copies only), `repair`
//! phase 2 writes the REAL client store; the operator procedure is to stop
//! the device service, run the repair against the live store, then restart.
//!
//! `finitechat repair skip-entry` is the only production-sanctioned way to
//! advance a durable room cursor past a rejected log entry. It never
//! accepts an operator-typed sequence: phase 1 (rehearsal) re-runs the
//! `diagnose rejected-entry` classification replay in a loop against byte
//! copies of `--store`, deriving the skip list from evidence. A replayed
//! entry is skippable only when it is attributed exactly, is
//! kind=application, and classifies as `mls_application_ciphertext`; any
//! other kind or error class stops the loop and refuses without changing
//! anything. Phase 2 (apply) runs only when the final rehearsal replay
//! advances the copied cursor to the capture head; it opens the real store
//! and advances the durable cursor once per derived skip, in ascending
//! order, through `SqliteClientStore::advance_room_cursor_and_save` (the
//! sanctioned monotonic cursor path). No entries are rewritten or deleted
//! and no other table is touched.
//!
//! Every run appends to `--audit-log` (JSONL, created mode 0600): one line
//! per skipped entry as it is applied, then a final summary line with
//! phase "apply" or "refused". Stdout carries the same privacy contract as
//! the classifier: seqs, kinds, SHA-256 bindings, error classes, cursor
//! numbers, and counts only — never identifiers, plaintext, ciphertext, or
//! secrets.

use std::io::Write;
use std::path::{Path, PathBuf};

use finitechat_client::rejected_entry_diagnostic::{
    CapturedRoomLog, CapturedRoomLogFile, RejectedEntryDiagnosticRequest, RejectedEntryErrorClass,
    RejectedEntryKind, ReplayOutcome, run_rejected_entry_diagnostic,
};
use finitechat_client::{FiniteChatDeviceConfig, SqliteClientStore, SqliteClientStoreOptions};
use serde::Serialize;

use crate::diagnose::split_capture;
use crate::{CliError, parse_account_secret, parse_u64, write_pretty_json};
use crate::{reject_extra_args, required_option, take_option, take_positional};

/// Schema version of the stdout record and the audit-log lines. Bump on
/// any field change.
const REPAIR_SKIP_ENTRY_SCHEMA_VERSION: u32 = 1;
/// Default bound on derived skips per run.
const DEFAULT_MAX_SKIPS: u32 = 16;
/// Hard cap on `--max-skips`; values above this are a usage error.
const HARD_MAX_SKIPS: u32 = 64;

pub(crate) fn run<W: Write>(mut args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(usage()));
    };
    match command.as_str() {
        "skip-entry" => cmd_skip_entry(&mut args, output),
        _ => Err(CliError::Usage(usage())),
    }
}

/// One derived skip, shared by the stdout record and the audit trail.
/// Privacy-locked: sequence, kind, and entry binding digest only.
#[derive(Debug, Clone, Serialize)]
struct DerivedSkip {
    seq: u64,
    kind: RejectedEntryKind,
    sha256: String,
    error_class: RejectedEntryErrorClass,
}

/// The offending classification behind a refusal (privacy-locked).
#[derive(Debug, Clone, Serialize)]
struct RefusedEntry {
    seq: u64,
    kind: RejectedEntryKind,
    sha256: String,
    error_class: Option<RejectedEntryErrorClass>,
}

/// The privacy-locked stdout record.
#[derive(Serialize)]
struct SkipEntryRepairRecord {
    schema_version: u32,
    incident_alias: String,
    cursor_before: u64,
    cursor_after: u64,
    rehearsal_outcome: ReplayOutcome,
    repair_disposition: &'static str,
    skipped: Vec<DerivedSkip>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refused_entry: Option<RefusedEntry>,
    max_skips: u32,
}

/// Audit trail: one line per skipped entry as it is applied to the real
/// store, then one summary line per run.
#[derive(Serialize)]
struct AuditEntryLine<'a> {
    schema_version: u32,
    incident_alias: &'a str,
    seq: u64,
    kind: RejectedEntryKind,
    entry_sha256: &'a str,
    error_class: RejectedEntryErrorClass,
    skipped_at_unix_seconds: u64,
}

#[derive(Serialize)]
struct AuditSummaryLine<'a> {
    schema_version: u32,
    incident_alias: &'a str,
    phase: &'static str,
    cursor_before: u64,
    cursor_after: u64,
    skips: usize,
}

struct AuditLog {
    file: std::fs::File,
}

impl AuditLog {
    fn open(path: &Path) -> Result<Self, CliError> {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|error| {
            CliError::Runtime(format!(
                "failed to open the audit log {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { file })
    }

    fn append<T: Serialize>(&mut self, line: &T) -> Result<(), CliError> {
        serde_json::to_writer(&mut self.file, line).map_err(CliError::Serialize)?;
        writeln!(self.file).map_err(CliError::Output)?;
        self.file.sync_data().map_err(CliError::Output)
    }
}

/// Result of phase 1: the rehearsal loop over byte copies of the store.
struct Rehearsal {
    cursor_before: u64,
    /// Outcome of the final classification replay.
    final_outcome: ReplayOutcome,
    skips: Vec<DerivedSkip>,
    refusal: Option<Refusal>,
}

struct Refusal {
    reason: &'static str,
    refused_entry: Option<RefusedEntry>,
}

fn cmd_skip_entry<W: Write>(args: &mut Vec<String>, output: &mut W) -> Result<(), CliError> {
    let store = required_option(args, "--store")?;
    let work_dir = required_option(args, "--work-dir")?;
    let room_log = required_option(args, "--room-log")?;
    let device_id = required_option(args, "--device-id")?;
    let account_secret_hex = required_option(args, "--account-secret-hex")?;
    let incident_alias = required_option(args, "--incident-alias")?;
    let audit_log = required_option(args, "--audit-log")?;
    let max_skips = take_option(args, "--max-skips")?
        .map(|value| parse_u64("--max-skips", &value))
        .transpose()?
        .map(u32::try_from)
        .transpose()
        .map_err(|_| CliError::Usage("--max-skips must fit in a u32".to_owned()))?
        .unwrap_or(DEFAULT_MAX_SKIPS);
    reject_extra_args(args)?;
    if max_skips > HARD_MAX_SKIPS {
        return Err(CliError::Usage(format!(
            "--max-skips must be at most {HARD_MAX_SKIPS}"
        )));
    }

    let store_path = PathBuf::from(store);
    let work_dir = PathBuf::from(work_dir);
    let audit_path = PathBuf::from(audit_log);
    check_audit_log_location(&store_path, &work_dir, &audit_path)?;

    let capture_bytes = std::fs::read(&room_log).map_err(|error| {
        CliError::Runtime(format!("failed to read the captured room log: {error}"))
    })?;
    let capture: CapturedRoomLogFile =
        serde_json::from_slice(&capture_bytes).map_err(CliError::Json)?;
    let (target, _other_rooms) = split_capture(capture)?;

    let now_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let config = FiniteChatDeviceConfig {
        account_secret_key: parse_account_secret(&account_secret_hex)?,
        device_id,
        now_unix_seconds,
        credential_not_before_unix_seconds: now_unix_seconds.saturating_sub(60),
        credential_not_after_unix_seconds: now_unix_seconds.saturating_add(60),
    };

    // Phase 1: rehearsal. Everything below runs against byte copies.
    let rehearsal = rehearse(
        &store_path,
        &work_dir,
        &config,
        &incident_alias,
        &target,
        max_skips,
    )?;

    let mut audit = AuditLog::open(&audit_path)?;

    if let Some(refusal) = rehearsal.refusal {
        // Fail closed: no phase 2, the real store is never opened.
        audit.append(&AuditSummaryLine {
            schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
            incident_alias: &incident_alias,
            phase: "refused",
            cursor_before: rehearsal.cursor_before,
            cursor_after: rehearsal.cursor_before,
            skips: 0,
        })?;
        return write_pretty_json(
            output,
            &SkipEntryRepairRecord {
                schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
                incident_alias,
                cursor_before: rehearsal.cursor_before,
                cursor_after: rehearsal.cursor_before,
                rehearsal_outcome: rehearsal.final_outcome,
                repair_disposition: "refused",
                skipped: rehearsal.skips,
                refusal_reason: Some(refusal.reason),
                refused_entry: refusal.refused_entry,
                max_skips,
            },
        );
    }

    if rehearsal.skips.is_empty() {
        // Already healthy: the rehearsal replay advanced (or the store was
        // already at the capture head) without any skip. The real store is
        // not written; the device converges on its own next sync.
        audit.append(&AuditSummaryLine {
            schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
            incident_alias: &incident_alias,
            phase: "apply",
            cursor_before: rehearsal.cursor_before,
            cursor_after: rehearsal.cursor_before,
            skips: 0,
        })?;
        return write_pretty_json(
            output,
            &SkipEntryRepairRecord {
                schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
                incident_alias,
                cursor_before: rehearsal.cursor_before,
                cursor_after: rehearsal.cursor_before,
                rehearsal_outcome: rehearsal.final_outcome,
                repair_disposition: "applied",
                skipped: Vec::new(),
                refusal_reason: None,
                refused_entry: None,
                max_skips,
            },
        );
    }

    // Phase 2: apply. Open the REAL store and advance the durable cursor
    // once per derived skip, in ascending order, through the sanctioned
    // monotonic cursor path. No entries are rewritten or deleted.
    let mut real_store = SqliteClientStore::open(
        &store_path,
        SqliteClientStoreOptions::from_nostr_secret(&config.account_secret_key, &config.device_id)
            .map_err(|error| {
                CliError::Runtime(format!("failed to prepare the client store: {error}"))
            })?,
    )
    .map_err(|error| CliError::Runtime(format!("failed to open the real store: {error}")))?;
    let mut device = real_store
        .load_device(config.clone())
        .map_err(|error| CliError::Runtime(format!("failed to load the device state: {error}")))?;
    let live_cursor = device
        .last_applied_seq(&target.room_id)
        .map_err(|error| CliError::Runtime(format!("failed to read the room cursor: {error}")))?;
    if live_cursor != rehearsal.cursor_before {
        // The store moved between rehearsal and apply (the device service
        // was not stopped?). Refuse before writing anything.
        audit.append(&AuditSummaryLine {
            schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
            incident_alias: &incident_alias,
            phase: "refused",
            cursor_before: rehearsal.cursor_before,
            cursor_after: live_cursor,
            skips: 0,
        })?;
        return write_pretty_json(
            output,
            &SkipEntryRepairRecord {
                schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
                incident_alias,
                cursor_before: rehearsal.cursor_before,
                cursor_after: live_cursor,
                rehearsal_outcome: rehearsal.final_outcome,
                repair_disposition: "refused",
                skipped: Vec::new(),
                refusal_reason: Some("cursor_changed_since_rehearsal"),
                refused_entry: None,
                max_skips,
            },
        );
    }

    let mut cursor_after = rehearsal.cursor_before;
    for skip in &rehearsal.skips {
        real_store
            .advance_room_cursor_and_save(&mut device, &target.room_id, skip.seq)
            .map_err(|error| {
                CliError::Runtime(format!(
                    "failed to advance the room cursor past seq {}: {error}",
                    skip.seq
                ))
            })?;
        cursor_after = skip.seq;
        audit.append(&AuditEntryLine {
            schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
            incident_alias: &incident_alias,
            seq: skip.seq,
            kind: skip.kind,
            entry_sha256: &skip.sha256,
            error_class: skip.error_class,
            skipped_at_unix_seconds: now_unix_seconds,
        })?;
    }
    audit.append(&AuditSummaryLine {
        schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
        incident_alias: &incident_alias,
        phase: "apply",
        cursor_before: rehearsal.cursor_before,
        cursor_after,
        skips: rehearsal.skips.len(),
    })?;

    write_pretty_json(
        output,
        &SkipEntryRepairRecord {
            schema_version: REPAIR_SKIP_ENTRY_SCHEMA_VERSION,
            incident_alias,
            cursor_before: rehearsal.cursor_before,
            cursor_after,
            rehearsal_outcome: rehearsal.final_outcome,
            repair_disposition: "applied",
            skipped: rehearsal.skips,
            refusal_reason: None,
            refused_entry: None,
            max_skips,
        },
    )
}

/// Phase 1: the classification-replay loop. Iteration N replays the
/// capture minus the skips derived so far against a fresh byte copy of
/// `--store`; a skippable rejection is recorded and replayed past, anything
/// else stops the loop. Nothing here writes the real store.
fn rehearse(
    store_path: &Path,
    work_dir: &Path,
    config: &FiniteChatDeviceConfig,
    incident_alias: &str,
    target: &CapturedRoomLog,
    max_skips: u32,
) -> Result<Rehearsal, CliError> {
    let mut skips: Vec<DerivedSkip> = Vec::new();
    let mut remaining = target.entries.clone();
    let mut cursor_before = None;
    let mut iteration = 0u32;
    loop {
        let request = RejectedEntryDiagnosticRequest {
            source_db_path: store_path.to_path_buf(),
            work_dir: work_dir.join(format!("rehearsal-iter-{iteration}")),
            config: config.clone(),
            incident_alias: incident_alias.to_owned(),
            target: CapturedRoomLog {
                room_id: target.room_id.clone(),
                entries: remaining.clone(),
            },
            // The repair decision depends only on the target room.
            other_rooms: Vec::new(),
        };
        let record = run_rejected_entry_diagnostic(&request)
            .map_err(|error| CliError::Runtime(format!("repair rehearsal failed: {error}")))?;
        let cursor_before = *cursor_before.get_or_insert(record.cursor_before);
        match record.replay_outcome {
            ReplayOutcome::Advanced | ReplayOutcome::Unchanged => {
                // Success only if the replay reached the capture head:
                // every captured sequence above the reached cursor must be
                // a derived skip (this covers poison entries at the tail).
                let reached = record.cursor_after;
                let covered = target
                    .entries
                    .iter()
                    .filter(|entry| entry.seq > reached)
                    .all(|entry| skips.iter().any(|skip| skip.seq == entry.seq));
                let refusal = if covered {
                    None
                } else {
                    Some(Refusal {
                        reason: "replay_did_not_reach_head",
                        refused_entry: None,
                    })
                };
                return Ok(Rehearsal {
                    cursor_before,
                    final_outcome: record.replay_outcome,
                    skips,
                    refusal,
                });
            }
            ReplayOutcome::Rejected => {
                let rejected = record.rejected;
                let skippable = rejected
                    .as_ref()
                    .map(|entry| entry.kind == RejectedEntryKind::Application)
                    .unwrap_or(false)
                    && record.error_class
                        == Some(RejectedEntryErrorClass::MlsApplicationCiphertext);
                if !skippable {
                    return Ok(Rehearsal {
                        cursor_before,
                        final_outcome: record.replay_outcome,
                        skips,
                        refusal: Some(Refusal {
                            reason: "rejected_entry_not_skippable",
                            refused_entry: rejected.map(|entry| RefusedEntry {
                                seq: entry.seq,
                                kind: entry.kind,
                                sha256: entry.sha256,
                                error_class: record.error_class,
                            }),
                        }),
                    });
                }
                let rejected = rejected.expect("skippable rejection is attributed");
                if skips.len() as u32 >= max_skips {
                    return Ok(Rehearsal {
                        cursor_before,
                        final_outcome: record.replay_outcome,
                        skips,
                        refusal: Some(Refusal {
                            reason: "max_skips_exceeded",
                            refused_entry: Some(RefusedEntry {
                                seq: rejected.seq,
                                kind: rejected.kind,
                                sha256: rejected.sha256,
                                error_class: record.error_class,
                            }),
                        }),
                    });
                }
                skips.push(DerivedSkip {
                    seq: rejected.seq,
                    kind: rejected.kind,
                    sha256: rejected.sha256,
                    error_class: RejectedEntryErrorClass::MlsApplicationCiphertext,
                });
                // Continue the replay past the rejected entry: the next
                // iteration serves the capture without it.
                remaining.retain(|entry| entry.seq != rejected.seq);
                iteration = iteration.saturating_add(1);
            }
        }
    }
}

/// Fail-fast path checks: the audit log must not live inside `--work-dir`
/// (rehearsal scratch is disposable; the audit trail is not) and must not
/// be the store itself.
fn check_audit_log_location(
    store_path: &Path,
    work_dir: &Path,
    audit_path: &Path,
) -> Result<(), CliError> {
    std::fs::create_dir_all(work_dir).map_err(|error| {
        CliError::Runtime(format!("failed to create the work directory: {error}"))
    })?;
    let work_canonical = std::fs::canonicalize(work_dir).map_err(|error| {
        CliError::Runtime(format!("failed to resolve the work directory: {error}"))
    })?;
    let store_canonical = std::fs::canonicalize(store_path)
        .map_err(|error| CliError::Runtime(format!("failed to resolve the store path: {error}")))?;
    let audit_parent = audit_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(audit_parent).map_err(|error| {
        CliError::Runtime(format!("failed to create the audit log directory: {error}"))
    })?;
    let audit_canonical = if audit_path.exists() {
        std::fs::canonicalize(audit_path)
    } else {
        std::fs::canonicalize(audit_parent)
            .map(|parent| parent.join(audit_path.file_name().unwrap_or_default()))
    }
    .map_err(|error| CliError::Runtime(format!("failed to resolve the audit log path: {error}")))?;
    if audit_canonical.starts_with(&work_canonical) {
        return Err(CliError::Usage(
            "--audit-log must not be inside --work-dir".to_owned(),
        ));
    }
    if audit_canonical == store_canonical {
        return Err(CliError::Usage(
            "--audit-log must not be the client store".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn usage() -> String {
    "repair commands (operator-only; phase 2 WRITES the real store; fail-closed):\n  finitechat repair skip-entry --store PATH --work-dir PATH --room-log PATH --device-id ID --account-secret-hex HEX --incident-alias ALIAS --audit-log PATH [--max-skips N]\n    the only sanctioned way to advance a durable room cursor past a rejected entry\n    phase 1 (rehearsal): replays the capture against byte copies of --store in a loop and DERIVES the skip list from the rejected-entry classification (no operator-typed seqs); only kind=application entries classified mls_application_ciphertext are skippable, anything else refuses and changes nothing\n    phase 2 (apply): runs only if the rehearsal replay reaches the capture head; advances the REAL store's cursor once per derived skip via the sanctioned monotonic cursor path; no entries are rewritten or deleted\n    --max-skips: bound on derived skips (default 16, hard cap 64)\n    --audit-log: append-only JSONL audit trail (created mode 0600), one line per skipped entry plus a summary line; must not be inside --work-dir\n    zero derived skips means already healthy: disposition applied, the store is not written\n    production procedure: stop the device service, run against the live store, restart the service".to_owned()
}
