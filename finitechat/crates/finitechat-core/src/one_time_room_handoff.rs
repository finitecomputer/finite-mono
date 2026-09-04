//! Temporary, operator-only cross-account Room handoff.
//!
//! This module is deliberately absent from the product action and UniFFI
//! surfaces. It exists only on an unmerged migration branch so an operator can
//! open coordinated copies of two hosted client stores, move one exact Room's
//! retained plaintext projection between them, and preserve normal MLS
//! membership and authorship. The transfer bundle is intentionally not
//! serializable: decrypted history must remain in memory.

use super::*;

const ONE_TIME_ROOM_HANDOFF_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneTimeRoomHandoffIntent {
    pub version: u16,
    pub migration_id: String,
    pub project_id: String,
    pub room_id: String,
    pub source: DeviceRef,
    pub target: DeviceRef,
    pub expected_member_account_ids: Vec<String>,
    pub expected_target_other_room_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneTimeRoomHandoffEvidence {
    pub through_seq: u64,
    pub history_event_count: u64,
    /// Legacy persisted application rows on the source. Despite the table's
    /// historical name, these include non-chat application events.
    pub source_cached_application_row_count: u64,
    /// User-visible chat messages after applying the typed projection.
    pub projected_chat_message_count: u64,
    pub chunk_count: u32,
    pub history_sha256: String,
    pub manifest_sha256: String,
}

/// Decrypted history is process-local by construction. Do not add `Serialize`
/// or `Debug`; the operator ledger records only [`OneTimeRoomHandoffEvidence`].
pub struct OneTimeRoomHandoffBundle {
    intent: OneTimeRoomHandoffIntent,
    evidence: OneTimeRoomHandoffEvidence,
    chunks: Vec<DeviceLinkBootstrapV2>,
}

impl OneTimeRoomHandoffBundle {
    pub fn evidence(&self) -> OneTimeRoomHandoffEvidence {
        self.evidence.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneTimeRoomHandoffReport {
    pub migration_id: String,
    pub project_id: String,
    pub room_id: String,
    pub source: DeviceRef,
    pub target: DeviceRef,
    pub evidence: OneTimeRoomHandoffEvidence,
    pub exact_replay: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneTimeRoomHandoffPreparedRemoval {
    pub intent: OneTimeRoomHandoffIntent,
    pub evidence: OneTimeRoomHandoffEvidence,
    pub request: SubmitCommitRequest,
    pub message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneTimeRoomHandoffFinalizeReport {
    pub migration_id: String,
    pub project_id: String,
    pub room_id: String,
    pub removed_source: DeviceRef,
    pub retained_target: DeviceRef,
    pub accepted_seq: u64,
    pub message_id: String,
    pub remaining_account_ids: Vec<String>,
}

impl AppRuntimeState {
    pub(super) fn export_one_time_room_handoff(
        &mut self,
        intent: OneTimeRoomHandoffIntent,
    ) -> Result<OneTimeRoomHandoffBundle, FiniteChatCoreError> {
        validate_handoff_intent(&intent)?;
        let owner = self.core.device.device_ref().clone();
        if owner != intent.source {
            return Err(client_error(
                "one-time Room handoff source identity does not match the open store",
            ));
        }
        self.require_exact_handoff_members(&intent)?;

        let synced = self.core.sync_room_with_projection(&intent.room_id)?;
        self.apply_targeted_sync_projection(&intent.room_id, synced)?;
        self.require_exact_handoff_members(&intent)?;
        let through_seq = self
            .core
            .device
            .last_applied_seq(&intent.room_id)
            .map_err(client_error)?;
        audit_source_history_through(self, &intent.room_id, through_seq)?;

        let room = self
            .room(&intent.room_id)
            .cloned()
            .ok_or_else(|| client_error("one-time Room handoff source projection is missing"))?;
        let member_account_ids = intent
            .expected_member_account_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let profiles = member_account_ids
            .iter()
            .map(|account_id| {
                self.profile_cache
                    .get(account_id)
                    .cloned()
                    .unwrap_or_else(|| placeholder_profile(account_id))
            })
            .map(device_link_bootstrap_profile_from_app)
            .collect::<Vec<_>>();
        let canonical_selection = self
            .canonical_agent_bootstrap_selection()
            .filter(|selection| selection.room_id == intent.room_id);
        let history = load_handoff_history(self, &intent.room_id, through_seq)?;
        let source_cached_application_row_count = self
            .core
            .store
            .load_app_messages(&owner, MAX_APP_MESSAGES_U32)
            .map_err(store_error)?
            .iter()
            .filter(|message| message.room_id == intent.room_id && message.seq <= through_seq)
            .count() as u64;
        let projected_chat_message_count = self
            .chat_projection
            .messages()
            .iter()
            .filter(|message| message.room_id == intent.room_id && message.seq <= through_seq)
            .count() as u64;
        let chunks = build_handoff_chunks(&intent, &room, canonical_selection, profiles, history)?;
        let first = chunks
            .first()
            .ok_or_else(|| client_error("one-time Room handoff produced no chunks"))?;
        let receipt =
            StoredDeviceLinkBootstrapReceipt::from_bootstrap(&owner, first).map_err(store_error)?;
        let evidence = OneTimeRoomHandoffEvidence {
            through_seq,
            history_event_count: receipt.total_history_events,
            source_cached_application_row_count,
            projected_chat_message_count,
            chunk_count: receipt.chunk_count,
            history_sha256: receipt.history_sha256.clone(),
            manifest_sha256: receipt.manifest_sha256.clone(),
        };
        Ok(OneTimeRoomHandoffBundle {
            intent,
            evidence,
            chunks,
        })
    }

    pub(super) fn import_one_time_room_handoff(
        &mut self,
        intent: OneTimeRoomHandoffIntent,
        evidence: OneTimeRoomHandoffEvidence,
        bundle: OneTimeRoomHandoffBundle,
    ) -> Result<OneTimeRoomHandoffReport, FiniteChatCoreError> {
        validate_handoff_intent(&intent)?;
        if bundle.intent != intent || bundle.evidence != evidence {
            return Err(client_error(
                "one-time Room handoff bundle does not match the recorded intent and evidence",
            ));
        }
        let owner = self.core.device.device_ref().clone();
        if owner != intent.target {
            return Err(client_error(
                "one-time Room handoff target identity does not match the open store",
            ));
        }
        self.require_target_room_isolation(&intent)?;
        self.require_exact_handoff_members(&intent)?;
        let first = bundle
            .chunks
            .first()
            .ok_or_else(|| client_error("one-time Room handoff bundle has no chunks"))?;
        let expected = StoredDeviceLinkBootstrapReceipt::from_bootstrap(&intent.source, first)
            .map_err(store_error)?;
        if !evidence_matches_receipt(&evidence, &expected) {
            return Err(client_error(
                "one-time Room handoff evidence does not match the bundle manifest",
            ));
        }

        if let Some(receipt) = self
            .core
            .store
            .load_completed_device_link_bootstrap_receipt(
                &owner,
                &intent.room_id,
                &intent.migration_id,
                &intent.source,
            )
            .map_err(store_error)?
        {
            if receipt != expected {
                return Err(client_error(
                    "one-time Room handoff replay conflicts with the committed receipt",
                ));
            }
            self.replay_room_history_into_projection(&intent.room_id)?;
            return self.handoff_report(intent, evidence, true);
        }

        let history_cutoff_seq = evidence
            .through_seq
            .checked_add(1)
            .ok_or_else(|| client_error("one-time Room handoff sequence overflow"))?;
        let mut pending = None;
        for chunk in &bundle.chunks {
            let outcome = self
                .core
                .store
                .stage_one_time_cross_account_room_handoff_chunk(
                    &owner,
                    &intent.source,
                    history_cutoff_seq,
                    chunk,
                )
                .map_err(store_error)?;
            match outcome {
                DeviceLinkBootstrapStageOutcome::Ready(ready) => pending = Some(ready),
                DeviceLinkBootstrapStageOutcome::Pending { .. }
                | DeviceLinkBootstrapStageOutcome::ExactDuplicate { .. } => {}
                DeviceLinkBootstrapStageOutcome::AlreadyCommitted(receipt)
                    if receipt == expected =>
                {
                    self.replay_room_history_into_projection(&intent.room_id)?;
                    return self.handoff_report(intent, evidence, true);
                }
                DeviceLinkBootstrapStageOutcome::AlreadyCommitted(_)
                | DeviceLinkBootstrapStageOutcome::Poisoned
                | DeviceLinkBootstrapStageOutcome::CapacityExceeded => {
                    return Err(client_error(
                        "one-time Room handoff staging did not converge to the exact manifest",
                    ));
                }
            }
        }
        let pending = pending.ok_or_else(|| {
            client_error("one-time Room handoff did not stage every required history chunk")
        })?;
        if pending.receipt != expected || !pending.is_complete() {
            return Err(client_error(
                "one-time Room handoff staged receipt is incomplete or inconsistent",
            ));
        }
        let members = self
            .core
            .device
            .room_members(&intent.room_id)
            .map_err(client_error)?
            .into_iter()
            .map(|member| member.account_id)
            .collect::<BTreeSet<_>>();
        let profiles = pending
            .profiles
            .iter()
            .filter(|profile| members.contains(&profile.account_id))
            .cloned()
            .map(app_profile_from_device_link_bootstrap)
            .map(|profile| stored_profile_from_app(&profile))
            .collect::<Vec<_>>();
        let room = StoredAppRoom {
            room_id: intent.room_id.clone(),
            display_name: pending.room.display_name.clone(),
            picture: pending.room.picture.clone(),
            state: StoredAppRoomState::Connected,
            status: "connected".to_owned(),
            local_read_seq: 0,
        };
        match self
            .core
            .store
            .commit_one_time_cross_account_room_handoff_atomically(
                &owner, &expected, &room, &profiles,
            )
            .map_err(store_error)?
        {
            DeviceLinkBootstrapCommitOutcome::Committed(receipt) if receipt == expected => {}
            DeviceLinkBootstrapCommitOutcome::AlreadyCommitted(receipt) if receipt == expected => {}
            DeviceLinkBootstrapCommitOutcome::Committed(_)
            | DeviceLinkBootstrapCommitOutcome::AlreadyCommitted(_)
            | DeviceLinkBootstrapCommitOutcome::Poisoned
            | DeviceLinkBootstrapCommitOutcome::Incomplete => {
                return Err(client_error(
                    "one-time Room handoff atomic import did not commit the exact receipt",
                ));
            }
        }

        self.upsert_room(
            &intent.room_id,
            &pending.room.display_name,
            pending.room.picture,
            AppRoomState::Connected,
            "connected",
        );
        self.replay_room_history_into_projection(&intent.room_id)?;
        for profile in pending.profiles {
            if members.contains(&profile.account_id) {
                let profile = app_profile_from_device_link_bootstrap(profile);
                self.profile_records.insert(
                    profile.account_id.clone(),
                    stored_profile_from_app(&profile).profile,
                );
                self.profile_cache
                    .insert(profile.account_id.clone(), profile);
            }
        }
        self.sync_profile_state();
        // Do not apply the source's paired-agent selection while the source
        // account is still a temporary member. The operator binds the target
        // only after verification and the later MLS removal of the source.
        self.persist_app_state()?;
        self.sync_chat_projection();
        let imported_messages = self
            .chat_projection
            .messages()
            .iter()
            .filter(|message| message.room_id == intent.room_id)
            .count() as u64;
        if imported_messages != evidence.projected_chat_message_count {
            return Err(client_error(format!(
                "one-time Room handoff imported {imported_messages} messages; expected {}",
                evidence.projected_chat_message_count
            )));
        }
        self.handoff_report(intent, evidence, false)
    }

    fn require_exact_handoff_members(
        &self,
        intent: &OneTimeRoomHandoffIntent,
    ) -> Result<(), FiniteChatCoreError> {
        if !self.core.has_room(&intent.room_id) {
            return Err(client_error(
                "one-time Room handoff Room is not available in the open store",
            ));
        }
        let actual = self
            .core
            .device
            .room_members(&intent.room_id)
            .map_err(client_error)?
            .into_iter()
            .map(|member| member.account_id)
            .collect::<BTreeSet<_>>();
        let expected = intent
            .expected_member_account_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(client_error(format!(
                "one-time Room handoff membership mismatch: expected {expected:?}, found {actual:?}"
            )));
        }
        Ok(())
    }

    fn require_target_room_isolation(
        &self,
        intent: &OneTimeRoomHandoffIntent,
    ) -> Result<(), FiniteChatCoreError> {
        let mut expected = intent
            .expected_target_other_room_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        expected.insert(intent.room_id.clone());
        let actual = self
            .app
            .rooms
            .iter()
            .map(|room| room.room_id.clone())
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(client_error(format!(
                "one-time Room handoff target Room isolation mismatch: expected {expected:?}, found {actual:?}"
            )));
        }
        Ok(())
    }

    fn handoff_report(
        &mut self,
        intent: OneTimeRoomHandoffIntent,
        evidence: OneTimeRoomHandoffEvidence,
        exact_replay: bool,
    ) -> Result<OneTimeRoomHandoffReport, FiniteChatCoreError> {
        self.app.status = "one-time Room handoff imported".to_owned();
        Ok(OneTimeRoomHandoffReport {
            migration_id: intent.migration_id,
            project_id: intent.project_id,
            room_id: intent.room_id,
            source: intent.source,
            target: intent.target,
            evidence,
            exact_replay,
        })
    }

    pub(super) fn prepare_one_time_room_handoff_source_removal(
        &mut self,
        intent: OneTimeRoomHandoffIntent,
        evidence: OneTimeRoomHandoffEvidence,
    ) -> Result<OneTimeRoomHandoffPreparedRemoval, FiniteChatCoreError> {
        self.require_committed_handoff(&intent, &evidence)?;
        self.require_exact_handoff_members(&intent)?;
        let imported_messages = self
            .chat_projection
            .messages()
            .iter()
            .filter(|message| message.room_id == intent.room_id)
            .count() as u64;
        if imported_messages != evidence.projected_chat_message_count {
            return Err(client_error(format!(
                "source removal requires {} imported messages; found {imported_messages}",
                evidence.projected_chat_message_count
            )));
        }
        let idempotency_key = handoff_source_removal_idempotency_key(&intent);
        let prepared = self
            .core
            .device
            .prepare_remove_member_commit(&intent.room_id, &intent.source, idempotency_key)
            .map_err(client_error)?;
        self.core
            .store
            .save_device_state(&self.core.device)
            .map_err(store_error)?;
        Ok(OneTimeRoomHandoffPreparedRemoval {
            intent,
            evidence,
            request: prepared.request,
            message_id: prepared.message_id,
        })
    }

    pub(super) fn submit_one_time_room_handoff_source_removal(
        &mut self,
        prepared: OneTimeRoomHandoffPreparedRemoval,
    ) -> Result<OneTimeRoomHandoffFinalizeReport, FiniteChatCoreError> {
        let intent = &prepared.intent;
        self.require_committed_handoff(intent, &prepared.evidence)?;
        self.require_exact_handoff_members(intent)?;
        validate_prepared_source_removal(&prepared, self.core.device.device_ref())?;
        if !self
            .core
            .device
            .has_pending_commit(&intent.room_id)
            .map_err(client_error)?
        {
            return Err(client_error(
                "recorded source-removal request has no matching durable pending MLS Commit",
            ));
        }
        let accepted = self
            .core
            .home_delivery()
            .submit_commit(prepared.request.clone())
            .map_err(delivery_error)?;
        if accepted.message_id != prepared.message_id {
            return Err(client_error(
                "source-removal acceptance does not match the recorded Commit",
            ));
        }
        let synced = self.core.sync_room_with_projection(&intent.room_id)?;
        self.apply_targeted_sync_projection(&intent.room_id, synced)?;
        if self
            .core
            .device
            .has_pending_commit(&intent.room_id)
            .map_err(client_error)?
        {
            return Err(client_error(
                "source-removal Commit was accepted but not durably merged",
            ));
        }
        let actual = self
            .core
            .device
            .room_members(&intent.room_id)
            .map_err(client_error)?
            .into_iter()
            .map(|member| member.account_id)
            .collect::<BTreeSet<_>>();
        let mut expected = intent
            .expected_member_account_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        expected.remove(&intent.source.account_id);
        if actual != expected {
            return Err(client_error(format!(
                "source removal left unexpected membership: expected {expected:?}, found {actual:?}"
            )));
        }
        self.pair_agent(intent.room_id.clone())?;
        self.persist_app_state()?;
        Ok(OneTimeRoomHandoffFinalizeReport {
            migration_id: intent.migration_id.clone(),
            project_id: intent.project_id.clone(),
            room_id: intent.room_id.clone(),
            removed_source: intent.source.clone(),
            retained_target: intent.target.clone(),
            accepted_seq: accepted.seq,
            message_id: accepted.message_id,
            remaining_account_ids: actual.into_iter().collect(),
        })
    }

    fn require_committed_handoff(
        &self,
        intent: &OneTimeRoomHandoffIntent,
        evidence: &OneTimeRoomHandoffEvidence,
    ) -> Result<(), FiniteChatCoreError> {
        validate_handoff_intent(intent)?;
        if self.core.device.device_ref() != &intent.target {
            return Err(client_error(
                "one-time Room handoff finalization must run from the target Device",
            ));
        }
        let receipt = self
            .core
            .store
            .load_completed_device_link_bootstrap_receipt(
                self.core.device.device_ref(),
                &intent.room_id,
                &intent.migration_id,
                &intent.source,
            )
            .map_err(store_error)?
            .ok_or_else(|| client_error("source removal requires a committed handoff receipt"))?;
        if !evidence_matches_receipt(evidence, &receipt) {
            return Err(client_error(
                "source removal evidence does not match the committed handoff receipt",
            ));
        }
        Ok(())
    }
}

fn validate_handoff_intent(intent: &OneTimeRoomHandoffIntent) -> Result<(), FiniteChatCoreError> {
    if intent.version != ONE_TIME_ROOM_HANDOFF_VERSION {
        return Err(client_error(
            "unsupported one-time Room handoff intent version",
        ));
    }
    validate_string_bytes(
        "handoff.migration_id",
        &intent.migration_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(client_error)?;
    validate_string_bytes(
        "handoff.project_id",
        &intent.project_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(client_error)?;
    finitechat_proto::validate_room_id(&intent.room_id).map_err(client_error)?;
    intent.source.validate_limits().map_err(client_error)?;
    intent.target.validate_limits().map_err(client_error)?;
    if intent.source.account_id == intent.target.account_id {
        return Err(client_error(
            "one-time Room handoff requires two different Chat accounts",
        ));
    }
    let expected = intent
        .expected_member_account_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected.len() != intent.expected_member_account_ids.len()
        || !expected.contains(&intent.source.account_id)
        || !expected.contains(&intent.target.account_id)
    {
        return Err(client_error(
            "one-time Room handoff expected membership must be unique and contain source and target",
        ));
    }
    let target_other_rooms = intent
        .expected_target_other_room_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if target_other_rooms.len() != intent.expected_target_other_room_ids.len()
        || target_other_rooms.contains(&intent.room_id)
    {
        return Err(client_error(
            "one-time Room handoff target other Rooms must be unique and exclude the canonical Room",
        ));
    }
    for room_id in target_other_rooms {
        finitechat_proto::validate_room_id(&room_id).map_err(client_error)?;
    }
    Ok(())
}

fn audit_source_history_through(
    state: &mut AppRuntimeState,
    room_id: &str,
    through_seq: u64,
) -> Result<(), FiniteChatCoreError> {
    let owner = state.core.device.device_ref().clone();
    let mut after_seq = 0;
    loop {
        let page = state
            .core
            .delivery_for(&state.core.room_server_url(room_id))
            .sync_events(room_id, &owner, after_seq)
            .map_err(delivery_error)?;
        for entry in page
            .entries
            .iter()
            .take_while(|entry| entry.seq <= through_seq)
        {
            if entry.kind == LogEntryKind::Application
                && !state
                    .core
                    .store
                    .has_app_event_identity(
                        &owner,
                        room_id,
                        entry.seq,
                        &entry.message_id,
                        &entry.sender,
                        entry.timestamp_unix_seconds,
                    )
                    .map_err(store_error)?
            {
                return Err(client_error(format!(
                    "one-time Room handoff source cannot certify local plaintext for seq {}",
                    entry.seq
                )));
            }
        }
        let reached = page.next_after_seq >= through_seq || !page.has_more;
        if reached {
            return Ok(());
        }
        if page.next_after_seq <= after_seq {
            return Err(client_error("one-time Room handoff history audit stalled"));
        }
        after_seq = page.next_after_seq;
    }
}

fn load_handoff_history(
    state: &AppRuntimeState,
    room_id: &str,
    through_seq: u64,
) -> Result<Vec<DeviceLinkBootstrapEventV2>, FiniteChatCoreError> {
    let owner = state.core.device.device_ref().clone();
    let mut after_seq = 0;
    let mut after_message_id = String::new();
    let mut history = Vec::new();
    loop {
        let page = state
            .core
            .store
            .load_app_events_for_room_page(
                &owner,
                room_id,
                through_seq,
                after_seq,
                &after_message_id,
                DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE,
            )
            .map_err(store_error)?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for event in page {
            after_seq = event.seq;
            after_message_id.clone_from(&event.message_id);
            if !is_device_link_control_event(&event.plaintext) {
                history.push(device_link_history_event_from_stored(event)?);
            }
        }
        if page_len < DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE as usize {
            break;
        }
    }
    Ok(history)
}

fn build_handoff_chunks(
    intent: &OneTimeRoomHandoffIntent,
    room: &AppRoomSummary,
    canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
    profiles: Vec<DeviceLinkBootstrapProfileV2>,
    history: Vec<DeviceLinkBootstrapEventV2>,
) -> Result<Vec<DeviceLinkBootstrapV2>, FiniteChatCoreError> {
    let room = DeviceLinkBootstrapRoomV2 {
        room_id: intent.room_id.clone(),
        display_name: room.display_name.clone(),
        picture: room.picture.clone(),
    };
    let mut partitioned = Vec::<Vec<DeviceLinkBootstrapEventV2>>::new();
    let mut current = Vec::new();
    for event in history {
        let mut sized = current.clone();
        sized.push(event.clone());
        let candidate = DeviceLinkBootstrapV2 {
            version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
            bootstrap_id: intent.migration_id.clone(),
            target: intent.target.clone(),
            chunk_index: u32::MAX,
            chunk_count: u32::MAX,
            total_history_events: u64::MAX,
            history_sha256: "0".repeat(64),
            room: room.clone(),
            canonical_selection: canonical_selection.clone(),
            profiles: profiles.clone(),
            history: sized,
        };
        if current.len() >= MAX_DEVICE_LINK_BOOTSTRAP_EVENTS as usize
            || serde_json::to_vec(&candidate).map_err(client_error)?.len()
                > MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES as usize
        {
            if current.is_empty() {
                return Err(client_error(
                    "one Room history event cannot fit a handoff chunk",
                ));
            }
            partitioned.push(std::mem::take(&mut current));
        }
        current.push(event);
    }
    if !current.is_empty() {
        partitioned.push(current);
    }
    if partitioned.is_empty() {
        partitioned.push(Vec::new());
    }
    if partitioned.len() > MAX_DEVICE_LINK_BOOTSTRAP_CHUNKS as usize {
        return Err(client_error(
            "one-time Room handoff exceeds the chunk limit",
        ));
    }
    let total_history_events = partitioned.iter().map(Vec::len).sum::<usize>() as u64;
    let history_sha256 = {
        let mut digest = Sha256::new();
        for chunk in &partitioned {
            digest.update(device_link_bootstrap_chunk_sha256(chunk));
        }
        hex::encode(digest.finalize())
    };
    let chunk_count = partitioned.len() as u32;
    partitioned
        .into_iter()
        .enumerate()
        .map(|(chunk_index, history)| {
            let chunk = DeviceLinkBootstrapV2 {
                version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
                bootstrap_id: intent.migration_id.clone(),
                target: intent.target.clone(),
                chunk_index: chunk_index as u32,
                chunk_count,
                total_history_events,
                history_sha256: history_sha256.clone(),
                room: room.clone(),
                canonical_selection: canonical_selection.clone(),
                profiles: profiles.clone(),
                history,
            };
            chunk.validate_limits().map_err(client_error)?;
            Ok(chunk)
        })
        .collect()
}

fn evidence_matches_receipt(
    evidence: &OneTimeRoomHandoffEvidence,
    receipt: &StoredDeviceLinkBootstrapReceipt,
) -> bool {
    evidence.history_event_count == receipt.total_history_events
        && evidence.chunk_count == receipt.chunk_count
        && evidence.history_sha256 == receipt.history_sha256
        && evidence.manifest_sha256 == receipt.manifest_sha256
}

fn handoff_source_removal_idempotency_key(intent: &OneTimeRoomHandoffIntent) -> String {
    let mut digest = Sha256::new();
    digest.update(b"finitechat.one-time-room-handoff.remove-source.v1");
    update_device_link_digest_field(&mut digest, intent.migration_id.as_bytes());
    update_device_link_digest_field(&mut digest, intent.project_id.as_bytes());
    update_device_link_digest_field(&mut digest, intent.room_id.as_bytes());
    update_device_link_digest_field(&mut digest, intent.source.account_id.as_bytes());
    update_device_link_digest_field(&mut digest, intent.source.device_id.as_bytes());
    format!("oth-rm-{}", hex::encode(digest.finalize()))
}

fn validate_prepared_source_removal(
    prepared: &OneTimeRoomHandoffPreparedRemoval,
    owner: &DeviceRef,
) -> Result<(), FiniteChatCoreError> {
    prepared.request.validate_limits().map_err(client_error)?;
    let request = &prepared.request;
    let intent = &prepared.intent;
    let envelope_message_id = request.envelope.message_id().map_err(client_error)?;
    if request.room_id != intent.room_id
        || request.sender != *owner
        || request.envelope.room_id != intent.room_id
        || request.envelope.sender != *owner
        || request.membership_delta.commit_message_id != prepared.message_id
        || envelope_message_id != prepared.message_id
        || !request.membership_delta.adds.is_empty()
        || request.membership_delta.removes.len() != 1
        || request.membership_delta.removes[0].device != intent.source
        || request.idempotency_key != handoff_source_removal_idempotency_key(intent)
    {
        return Err(client_error(
            "recorded source-removal request is inconsistent with the handoff intent",
        ));
    }
    Ok(())
}
