# Scenario Coverage

Status: fake-MLS reducer scenarios, SQLite restart scenarios, OpenMLS
credential proof, and the reusable OpenMLS client proof passing.

Run:

```sh
cargo test -p finitechat-sim --test scenarios
cargo test -p finitechat-store --test sqlite_scenarios
cargo test -p finitechat-mls
cargo test -p finitechat-client
```

## Proven Scenarios

Each item below has a named test in
`crates/finitechat-sim/tests/scenarios.rs`.

- `create_dm_room_and_release_welcome_after_commit`
- `key_package_claim_returns_opaque_payload`
- `server_routes_key_packages_without_becoming_identity_authority`
- `account_key_package_claim_returns_one_available_package_per_device`
- `revoked_device_cannot_replenish_or_claim_key_packages`
- `key_package_inventory_is_bounded_and_consumed_packages_free_space`
- `revoked_device_cannot_claim_or_activate_pending_welcome`
- `revoked_active_device_cannot_send_or_commit`
- `welcome_activation_makes_new_device_active`
- `add_commit_requires_staged_welcome_bytes_before_mutation`
- `duplicate_commit_retry_returns_same_result_after_side_effects`
- `conflicting_idempotency_key_rejects_without_side_effects`
- `same_epoch_loser_restart_retry_replays_rejection`
- `welcome_is_not_released_before_accepted_commit`
- `key_package_lease_expiry_returns_package_to_available`
- `consumed_key_package_cannot_be_reused`
- `stale_key_package_ref_is_rejected_without_side_effects`
- `invalid_commit_report_fails_closed`
- `membership_delta_disagreement_enters_needs_repair`
- `false_remove_delta_does_not_block_removed_device_from_validating_removal_seq`
- `new_device_linking_partial_failure_retries_only_failed_room`
- `link_mailbox_payload_is_opaque_to_server_state`
- `link_session_duplicate_conflict_expiry_and_delivery_rules`
- `push_wake_is_only_a_hint_and_does_not_advance_client_state`
- `stale_push_for_removed_device_cannot_authorize_new_events`
- `accepted_commit_response_lost_then_server_restart_replays_same_result`
- `commit_durable_before_welcome_release_restart_releases_exactly_once`
- `commit_effects_are_atomic_at_reducer_boundary`
- `welcome_claim_crash_before_ack_can_resume_after_restart`
- `delayed_welcome_after_later_entries_syncs_forward_from_commit_seq`
- `welcome_terminal_failure_keeps_membership_interval_inactive`
- `fetch_then_stream_gap_is_repaired_by_pull_cursor`
- `stable_message_id_survives_retry_and_distinguishes_payloads`
- `membership_delta_structural_matrix_rejects_bad_shapes`
- `direct_room_create_or_get_and_third_account_rejection`
- `fake_device_credential_validation_rejects_wrong_bindings`
- `fake_welcome_missing_ratchet_tree_fails_activation`
- `login_challenge_replay_rules_are_single_use`
- `local_pending_commit_is_not_merged_until_server_log_observed`
- `fake_changed_leaf_credential_validation_uses_same_device_binding_rules`
- `link_fanout_existing_device_stale_isolated_to_failed_room`
- `oversized_application_payload_is_rejected_without_log_entry`
- `sync_events_returns_bounded_page`
- `duplicate_message_id_with_new_idempotency_key_is_rejected`
- `idempotency_capacity_rejects_new_mutations_but_allows_replay`
- `direct_room_rejects_too_many_devices_for_one_account`
- `multi_device_pending_invite_action_order_fuzz_keeps_server_roles_separate`
- `chat_receipt_is_durable_but_push_never`
- `runtime_state_snapshot_is_durable_but_push_never`
- `runtime_command_request_creates_command_inbox_work`
- `conversation_segment_start_is_durable_but_push_never`
- `ephemeral_activity_never_enqueues_push_or_advances_sequence`
- `ephemeral_activity_rejects_pending_unacked_device`
- `ephemeral_activity_rejects_removed_or_revoked_device`
- `ephemeral_activity_expiry_is_bounded`
- `server_activity_cache_enforces_per_route_limit_without_seq_gap`

## Meaning Of Fake-MLS

These tests prove protocol ordering, idempotency, delivery, and state-machine
rules before real MLS is wired in. They intentionally do not prove OpenMLS
cryptographic correctness.

Some OpenMLS-specific scenarios are represented as fake validation gates for
now:

- device credential validation;
- changed LeafNode credential validation;
- Welcome activation requiring ratchet-tree material;
- local pending Commit merge only after server-log observation.

Those gates become real OpenMLS tests in Phase 2.

## OpenMLS Credential Proof

The first real OpenMLS-facing tests live in `crates/finitechat-mls/src/lib.rs`.

Proven credential scenarios:

- `nostr_signed_device_credential_verifies`
- `nostr_secret_derivation_is_stable_and_domain_separated`
- `nostr_secret_derivation_rejects_unbounded_input`
- `wrong_account_key_rejects`
- `wrong_device_id_rejects`
- `wrong_mls_leaf_key_rejects`
- `tampered_signature_payload_rejects`
- `expired_credential_rejects`
- `not_yet_valid_credential_rejects`
- `invalid_sizes_reject_before_signing`
- `openmls_basic_credential_round_trips_finite_identity_bytes`
- `openmls_key_package_carries_nostr_rooted_device_credential`
- `openmls_welcome_adds_device_after_server_ordered_commit_merge`
- `openmls_welcome_without_ratchet_tree_material_rejects`

