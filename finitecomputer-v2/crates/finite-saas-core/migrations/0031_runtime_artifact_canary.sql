-- Scoped candidates stay unpromoted, including under older Core readers.
ALTER TABLE runtime_artifacts
  ADD COLUMN IF NOT EXISTS canary_runtime_id TEXT REFERENCES agent_runtimes(id);

DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint
                 WHERE conname = 'runtime_artifacts_canary_unpromoted') THEN
    ALTER TABLE runtime_artifacts
      ADD CONSTRAINT runtime_artifacts_canary_unpromoted
      CHECK (canary_runtime_id IS NULL OR promoted_at IS NULL);
  END IF;
END $$;

-- Older writers do not know that a scoped candidate is already approved.
-- Freeze its identity at registration, before it has a mounted runtime.
-- Retirement remains allowed; general promotion requires a new artifact ID.
CREATE OR REPLACE FUNCTION core_guard_canary_artifact() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
  IF OLD.canary_runtime_id IS NOT NULL
     AND (to_jsonb(NEW) - 'retired_at') IS DISTINCT FROM
         (to_jsonb(OLD) - 'retired_at') THEN
    RAISE EXCEPTION 'scoped canary artifact is immutable'
      USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE TRIGGER runtime_artifacts_canary_immutable
BEFORE UPDATE ON runtime_artifacts
FOR EACH ROW EXECUTE FUNCTION core_guard_canary_artifact();
