-- Double the default Finite Private burst allowance without making an N-1
-- Core rollback silently restore the old value. Older binaries continue to
-- reconcile `finite-private-generous`; existing grants are moved to this new
-- profile id, which those binaries can already read.
CREATE OR REPLACE FUNCTION preserve_doubled_finite_private_default()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
  IF NEW.id = 'finite-private-generous' AND NEW.burst_limit_units < 100000000 THEN
    NEW.burst_limit_units := 100000000;
  END IF;
  RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS preserve_doubled_finite_private_default
  ON finite_private_limit_profiles;
CREATE TRIGGER preserve_doubled_finite_private_default
BEFORE INSERT OR UPDATE ON finite_private_limit_profiles
FOR EACH ROW EXECUTE FUNCTION preserve_doubled_finite_private_default();

INSERT INTO finite_private_limit_profiles (
  id,
  burst_window_seconds,
  burst_limit_units,
  weekly_limit_units,
  created_at,
  updated_at
)
VALUES (
  'finite-private-generous',
  18000,
  100000000,
  NULL,
  NOW(),
  NOW()
)
ON CONFLICT (id) DO UPDATE
SET burst_window_seconds = EXCLUDED.burst_window_seconds,
    burst_limit_units = EXCLUDED.burst_limit_units,
    weekly_limit_units = EXCLUDED.weekly_limit_units,
    updated_at = NOW();

INSERT INTO finite_private_limit_profiles (
  id,
  burst_window_seconds,
  burst_limit_units,
  weekly_limit_units,
  created_at,
  updated_at
)
VALUES (
  'finite-private-generous-v2',
  18000,
  100000000,
  NULL,
  NOW(),
  NOW()
)
ON CONFLICT (id) DO UPDATE
SET burst_window_seconds = EXCLUDED.burst_window_seconds,
    burst_limit_units = EXCLUDED.burst_limit_units,
    weekly_limit_units = EXCLUDED.weekly_limit_units,
    updated_at = NOW();

UPDATE finite_private_grants
SET limit_profile_id = 'finite-private-generous-v2',
    updated_at = NOW()
WHERE limit_profile_id = 'finite-private-generous';