## OpenMLS Client Proof

The production-shaped client tests live in
`crates/finitechat-client/tests/client_state.rs`.

Proven client scenarios:

- `client_state_machine_adds_device_and_decrypts_application_message`
- `multi_device_invite_late_joiner_catches_up_to_new_messages`
- `multi_device_real_mls_ordering_matrix_validates_late_catch_up`
- `sqlite_client_state_survives_restart_for_late_multi_device_catch_up`
- `sqlite_client_store_encrypts_state_and_rejects_wrong_or_tampered_key_material`
- `sqlite_client_welcome_activation_is_durable_before_server_ack`
- `sqlite_client_claimed_welcome_survives_restart_before_activation`
- `sqlite_client_failed_pending_welcome_activation_keeps_inbox_entry`
- `sqlite_client_apply_log_entry_persists_cursor_and_skips_replay_after_restart`
- `client_processes_remote_add_commit_before_epoch_two_messages`
- `client_processes_remote_update_commit_before_epoch_three_messages`
- `client_processes_remote_remove_commit_before_post_remove_messages`
- `stale_removed_device_can_process_removal_but_not_future_ciphertext`
- `client_recovers_losing_same_epoch_add_commit_and_retries`
- `client_recovers_losing_same_epoch_update_commit_and_retries`
- `client_recovers_losing_same_epoch_remove_commit_and_retries`
- `client_drops_losing_pending_commit_when_winning_race_removes_it`
- `client_key_package_replenishment_edges_use_real_packages`
- `client_key_package_replenishment_plan_maintains_bounded_inventory`
- `runtime_sync_tick_replenishes_welcomes_acks_and_syncs_after_restart`
- `runtime_sync_tick_retries_key_package_upload_after_response_loss`
- `new_device_history_policy_starts_at_add_commit_not_prior_messages`
- `client_links_new_device_into_existing_rooms_with_distinct_key_packages`
- `sqlite_link_fanout_worker_survives_restart_after_prepared_commit`
- `runtime_link_fanout_tick_links_later_device_after_submit_response_loss`
- `runtime_link_fanout_tick_reprepares_after_same_epoch_loss`
- `client_link_fanout_rejects_wrong_claim_before_pending_commit`
- `client_rejects_tampered_remote_commit_without_epoch_advance`
- `client_refuses_to_merge_pending_commit_before_server_observation`
- `client_rejects_invalid_invite_request_before_local_pending_commit`
- `client_rejects_tampered_ratchet_tree_before_ack`

This proves the identity refinement from the protocol docs: OpenMLS carries the
credential bytes, but Finite Chat clients verify the Nostr-rooted account,
device id, and MLS leaf signing key locally. The server can order room entries
without deciding who a device is.

## Application Policy Proof

The protocol crate now owns default delivery policies for generic durable app
kinds. Proven unit scenarios:

- `durable_app_event_defaults_match_push_and_inbox_policy`
- `runtime_state_projection_replaces_by_revision_and_sequence`
- `runtime_state_projection_preserves_unknown_schema_and_expiry`
- `runtime_command_request_validates_kind_body_and_target_policy`
- `runtime_command_result_requires_terminal_shape_and_bounded_clears`
- `runtime_command_ledger_records_after_decrypted_target_policy`
- `activity_projection_keeps_devices_separate_and_clear_scoped`
- `activity_refresh_extends_matching_device_expiry`
- `long_running_agent_activity_uses_command_or_run_id`
- `long_running_agent_activity_survives_refresh_without_push`
- `durable_terminal_clear_is_sender_and_activity_scoped`
- `runtime_command_result_clears_matching_activity`
- `runtime_command_result_clear_rejects_invalid_result_before_mutation`
- `activity_projection_expires_and_rejects_bad_lease_windows`
- `topic_message_routes_by_conversation_id`
- `first_message_lazily_materializes_missing_conversation`
- `topic_create_is_conversation_create_with_topic_metadata`
- `telegram_thread_id_imports_to_topic_conversation_id`
- `topic_skill_binding_is_encrypted_conversation_metadata`
- `conversation_metadata_rejects_missing_conversation_id_or_bad_payload`
- `new_command_inside_topic_starts_segment_not_conversation`
- `segment_boundary_rejects_missing_conversation_id_or_bad_payload`
- `archiving_topic_does_not_archive_sibling_topic`
- `hosted_web_mode_is_not_labeled_e2ee`
- `product_client_kinds_have_explicit_secret_locations`
- `native_and_electron_modes_keep_device_secrets_on_user_device`
- `runtime_device_keeps_device_secret_on_runtime_host`
- `hosted_web_bridge_is_not_a_local_device_e2ee_surface`
- `local_daemon_mode_keeps_device_secrets_local`
- `old_plaintext_chats_render_as_read_only_archive`
- `runtime_bridge_state_projection_is_scoped_by_room_source_device_and_key`

