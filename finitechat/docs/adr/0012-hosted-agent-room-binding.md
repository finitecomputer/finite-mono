# ADR 0012: Hosted Agent conversations have one durable Room binding

Status: accepted 2026-07-13; amended 2026-07-13 to remove automatic legacy
selection

For each current-shape `(Project, human Principal, Agent Principal)` tuple,
`finitechat-hosted-device` owns one versioned, authenticated-encrypted binding
to the canonical exact-member Agent Room. A selected Room, Topic, or Chat is a
navigation cursor and never chooses, creates, replaces, migrates, or repairs
that binding. Room creation time, display order, timestamps, and identifier
order likewise confer no authority. After its final write, ordinary product
flows treat the entire binding as immutable: open validates and uses the
recorded canonical and associated Room ids but never reconciles or rewrites
them.

Bootstrap opens and validates an existing binding before contacting the Agent
Runtime. An already-valid binding remains valid across reload, restart, deploy,
and upgrade. The authenticated Project-creation workflow writes a sealed,
one-time bootstrap authorization containing the Project and creation-request
identity before ordinary chat is opened. Once the Agent Principal is available,
the authorization advances into a sealed staged bootstrap journal. Before any
server mutation, the journal durably records the exact Room create request,
including its intended Room id, MLS group id, creator Device, and protocol.
After claiming the Agent KeyPackage, it durably records that exact claim before
Room creation. The Device sends only the journaled create request. If the server
accepted it but the Device failed before saving the matching local MLS group,
restart resubmits the same idempotent request and materializes local state with
the same MLS group id. The Device then prepares the add-member operation and
durably records the exact prepared commit before submitting it.

Creation and crash retry are serialized and may create, prepare, submit, or
finish only those journaled artifacts. They may not claim a replacement
KeyPackage after the claim is journaled, generate a different Room or MLS group,
regenerate a different prepared commit, scan for another Room, or adopt one.
This is product initialization, not recovery or legacy migration. A regression
covers the specific crash boundary where the Room server accepted creation but
the Device had not saved its matching local MLS group; restart must converge on
the one journaled Room rather than create another.

No ordinary load, restart, deploy, upgrade, or recovery path may synthesize the
authorization. One narrow handoff retry exists for the case where Core
committed Project creation but the dashboard lost the authorization response:
ordinary chat load exposes a user-visible `Finish chat setup` action but does
not authorize by itself. Invoking the action performs a fresh Core read and may
write authorization only if it finds the exact Account-owned Project and
exactly one durable creation request for that Project in `requested`,
`launching`, or `running` state. It never examines Room state to obtain
authority.

There is no automatic Room reconciliation, legacy Room selection, or migration.
If an existing Project has neither a binding nor its exact creation
authorization, has one or more retained candidate Rooms, or has a corrupt,
wrong-identity, changed-Agent, or membership-invalid binding, the device fails
closed without choosing another Room or rewriting retained state. A future
repair requires read-only evidence, a reproduced cause, explicit
production-mutation authority, synthetic proof, and a named backup and rollback
boundary.

Rooms already recorded as associated previous conversations are not deleted,
merged, left, or re-encrypted. Their Topics and Chats remain reachable under
`Previous conversations`. Owner claim and new-Chat creation always target the
valid canonical Room; ordinary navigation does not add or remove associated
Room ids. One client intent key maps deterministically to one Chat id, so
retrying the same action cannot create a second Chat; a separate intent creates
a separate Chat.

Recovery never creates a Room. The former dashboard `/fresh` endpoint and its
`StartGroupChat` behavior are deleted without a compatibility flag. Durable
protocol sync may discover and process already-authorized membership or
messages after reconnect; that is ordinary protocol convergence, not authority
to select a binding or a migration/repair mechanism.
