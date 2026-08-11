-- A reusable, account-assignable Finite Private profile with five times the
-- default burst allowance. Profile assignment is an operator action; this
-- migration deliberately does not select or mutate any customer grant.
INSERT INTO finite_private_limit_profiles (
  id,
  burst_window_seconds,
  burst_limit_units,
  weekly_limit_units,
  created_at,
  updated_at
)
VALUES (
  'finite-private-generous-5x-v1',
  18000,
  500000000,
  NULL,
  NOW(),
  NOW()
)
ON CONFLICT (id) DO UPDATE
SET burst_window_seconds = EXCLUDED.burst_window_seconds,
    burst_limit_units = EXCLUDED.burst_limit_units,
    weekly_limit_units = EXCLUDED.weekly_limit_units,
    updated_at = NOW();