The in-memory and SQLite stores now record delivery effects separately from
opaque MLS payload bytes. This proves that Finite Chat can enforce
push/unread/command-inbox behavior without requiring the room server to decrypt
semantic app payloads.

SQLite parity scenarios:

- `sqlite_chat_receipt_is_durable_but_push_never_after_reopen`
- `sqlite_runtime_state_snapshot_does_not_create_unread_or_inbox_work`
- `sqlite_runtime_command_request_creates_command_inbox_work_after_reopen`
- `sqlite_push_outbox_rows_are_durable_and_idempotent_after_reopen`
- `sqlite_ephemeral_activity_does_not_persist_or_advance_sequence`
- `sqlite_ephemeral_activity_rejects_pending_and_removed_devices`

## Daemon Survival Proof

The first daemon survival harness lives in
`crates/finitechat-sim/tests/daemon_survival.rs`. It uses fake runtime health,
but the real Delivery Service reducer and app-event policies.

Proven survival scenarios:

- `daemon_starts_when_hermes_is_absent_and_restarts_gateway`
- `hermes_hang_does_not_block_room_sync_or_state_snapshot`
- `runtime_state_command_result_publishes_post_mutation_snapshot`
- `daemon_publishes_gateway_down_snapshot_without_hermes`
- `attachment_download_does_not_depend_on_hermes_gateway`
- `daemon_publishes_inference_degraded_snapshot_without_agent_reply`
- `inference_timeout_preserves_user_message_and_clears_activity`
- `dashboard_reads_stale_snapshot_while_heartbeat_is_fresh`
- `hermes_invalid_output_marks_gateway_degraded_without_projection_corruption`
- `gateway_restart_success_publishes_result_and_snapshot`
- `gateway_restart_failure_publishes_terminal_result_without_retry_storm`
- `runtime_stream_callback_only_triggers_sync`
- `sse_hint_during_hermes_down_only_triggers_pull_sync`
- `daemon_restart_while_gateway_down_preserves_mls_and_cursors`
- `broken_gateway_poll_does_not_block_keypackage_replenishment`
- `broken_gateway_poll_does_not_block_welcome_ack`
- `command_ledger_survives_restart_after_request_before_execution`
- `command_ledger_survives_restart_after_execution_before_result`
- `runtime_state_snapshot_after_command_result_retries_idempotently`
- `survival_fuzzer_keeps_sync_status_and_command_ledger_bounded`

The survival harness now uses the shared `RuntimeCommandLedger` and typed
runtime command payloads instead of a test-local JSON parser. This keeps the
recovery proof tied to the production protocol shape: target policy is
decrypted, command bodies are schema-tagged and bounded, conflicting request id
reuse fails, workers execute only after a durable ledger record exists, and
terminal results record the accepted result message id and sequence instead of
flipping an untracked status flag.
The deterministic survival fuzzer mixes user messages, restart commands,
gateway state changes, daemon restarts, and crash-after-ledger-write points.
It asserts cursor monotonicity, bounded command ledger state, non-notifying
snapshots, and eventual recovery to no pending commands.

## Hermes Adapter Proof

The Hermes CLI/plugin contract is exercised in
`crates/finitechat-cli/tests/hermes_flow.rs`. The current adapter boundary now
has a named protocol regression:

- `hermes_poll_recovers_messages_already_applied_by_runtime_sync`: after Hermes
  has processed and acked an inbound user message, later messages are synced by a
  separate Rust app runtime before the Hermes bridge polls. `finitechat hermes
  poll` must still recover those messages from durable `client_app_events`, keep
  ordered-log sequence order, redeliver until ack, and stop replaying after ack.

This is the adapter-level version of the v1 transport invariant: streams,
pushes, and sidecar callbacks are hints; durable ordered sync plus a local
consumer cursor is the only consistency boundary.

Checkpoint test signal:

- The fake-MLS pending-Commit rule mapped cleanly to OpenMLS: `add_members`
  leaves a pending local Commit and does not advance the sender epoch until
  `merge_pending_commit`.
- The first real Welcome test exposed an OpenMLS storage behavior the fake
  reducer could not model: trying to stage a Welcome without ratchet-tree
  material can consume the local KeyPackage before failure. Production clients
  should persist the Welcome and wait for tree material before invoking OpenMLS
  Welcome staging.
- The credential tests were still relevant: no Nostr binding code changed when
  the OpenMLS provider/signer boundary was added.
- The first engine-through-MLS test caught a ratchet-tree timing mistake:
  exporting Alice's tree before the server-observed Commit merge produced
  OpenMLS `TreeHashMismatch`. The correct production rule is stricter than the
  fake reducer could express: publish or serve ratchet-tree material from the
  accepted post-Commit group state.
- The Welcome payload checkpoint closed the exposed server gap: `submit_commit`
  now requires staged Welcome and ratchet-tree bytes for every add, the engine
  and SQLite store return those exact bytes on claim, and the real OpenMLS test
  stages Bob from server-delivered bytes instead of a test-harness side channel.
- The real MLS proof also found the right OpenMLS API shape: Alice exports the
  post-Commit ratchet tree from the pending commit without merging local state,
  so the client can submit bytes to the server while still waiting for ordered
  Commit acceptance before `merge_pending_commit`.
