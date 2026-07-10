CREATE TABLE IF NOT EXISTS launch_code_batches (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
  code_count INTEGER NOT NULL CHECK (code_count BETWEEN 1 AND 1000),
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  revoked_by_workos_user_id TEXT,
  created_by_workos_user_id TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  CHECK (expires_at > created_at),
  CHECK (expires_at <= created_at + INTERVAL '30 days'),
  CHECK (
    (revoked_at IS NULL AND revoked_by_workos_user_id IS NULL)
    OR
    (revoked_at >= created_at AND revoked_by_workos_user_id IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS launch_code_batches_created_at_idx
  ON launch_code_batches(created_at DESC, id);

CREATE TABLE IF NOT EXISTS launch_codes (
  id TEXT PRIMARY KEY,
  batch_id TEXT NOT NULL REFERENCES launch_code_batches(id),
  code_hash TEXT NOT NULL UNIQUE,
  redeemed_customer_org_id TEXT REFERENCES customer_orgs(id),
  redemption_idempotency_key TEXT,
  redeemed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL,
  CHECK (
    (redeemed_customer_org_id IS NULL
      AND redemption_idempotency_key IS NULL
      AND redeemed_at IS NULL)
    OR
    (redeemed_customer_org_id IS NOT NULL
      AND redemption_idempotency_key IS NOT NULL
      AND redeemed_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS launch_codes_batch_id_idx
  ON launch_codes(batch_id, id);

-- The former sponsored-access label happened to equal the retired public
-- shared code. Keep the billing concept while removing code material from
-- customer, entitlement, and request rows before new foreign keys are added.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1
      FROM pg_constraint
      WHERE conrelid = 'customer_orgs'::regclass
        AND conname = 'customer_orgs_billing_class_check'
  ) THEN
    ALTER TABLE customer_orgs
      DROP CONSTRAINT customer_orgs_billing_class_check;
  END IF;

  UPDATE customer_orgs
    SET billing_class = 'sponsored'
    WHERE billing_class = 'off2026';

  ALTER TABLE customer_orgs
    ADD CONSTRAINT customer_orgs_billing_class_check
    CHECK (billing_class IN ('grandfathered', 'sponsored', 'standard'));
END $$;

UPDATE agent_creation_entitlements
  SET launch_code = NULL
  WHERE launch_code IS NOT NULL
    AND launch_code NOT IN (SELECT id FROM launch_codes);

UPDATE agent_creation_requests
  SET requested_launch_code = NULL
  WHERE requested_launch_code IS NOT NULL
    AND requested_launch_code NOT IN (SELECT id FROM launch_codes);

CREATE UNIQUE INDEX IF NOT EXISTS agent_creation_entitlements_one_launch_code
  ON agent_creation_entitlements(launch_code)
  WHERE launch_code IS NOT NULL;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
      FROM pg_constraint
      WHERE conrelid = 'agent_creation_entitlements'::regclass
        AND conname = 'agent_creation_entitlements_launch_code_fkey'
  ) THEN
    ALTER TABLE agent_creation_entitlements
      ADD CONSTRAINT agent_creation_entitlements_launch_code_fkey
      FOREIGN KEY (launch_code) REFERENCES launch_codes(id);
  END IF;

  IF NOT EXISTS (
    SELECT 1
      FROM pg_constraint
      WHERE conrelid = 'agent_creation_requests'::regclass
        AND conname = 'agent_creation_requests_requested_launch_code_fkey'
  ) THEN
    ALTER TABLE agent_creation_requests
      ADD CONSTRAINT agent_creation_requests_requested_launch_code_fkey
      FOREIGN KEY (requested_launch_code) REFERENCES launch_codes(id);
  END IF;
END $$;
