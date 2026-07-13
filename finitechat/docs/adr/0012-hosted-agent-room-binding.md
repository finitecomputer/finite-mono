# ADR 0012: Hosted Agent conversations have one durable Room binding

Status: accepted 2026-07-13

For each current-shape `(Project, human Principal, Agent Principal)` tuple,
`finitechat-hosted-device` owns one versioned, authenticated-encrypted binding
to the canonical exact-member Agent Room. A selected Room, Topic, or Chat is a
navigation cursor and never chooses, creates, replaces, or repairs that
binding.

Bootstrap opens and validates an existing binding before contacting the Agent
Runtime. A missing binding enters the bounded legacy migration: an existing
valid binding wins; otherwise connected Rooms with exactly the human and Agent
Principals are ordered by the oldest comparable durable fact, with Room-id
order as the deterministic fallback/tiebreak. The current store has no
comparable Room-creation timestamp, so the implementation uses Room-id order.
It records the first Room as canonical and every remaining eligible Room as an
associated previous conversation. Migration is idempotent and must preserve
the retained Room, Topic, and Chat identifier sets.

Associated Rooms are not deleted, merged, left, or re-encrypted. Their Topics
and Chats remain reachable under `Previous conversations`. Owner claim and
new-Chat creation always target the canonical Room. One client intent key maps
deterministically to one Chat id, so retrying the same action cannot create a
second Chat; a separate intent creates a separate Chat.

Recovery never creates a Room. The former dashboard `/fresh` endpoint and its
`StartGroupChat` behavior are deleted without a compatibility flag. Missing,
corrupt, wrong-identity, changed-Agent, or membership-invalid bindings fail
closed as recovery-required.