- The client checkpoint removed the raw engine/OpenMLS harness and moved that
  behavior into `finitechat-client`: KeyPackage bytes are claimed from server
  storage, Welcome bytes are claimed from server storage, Alice refuses app
  sends while a local Commit is pending, and Bob decrypts a finitecomputer-style
  JSON command after acking the Welcome.
- The multi-device checkpoint confirmed the earlier interval model was the
  right one: devices added by an accepted Commit can sync entries after that
  Commit even before they ack their Welcome, while the server still rejects
  sends until each device's Welcome is acked. The real OpenMLS test then proved
  a late Alice device can activate its batch Welcome and decrypt messages sent
  before it joined locally.
- The heavy real-MLS matrix replays that same invariant across all activation
  orders for three Alice devices and several Bob-message timing patterns. It
  stays in the normal test suite because it runs quickly enough to catch MLS
  ordering regressions before they reach integration work.
- The first client SQLite restart proof persists OpenMLS storage rows, the
  device profile, and room mappings. It reloads Bob before sending, reloads
  Alice browser after activation, and reloads a late Alice phone before it
  decrypts messages sent while it was pending.
- The encrypted client-store checkpoint replaces those raw local tables with a
  Nostr-derived encrypted snapshot. The restart proof still passes, and the new
  negative test checks that legacy cleartext tables are absent, sampled raw
  credential/OpenMLS bytes are not stored in the ciphertext, the wrong derived
  key cannot load the device, and tampering fails closed.
- The crash-resume checkpoint moves applied room cursors into the encrypted
  client snapshot and adds store-backed operations for Welcome activation and
  ordered-log apply. Bob can activate and persist a Welcome before server ack,
  restart, then ack and decrypt future messages. Bob can also process a remote
  Commit and an application message through the store, restart with the cursor
  already advanced, and skip replayed entries without asking OpenMLS to process
  an already-applied epoch/message again.
- The pending-Welcome checkpoint covers the remaining claim/activation crash
  window: after Bob claims a Welcome, the server no longer returns it from
  `claim_welcomes`; Bob persists the Welcome payload and ratchet tree in the
  encrypted client snapshot, restarts, activates from local state, clears the
  pending inbox entry, and then acks the server. Activated Welcomes now leave
  durable pending-ack state until the server ack succeeds; server ack is
  idempotent, so a crash after ack but before clearing local ack state can retry.
  A companion failure test corrupts the stored ratchet tree and proves OpenMLS
  rejection does not drop the only local pending-Welcome copy.
- The first remote Commit checkpoint adds a real ordered-log client API:
  application entries decrypt, own Commit entries merge only with pending local
  state, and remote Commit entries validate the log envelope before processing
  the OpenMLS staged Commit. The valid test advances Bob from epoch 1 to epoch
  2 after Alice adds Charlie, and the invalid test rejects tampered Commit bytes
  without advancing Bob's epoch.
- The remove/update checkpoint extends that same API instead of adding a second
  path: clients can now produce real OpenMLS self-update and remove Commits,
  submit empty-delta update Commits or remove deltas to the server, merge their
  own ordered Commit, and process another device's ordered Commit before
  accepting post-epoch messages. The remove proof also checks that the removed
  device can process its removal Commit, then cannot send locally or receive
  post-remove server events.
- The later-device-link checkpoint proves the thick client responsibility across
  more than one room: Alice has two existing rooms, a newly linked Alice phone
  uploads distinct KeyPackages for each room, existing room members add that
  phone with separate accepted Commits, and the phone activates both Welcomes
  before decrypting post-link messages in both rooms. This keeps KeyPackage
  single-use behavior visible instead of hiding it behind UI orchestration.
- The fanout-discovery checkpoint adds the server-side shape that a durable
  link worker needs next: account-room discovery is paged, includes
  current/pending devices for the account, survives SQLite reopen, and duplicate
  current/pending device adds are rejected before a retry can consume a leased
  KeyPackage or release another Welcome. The same checkpoint also makes the
  group-room devices-per-account cap executable, while direct rooms keep their
  tighter cap.
- The durable link-fanout worker checkpoint closes the client crash boundary
  around local pending Commits: Alice queues room plans from account discovery,
  prepares an add Commit for one room, persists the prepared server request with
  encrypted MLS state, restarts before submit, submits the recovered request,
  completes from the ordered log, and repeats for a second room before the new
  Alice device activates both Welcomes. A negative test passes a KeyPackage
  claim for the wrong target and proves no local pending Commit is created.
- The runtime link-fanout checkpoint moves that sequence behind the API
  finitecomputer should drive. The target device replenishes real MLS
  KeyPackages through the runtime sync tick; Alice's existing device starts a
  fanout, pages account rooms with bounded one-room discovery, claims one
  target-device KeyPackage per room, persists each claimed package with the
  encrypted fanout plan, prepares both add Commits, loses the first submit
  response after the server accepted it, restarts from stored prepared Commits,
  retries idempotently, completes both rooms from the ordered log, and then the
  target device claims and activates both Welcomes through the normal runtime
  sync tick. The first version of this proof exposed a cursor/MLS mismatch in
  the setup: Alice had already merged the setup Commit but the encrypted cursor
  still pointed at zero, so the worker tried to process an old epoch. The test
  now explicitly persists the setup cursors before starting fanout.
