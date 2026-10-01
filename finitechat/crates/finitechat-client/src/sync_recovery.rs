//! Bounded recovery to an existing, authenticated epoch transition. Rejected
//! ciphertext is retained on the server; only its binding is recorded locally.

use super::*;

const MAX_RECOVERY_SKIPS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSyncRecovery {
    pub room_id: RoomId,
    pub seq: u64,
    pub ciphertext_sha256: String,
    pub recovery_commit_seq: u64,
    pub previous_epoch: u64,
    pub recovered_epoch: u64,
}

fn restore_candidate(
    device: &FiniteChatDevice,
    state: FiniteChatDeviceState,
) -> Result<FiniteChatDevice, ClientError> {
    FiniteChatDevice::from_state_for_device(
        device.credential.account_public_key(),
        device.device_ref.device_id.clone(),
        device.now_unix_seconds,
        state,
    )
}

pub(super) fn recover_room<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
    room_id: &str,
    report: &mut RuntimeSyncReport,
) -> Result<bool, RuntimeWorkerError<D::Error>> {
    let owner = device.device_ref();
    let state = load_device_state(
        &store.conn,
        &store.options.encryption_key,
        &owner.account_id,
        &owner.device_id,
    )?
    .ok_or_else(|| ClientStoreError::DeviceStateNotFound {
        account_id: owner.account_id.clone(),
        device_id: owner.device_id.clone(),
    })?;
    let mut candidate = restore_candidate(device, state)?;
    if candidate.has_pending_commit(room_id)? {
        return Ok(false);
    }
    let mut after_seq = candidate.last_applied_seq(room_id)?;
    let mut caught_up = false;
    let mut recoveries = Vec::new();
    let mut applied_entries = Vec::new();
    let mut messages = Vec::new();
    let mut events = Vec::new();
    for _ in 0..options.max_sync_pages_per_room {
        let page = delivery
            .sync_events(room_id, candidate.device_ref(), after_seq)
            .map_err(RuntimeWorkerError::Delivery)?;
        report.record_sync_page()?;
        if page.next_after_seq < after_seq {
            return Err(ClientError::RuntimeSyncCursorRegression {
                room_id: room_id.to_owned(),
                current_seq: after_seq,
                next_after_seq: page.next_after_seq,
            }
            .into());
        }
        for entry in page.entries {
            let epoch = candidate.group_epoch(room_id)?;
            // Failed MLS processing may mutate secret-tree state. Every
            // rejected application is rolled back before continuing replay.
            let before = candidate.export_state()?;
            let applied = match apply_log_entry_in_memory(&mut candidate, room_id, &entry) {
                Ok(applied) => applied,
                Err(ClientStoreError::Client(ClientError::ApplicationGenerationUnavailable {
                    ..
                })) if entry.kind == LogEntryKind::Application
                    && entry.sender != *candidate.device_ref()
                    && entry.epoch == epoch
                    && recoveries.len() < MAX_RECOVERY_SKIPS =>
                {
                    candidate = restore_candidate(&candidate, before)?;
                    candidate.set_last_applied_seq(room_id, entry.seq)?;
                    recoveries.push(StoredSyncRecovery {
                        room_id: room_id.to_owned(),
                        seq: entry.seq,
                        ciphertext_sha256: hex_lower(&Sha256::digest(&entry.envelope.payload)),
                        recovery_commit_seq: 0,
                        previous_epoch: epoch,
                        recovered_epoch: 0,
                    });
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let Some(applied) = applied else { continue };
            let new_epoch = candidate.group_epoch(room_id)?;
            if entry.kind == LogEntryKind::Commit && new_epoch > epoch {
                for recovery in &mut recoveries {
                    if recovery.recovery_commit_seq == 0 && new_epoch > recovery.previous_epoch {
                        recovery.recovery_commit_seq = entry.seq;
                        recovery.recovered_epoch = new_epoch;
                    }
                }
            }
            if let Some(message) = stored_app_message_from_applied(
                room_id,
                entry.seq,
                &entry.message_id,
                entry.timestamp_unix_seconds,
                &applied,
            ) {
                messages.push(message);
            }
            if let Some(event) = stored_app_event_from_applied(
                room_id,
                entry.seq,
                &entry.message_id,
                entry.timestamp_unix_seconds,
                &applied,
            ) {
                events.push(event);
            }
            applied_entries.push(RuntimeAppliedEntry {
                room_id: room_id.to_owned(),
                seq: entry.seq,
                message_id: entry.message_id,
                timestamp_unix_seconds: entry.timestamp_unix_seconds,
                entry: applied,
            });
        }
        if page.next_after_seq > candidate.last_applied_seq(room_id)? {
            candidate.set_last_applied_seq(room_id, page.next_after_seq)?;
        }
        if !page.has_more {
            caught_up = true;
            break;
        }
        if page.next_after_seq == after_seq {
            return Err(ClientError::RuntimeSyncStalled {
                room_id: room_id.to_owned(),
                after_seq,
            }
            .into());
        }
        after_seq = page.next_after_seq;
    }
    if recoveries.is_empty() || recoveries.iter().any(|r| r.recovery_commit_seq == 0) {
        return Ok(false);
    }
    if caught_up {
        candidate.complete_currency_initialization(room_id)?;
        candidate.clear_behind_server_if_healed(room_id)?;
    }
    // The cursor, readable entries, and gap evidence either all commit or
    // none do. No synthetic app event is delivered to an agent as a command.
    store.save_sync_tick(&candidate, &messages, &events, &recoveries)?;
    if caught_up {
        store.note_room_currency_verified(room_id);
    }
    for verified_room in &store.currency_verified_rooms {
        candidate.mark_room_currency_verified(verified_room);
    }
    *device = candidate;
    report.applied_entries.extend(applied_entries);
    Ok(true)
}

pub(super) fn save_recoveries(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    recoveries: &[StoredSyncRecovery],
) -> Result<(), ClientStoreError> {
    for recovery in recoveries {
        tx.execute(
            "INSERT INTO client_sync_recoveries
             (account_id, device_id, room_id, seq, ciphertext_sha256,
              recovery_commit_seq, previous_epoch, recovered_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                owner.account_id,
                owner.device_id,
                recovery.room_id,
                recovery.seq,
                recovery.ciphertext_sha256,
                recovery.recovery_commit_seq,
                recovery.previous_epoch,
                recovery.recovered_epoch
            ],
        )?;
    }
    Ok(())
}

impl SqliteClientStore {
    pub fn load_sync_recoveries(
        &self,
        owner: &DeviceRef,
        room_id: &str,
    ) -> Result<Vec<StoredSyncRecovery>, ClientStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ciphertext_sha256, recovery_commit_seq, previous_epoch, recovered_epoch
             FROM client_sync_recoveries
             WHERE account_id = ?1 AND device_id = ?2 AND room_id = ?3 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![owner.account_id, owner.device_id, room_id], |row| {
            Ok(StoredSyncRecovery {
                room_id: room_id.to_owned(),
                seq: row.get(0)?,
                ciphertext_sha256: row.get(1)?,
                recovery_commit_seq: row.get(2)?,
                previous_epoch: row.get(3)?,
                recovered_epoch: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ClientStoreError::from)
    }
}
