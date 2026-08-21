-- One forward-only offboarding phase on the runtime record. Before this
-- column, offboarding progress was spread across independent durable facts
-- (the destroy control request, the verified retirement receipt, the
-- project-runtime link, room membership, relay credential, Finite Private
-- keys, and the departure fact) with no single state and no owner of "which
-- step am I on" — the ghost-drift-records class behind the Sites Canary 0715
-- half-retirement, the Sol 2 unarchived disappearance, and the
-- runtime-retire-exact uncapped retry wedge against an absent container.
--
-- The phase is written strictly forward, each transition in the same
-- transaction as the side effect it records. NULL means the runtime is live
-- (not offboarding). `archived` is terminal; Purge User Data stays the
-- separate retention-gated path (ADR 0001) and is out of scope here.

ALTER TABLE agent_runtimes
  ADD COLUMN IF NOT EXISTS offboarding_phase TEXT;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conrelid = 'agent_runtimes'::regclass
      AND conname = 'agent_runtimes_offboarding_phase_check'
  ) THEN
    ALTER TABLE agent_runtimes
      ADD CONSTRAINT agent_runtimes_offboarding_phase_check
      CHECK (
        offboarding_phase IS NULL
        OR offboarding_phase IN (
          'retirement_requested',
          'receipt_verified',
          'compute_removed',
          'link_deactivated',
          'archived'
        )
      );
  END IF;
END $$;

-- One-time backfill from the legacy durable facts. The WHERE guard makes
-- re-applying the schema a no-op for every row the phase machine has already
-- recorded, so the mapping runs exactly once per pre-existing row. The truth
-- table (mirrored by OffboardingPhase::from_legacy_facts):
--
--   verified receipt + active link    -> compute_removed (the half-retired
--                                        ghost: the runner completed the
--                                        destroy, offboarding never ran)
--   verified receipt + inactive link  -> archived (retirement completed)
--   no receipt + active destroy + active link -> retirement_requested
--   no receipt + active link          -> NULL (live)
--   no receipt + no link rows         -> NULL (never linked; no evidence)
--   no receipt + inactive link, project has another active link
--                                     -> NULL (superseded by relocation,
--                                        not offboarded)
--   no receipt + inactive link, project has no active link
--                                     -> archived (unrecoverable archive or
--                                        legacy offboard)
UPDATE agent_runtimes AS runtime
  SET offboarding_phase = CASE
    WHEN EXISTS (
      SELECT 1 FROM runtime_retirement_snapshots AS snapshot
      WHERE snapshot.agent_runtime_id = runtime.id
    ) THEN CASE
      WHEN EXISTS (
        SELECT 1 FROM project_runtime_links AS link
        WHERE link.agent_runtime_id = runtime.id AND link.active
      ) THEN 'compute_removed'
      ELSE 'archived'
    END
    WHEN EXISTS (
      SELECT 1 FROM runtime_control_requests AS control
      WHERE control.agent_runtime_id = runtime.id
        AND control.kind = 'destroy'
        AND control.status IN ('requested', 'running')
    ) AND EXISTS (
      SELECT 1 FROM project_runtime_links AS link
      WHERE link.agent_runtime_id = runtime.id AND link.active
    ) THEN 'retirement_requested'
    WHEN EXISTS (
      SELECT 1 FROM project_runtime_links AS link
      WHERE link.agent_runtime_id = runtime.id AND link.active
    ) THEN NULL
    WHEN NOT EXISTS (
      SELECT 1 FROM project_runtime_links AS link
      WHERE link.agent_runtime_id = runtime.id
    ) THEN NULL
    WHEN EXISTS (
      SELECT 1 FROM project_runtime_links AS project_link
      WHERE project_link.project_id = runtime.project_id AND project_link.active
    ) THEN NULL
    ELSE 'archived'
  END
  WHERE runtime.offboarding_phase IS NULL;