- The same-epoch runtime fanout proof closes the retry hole around prepared
  link adds. Alice prepares a later-device add, submit fails before reaching
  the server, Bob wins the epoch with a self-update, Alice processes Bob's
  ordered Commit and clears her losing pending Commit, then the fanout worker
  reuses the still-leased claimed KeyPackage from encrypted fanout state,
  prepares a fresh add at the new epoch, submits it, completes from the ordered
  log, and the target device activates its Welcome.
- The revocation checkpoint clones Charlie's client state before removal to
  model a stale/lost device. After Bob removes Charlie, that stale client can
  fetch and process the removal Commit, but the server rejects its old-epoch
  send, rejects a forged new-epoch send as inactive, withholds post-remove log
  entries, and OpenMLS rejects a leaked post-remove ciphertext.
- The durable device-status checkpoint adds the server-side revocation ledger
  that room MLS removal needs around it. Revoked devices cannot replenish or
  claim KeyPackages, cannot claim or activate pending Welcomes, cannot send
  application events, and cannot submit Commits. SQLite proves the status
  survives reopen.
- The same-epoch recovery checkpoint creates two real local pending Commits at
  epoch 1. Alice's add wins, Bob's add loses with `WrongEpoch`, Bob keeps local
  pending state until he observes Alice's ordered Commit, then `apply_log_entry`
  clears the loser, processes the winner, and lets Bob retry at epoch 2. The
  retry reuses the still-leased Dana KeyPackage because the rejected Commit did
  not consume it or release a Welcome.
- The broader same-epoch recovery checkpoint keeps that same branch under real
  OpenMLS for non-add operations. An update loser retries after an update
  winner, a remove loser retries after an update winner, and a device whose
  pending update lost because it was removed clears pending state, cannot retry,
  cannot send locally or through the server, and stops receiving future entries.
- The KeyPackage replenishment checkpoint uses real OpenMLS package bytes for
  the client boundary: exact duplicate upload retry is idempotent, conflicting
  duplicate upload is rejected, account claim exhaustion returns no packages,
  uploading a fresh package replenishes availability, and lease expiry makes the
  original package reclaimable. The client planner now takes server inventory,
  generates only the missing upload requests needed to reach a target, auto-ids
  packages from their MLS payload hash, persists pending upload requests in
  encrypted client state, and refuses over-cap targets. The runtime tick saves
  local OpenMLS state plus replayable pending uploads before upload so a
  server-visible KeyPackage is not missing its local private state after
  restart. Sim and SQLite prove the server cap counts available plus leased
  packages, accepted add Commits free consumed package space, and cap behavior
  survives reopen. The response-loss runtime test proved the earlier
  save-before-upload rule was incomplete by itself: after the server accepted
  one upload and the client crashed before local clear, restart retried the
  exact pending upload idempotently and did not generate extra local packages.
- The runtime sync checkpoint exposed a gap in the earlier crash proof: we had
  durable activation before server ack, but no durable marker telling the
  automated runtime loop to send that ack after restart. The fix adds
  `pending_welcome_acks` to encrypted client state and makes server ack retry
  safe.
- The history-policy checkpoint makes the v1 product decision executable:
  Alice's newly linked phone syncs from cursor zero, but the server only returns
  entries from the accepted add Commit forward. The phone decrypts the
  post-invite message and never receives Bob's pre-invite room-log message.
- Existing server tests mattered again here: the first version of the remote
  add proof accidentally used a direct room for a third-account add, and
  `DirectRoomThirdAccount` failed the scenario before it could become false
  confidence. The proof now uses a group room while direct-room limits remain
  covered separately.
- The survival fuzzer found a bug in the fake daemon harness itself: the crash
  path advanced the cursor to the whole page after recording one command,
  which could skip later commands in the same sync page. The harness now
  advances after each interpreted entry, matching the production state-machine
  invariant we want.

## SQLite Follow-Up

The SQLite suite lives in
`crates/finitechat-store/tests/sqlite_scenarios.rs`.

Proven SQLite restart scenarios:

