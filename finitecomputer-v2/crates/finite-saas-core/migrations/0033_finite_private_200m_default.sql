-- Raise the standard five-hour allowance without resetting grants or usage.
-- Core replays all migrations at startup, including after a binary rollback.
-- Keep this guard separate from 0013's function/trigger: an older Core replays
-- 0013 and replaces those definitions before upserting the old 100M values.
CREATE OR REPLACE FUNCTION preserve_finite_private_200m_default()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
  IF NEW.id IN ('finite-private-generous', 'finite-private-generous-v2')
     AND NEW.burst_limit_units < 200000000 THEN
    NEW.burst_limit_units := 200000000;
  END IF;
  RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS preserve_finite_private_200m_default
  ON finite_private_limit_profiles;
CREATE TRIGGER preserve_finite_private_200m_default
BEFORE INSERT OR UPDATE ON finite_private_limit_profiles
FOR EACH ROW EXECUTE FUNCTION preserve_finite_private_200m_default();

UPDATE finite_private_limit_profiles
SET burst_limit_units = 200000000,
    updated_at = NOW()
WHERE id IN ('finite-private-generous', 'finite-private-generous-v2')
  AND burst_limit_units < 200000000;

-- The five-hour window, no-weekly-cap policy, 500M profile, keys, grants,
-- reservation ledger, daily resets, and token weighting are unchanged.
