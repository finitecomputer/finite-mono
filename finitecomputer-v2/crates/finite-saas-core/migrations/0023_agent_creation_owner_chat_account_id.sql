-- Launch-time owner chat identity granting (chat-authz/owner-npubs).
-- The dashboard pre-mints the owner's hosted chat identity and submits its
-- 64-hex account id with the agent-creation request. Core persists it here
-- and, at lease time, injects it into the runtime spec environment as
-- FINITECHAT_OWNER_NPUBS so the runtime image can scope the Hermes adapter
-- allowlist and the chat sidecar Welcome allowlist to the owner. Purely
-- additive and default-null: existing rows keep the legacy allow-all
-- behavior, and the migration is safe to reapply at every Core startup just
-- like the rest of the schema concat.

ALTER TABLE agent_creation_requests
  ADD COLUMN IF NOT EXISTS owner_chat_account_id TEXT;

COMMENT ON COLUMN agent_creation_requests.owner_chat_account_id IS
  'Owner hosted-chat account id (64 lowercase hex), submitted at creation and injected into the lease-time runtime spec environment as FINITECHAT_OWNER_NPUBS; NULL keeps legacy allow-all chat admission.';