- `sqlite_create_dm_room_and_release_welcome_after_commit`
- `sqlite_key_package_payload_survives_reopen_and_claim`
- `sqlite_duplicate_key_package_upload_is_rejected_after_reopen`
- `sqlite_account_key_package_claim_survives_reopen`
- `sqlite_key_package_inventory_cap_survives_reopen_and_consumed_frees_space`
- `sqlite_revoked_device_status_survives_reopen_and_blocks_key_packages`
- `sqlite_claimed_welcome_payload_survives_reopen`
- `sqlite_revoked_device_blocks_welcome_activation_and_sends_after_reopen`
- `sqlite_add_commit_requires_staged_welcome_bytes_before_mutation`
- `sqlite_duplicate_commit_retry_after_reopen_returns_same_result`
- `sqlite_rejected_commit_is_replayable_after_reopen`
- `sqlite_conflicting_idempotency_key_has_no_side_effects`
- `sqlite_welcome_not_released_before_accepted_commit`
- `sqlite_key_package_lease_expiry_and_reclaim_survives_reopen`
- `sqlite_consumed_key_package_cannot_be_reused`
- `sqlite_removed_device_can_sync_through_removal_after_reopen`
- `sqlite_invalid_commit_report_blocks_room_after_reopen`
- `sqlite_welcome_claim_crash_before_ack_resumes_after_reopen`
- `sqlite_delayed_welcome_syncs_forward_from_commit_seq`
- `sqlite_terminal_welcome_failure_keeps_interval_inactive`
- `sqlite_link_session_state_machine_survives_reopen`
- `sqlite_account_room_discovery_pages_after_reopen`
- `sqlite_duplicate_pending_device_add_is_rejected_before_side_effects`
- `sqlite_direct_room_create_or_get_and_third_account_rejection`
- `sqlite_oversized_application_payload_is_rejected_without_persisting_log`
- `sqlite_sync_events_returns_bounded_page_after_reopen`
- `sqlite_duplicate_message_id_is_typed_engine_error`
- `sqlite_link_payload_limit_is_rejected`
- `sqlite_idempotency_capacity_rejects_new_mutations_but_allows_replay`
- `sqlite_operation_fuzz_matches_in_memory_delivery_service`
- `sqlite_commit_crash_matrix_rolls_back_and_retry_converges`
- `sqlite_commit_epoch_unique_index_blocks_second_commit_row`

The SQLite crash matrix injects transaction rollbacks after log append, room
head update, removed membership update, added membership insert, KeyPackage
consumption, Welcome release, and idempotency record insert. It then reopens the
store, retries the same Commit, and verifies convergence to one log entry, one
epoch advance, correct membership intervals, consumed KeyPackages, released
Welcomes, and a replayable idempotency result.

The SQLite operation fuzzer applies the same deterministic sequence to the
in-memory reducer and SQLite store, then compares room, device, KeyPackage,
KeyPackage inventory, and Welcome state after every operation. It mixes
register/revoke, upload/claim, account claim, lease expiry, Welcome claim/ack,
app events, add/remove Commits, stale epochs, and exact idempotent retries. The
first version caught a real reducer/store drift: explicit claim of a leased
KeyPackage owned by a revoked device returned `KeyPackageUnavailable` in memory
but `DeviceRevoked` in SQLite. The store now matches the reducer ordering.

Push outbox rows now have their own durable SQLite table and idempotent replay
coverage. The next crash-matrix expansion should add a failure point after
outbox enqueue and assert exactly one durable wake record.

## Activity Scenarios

The Pika typing-indicator behavior should become explicit Finite Chat protocol
coverage before the room server owns push fanout:

- `ephemeral_activity_never_enqueues_push_or_advances_sequence`
- `ephemeral_activity_rejects_pending_unacked_device`
- `ephemeral_activity_rejects_removed_or_revoked_device`
- `ephemeral_activity_rejects_non_member_device`
- `ephemeral_activity_expiry_is_bounded`
- `ephemeral_activity_payload_is_opaque_to_server`
- `ephemeral_activity_epoch_mismatch_drops_without_repair`
- `server_activity_cache_enforces_per_route_limit_without_seq_gap`
- `activity_projection_keeps_devices_separate_and_clear_scoped`
- `activity_refresh_extends_matching_device_expiry`
- `durable_terminal_clear_is_sender_and_activity_scoped`
- `conversation_id_does_not_authorize_cross_room_activity`
- `activity_clear_does_not_remove_unrelated_kind`
- `activity_clear_does_not_remove_different_activity_id`
- `stale_agent_activity_clear_does_not_hide_newer_run`
- `reserved_activity_kinds_render_generically`
- `unknown_namespaced_activity_kind_is_preserved`
- `app_specific_activity_kind_does_not_trigger_generic_ui`
- `present_without_conversation_id_is_room_scoped`
- `present_with_conversation_id_is_conversation_scoped`
- `activity_default_expiry_guidance_stays_within_v1_cap`
- `activity_projection_rolls_up_identity_for_normal_ui`
- `durable_chat_message_clears_matching_default_typing`
- `durable_command_result_clears_matching_working_activity`
- `dropped_ephemeral_clear_is_repaired_by_durable_terminal_event`
- `durable_terminal_clear_is_sender_scoped`
- `durable_terminal_clear_does_not_remove_different_activity_id`
- `server_activity_cache_keeps_kind_and_activity_id_opaque`
- `server_activity_cache_preserves_multiple_opaque_events_per_route`
- `activity_projection_expires_and_rejects_bad_lease_windows`

## Command/RPC Scenarios

Finitecomputer command transport should be proven as generic Finite Chat
application payload behavior:

