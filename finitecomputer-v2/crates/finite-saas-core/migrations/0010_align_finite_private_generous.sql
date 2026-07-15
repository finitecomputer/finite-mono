-- Keep the named default profile identical to the deployed production policy.
-- This updates policy only: grants, keys, reservations, and current usage stay
-- intact. The predicate keeps repeated schema application a no-op.
UPDATE finite_private_limit_profiles
SET burst_window_seconds = 18000,
    burst_limit_units = 50000000,
    weekly_limit_units = NULL,
    updated_at = NOW()
WHERE id = 'finite-private-generous'
  AND (
    burst_window_seconds IS DISTINCT FROM 18000
    OR burst_limit_units IS DISTINCT FROM 50000000
    OR weekly_limit_units IS NOT NULL
  );