- `runtime_command_request_creates_command_inbox_work`
- `daemon_starts_when_hermes_is_absent_and_restarts_gateway`
- `runtime_command_request_validates_kind_body_and_target_policy`
- `runtime_command_result_requires_terminal_shape_and_bounded_clears`
- `runtime_command_ledger_records_after_decrypted_target_policy`
- `runtime_command_result_is_idempotent_terminal_event`
- `runtime_command_cancel_races_with_result_first_terminal_wins`
- `runtime_command_cancel_validates_kind_reason_and_known_request`
- `runtime_command_terminal_event_must_follow_request_sequence`
- `runtime_command_result_clears_matching_activity`
- `runtime_state_command_result_publishes_post_mutation_snapshot`
- `runtime_command_retry_reuses_message_id_and_idempotency_key`
- `runtime_command_duplicate_message_with_new_idempotency_key_rejects`
- `runtime_config_commands_serialize_per_resource`
- `runtime_bridge_commands_serialize_per_physical_resource`
- `unkeyed_runtime_commands_do_not_block_keyed_resources`
- `dashboard_status_page_load_reads_projection_without_command`
- `runtime_sync_persists_request_ledger_before_execution`
- `runtime_stream_callback_only_triggers_sync`
- `explicit_status_refresh_uses_runtime_command_without_push`
- `runtime_command_progress_uses_ephemeral_activity_without_inbox_work`
- `runtime_command_request_id_is_opaque_to_server`
- `runtime_state_snapshot_expires_to_stale_without_liveness_confusion`
- `runtime_state_snapshot_unknown_schema_is_preserved`
- `runtime_state_slow_refresh_cadence_is_bounded`
- `runtime_liveness_heartbeat_is_not_encrypted_runtime_state`
- `runtime_config_command_result_includes_post_mutation_status`
- `portable_agent_command_does_not_assume_hosted_runner`
- `hosted_runner_admin_operation_stays_out_of_generic_chat_command`
- `dashboard_does_not_require_inbound_agent_http`
- `chat_payloads_do_not_travel_over_generic_management_queue`
- `runtime_wake_hint_is_non_authoritative`
- `runtime_target_policy_uses_decrypted_payload`

FiniteChat HTTP/runtime status:

| Responsibility | Status |
| --- | --- |
| Opaque command request delivery | Covered by `/application-events` tests that persist command-inbox work, preserve opaque request ids, replay exact idempotent publishes, and reject duplicate durable message ids with new idempotency keys. |
| Command result/cancel delivery policy | Covered at the HTTP effect layer: result/cancel events are durable, non-notifying application events and survive SQLite restart. |
| Decrypted command validation, target policy, terminal races, resource serialization, and activity clears | Product-layer behavior above the HTTP delivery service. Coverage remains in `finitechat-proto` and daemon-survival tests because the server must not parse encrypted command payloads. |
| Runtime daemon execution and crash recovery | Product runtime behavior. The current repo has fake-daemon survival coverage; the HTTP delivery service only provides ordered durable input/output and idempotent publish routes until a production daemon entrypoint exists. |
| Hosted runner and management queue boundaries | Product architecture behavior outside the chat transport. These scenarios should stay outside the delivery service unless a future adapter exposes a concrete route boundary. |

## Transport Scenarios

V1 transport should prove streams are hints and pull sync is authoritative:

- `http_post_append_retries_are_idempotent`
- `sync_projection_advances_only_from_pull_pages_not_stream_hints`
- `sse_drop_duplicate_reorder_repairs_by_pull_sync`
- `sync_projection_rejects_replayed_or_wrong_room_pages`
- `sync_projection_rebuilds_same_view_after_restart`
- `stream_callback_never_executes_command_directly`
- `runtime_stream_callback_only_triggers_sync`
- `websocket_transport_not_required_for_v1`
- `push_wake_and_sse_share_hint_only_semantics`

`RoomSyncProjection` now gives clients a small protocol-shaped state machine for
this rule: stream/SSE/push hints only set `needs_pull`; only bounded
`sync_events` pages can advance the cursor or apply room-log entries.

FiniteChat HTTP/runtime status:

| Scenario | Status |
| --- | --- |
| `http_post_append_retries_are_idempotent` | Covered by raw `/messages`, typed `/events`, live-server CLI, and process-level replay tests over SQLite. |
| `sync_projection_advances_only_from_pull_pages_not_stream_hints` | Covered by the projection unit test and by `sync_projection_advances_only_from_finitechat_http_pull_pages` against the SQLite-backed FiniteChat HTTP adapter. |
| `sse_drop_duplicate_reorder_repairs_by_pull_sync` | Covered at the transport rule level by projection and partial-pull repair tests. `/sync/stream` route coverage proves coalesced high-watermark hints; `app_runtime_wait_for_update_uses_sse_hints_for_admission_and_messages` proves the app runtime treats hints as wakeups and still pulls state. |
| `sync_projection_rejects_replayed_or_wrong_room_pages` | Covered at the projection layer; the HTTP runtime adapter also rejects decoded room entries whose embedded room id does not match the requested sync room. |
| `sync_projection_rebuilds_same_view_after_restart` | Covered at the projection layer and by HTTP/runtime restart tests that rebuild state from pulled ordered log pages. |
| `stream_callback_never_executes_command_directly` | Product runtime callback behavior. `FiniteChatRuntime::wait_for_update` only converts SSE events into a pull/admission tick; command execution remains outside the hint path. Daemon-specific callback coverage should move when a production runtime daemon exists. |
| `runtime_stream_callback_only_triggers_sync` | Covered for the app runtime by `app_runtime_wait_for_update_uses_sse_hints_for_admission_and_messages`; only pulled pages and invite admission state change `AppState`. |
| `websocket_transport_not_required_for_v1` | Covered by the HTTP-only route, CLI, live-server, process-binary, and runtime-delivery tests; no WebSocket path is needed for the current V1 port. |
| `push_wake_and_sse_share_hint_only_semantics` | Covered for SSE by `/sync/stream` route tests and the Rust app runtime wait test. Push can reuse the same hint-only rule when a push adapter lands. |

## Runtime Status Snapshot Scenarios

Runtime status is encrypted application state, not request/response RPC:

- `runtime_state_projection_requires_fresh_matching_schema`
- `runtime_state_projection_fails_loudly_for_missing_stale_wrong_or_malformed_status`
- `runtime_state_snapshot_rejects_empty_key_schema_or_payload`
- `runtime_state_snapshot_is_durable_but_push_never`
- `dashboard_status_page_load_reads_projection_without_command`
- `runtime_state_snapshot_expires_to_stale_without_liveness_confusion`
- `runtime_state_snapshot_unknown_schema_is_preserved`
- `runtime_state_slow_refresh_cadence_is_bounded`

The typed projection read path rejects missing, stale, schema-mismatched, and
malformed payloads without issuing command work. Page loads should read this
projection or fail loudly; an explicit refresh remains a command.

FiniteChat HTTP/runtime status:

| Responsibility | Status |
| --- | --- |
| Snapshot payload validation and dashboard read semantics | Product-layer coverage in `finitechat-proto`; the delivery service treats snapshots as opaque encrypted application payloads. |
| Durable snapshot transport without notification side effects | Covered by HTTP application-delivery policy tests; runtime state snapshots are durable but do not create push, unread, or command-inbox work. |
| Rebuilding projected status from the ordered HTTP log | Covered by `sqlite_runtime_state_snapshot_projects_from_http_log_after_restart`, which syncs the snapshot after SQLite restart and rebuilds `RuntimeStateProjection`. |
| Runtime liveness separation | Covered by `/devices/liveness` tests: heartbeats are volatile server-visible delivery state, do not advance room sync, and are cleared by restart. |
| Slow refresh cadence and explicit refresh command behavior | Product runtime/client behavior; it should stay above the delivery service unless a future runtime daemon entrypoint owns refresh scheduling. |

## Chat Payload Scenarios

Generic chat payload semantics should stay small and non-notifying where
appropriate:

- `chat_receipt_is_encrypted_payload_semantics`
- `conversation_segment_start_is_durable_but_push_never`
- `topic_message_routes_by_conversation_id`
- `decrypted_application_event_rejects_empty_conversation_id`
- `first_message_lazily_materializes_missing_conversation`
- `new_command_inside_topic_starts_segment_not_conversation`
- `segment_boundary_rejects_missing_conversation_id_or_bad_payload`
- `archiving_topic_does_not_archive_sibling_topic`
- `reaction_edit_and_receipt_do_not_push_by_default`
- `topic_activity_is_scoped_by_conversation_id`
- `segment_boundary_is_projected_without_protocol_managed_prompt_state`

## Attachment Scenarios

Finite Chat attachments should copy the useful Pika/Blossom shape while keeping
metadata inside encrypted app payloads:

- `attachment_encrypts_before_blob_upload_and_hides_plaintext_metadata_from_store`
- `attachment_upload_verifies_ciphertext_hash`
- `attachment_download_verifies_ciphertext_hash_before_decrypt`
- `attachment_download_verifies_plaintext_hash_after_decrypt`
- `attachment_reference_metadata_lives_inside_encrypted_application_payload`
- `attachment_rejects_plaintext_over_v1_size_limit`
- `attachment_roundtrips_through_memory_blob_store`
- `attachment_reference_rejects_uppercase_hex`
- `blossom_http_upload_request_uses_ciphertext_only`
- `blossom_http_upload_request_rejects_tampered_prepared_ciphertext`
- `blossom_http_upload_response_verifies_descriptor_before_reference`
- `blossom_http_upload_response_rejects_descriptor_size_mismatch`
- `blossom_http_upload_retries_next_server_after_failure`
- `blossom_http_download_verifies_ciphertext_before_decrypt`
- `blossom_http_download_retries_same_reference_after_failure`
- `blossom_http_download_rejects_http_error_before_body_validation`
- `blossom_http_download_request_rejects_unsupported_reference_scheme`

These are currently proven in `finitechat-blob` with a local
Blossom-compatible memory store plus a Blossom-shaped HTTP request/response
boundary. The actual network executor and finitecomputer route migration remain
integration work; the encrypted reference and hash verification shape should
not change for that adapter.

## Product Mode Scenarios

Finitecomputer hosted web mode and standalone Finite Chat clients should stay
honest about trust boundaries:

- `hosted_web_mode_uses_server_side_trusted_client`
- `hosted_web_mode_is_not_labeled_e2ee`
- `product_client_kinds_have_explicit_secret_locations`
- `native_and_electron_modes_keep_device_secrets_on_user_device`
- `runtime_device_keeps_device_secret_on_runtime_host`
- `hosted_web_bridge_is_not_a_local_device_e2ee_surface`
- `local_daemon_mode_keeps_device_secrets_local`
- `runtime_bridge_state_projection_is_scoped_by_room_source_device_and_key`
- `old_plaintext_chats_render_as_read_only_archive`
