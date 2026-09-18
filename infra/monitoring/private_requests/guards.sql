-- Current grant state, not additive usage events. Do not sum repeat snapshots.
-- The rolling-week sum follows Core's reservation-created-at accounting rule.
SELECT COALESCE(json_agg(record), '[]'::json) FROM (
  SELECT (EXTRACT(EPOCH FROM CURRENT_TIMESTAMP) * 1000000000)::numeric(30,0)::text AS "timestampNs",
    json_build_object(
      'version', 1, 'observedAtUnixSeconds', EXTRACT(EPOCH FROM CURRENT_TIMESTAMP)::bigint, 'observedAt', CURRENT_TIMESTAMP, 'grantId', g.id,
      'burstLimitUnits', p.burst_limit_units,
      'burstUsedUnits', CASE WHEN CURRENT_TIMESTAMP < g.current_window_started_at + p.burst_window_seconds * INTERVAL '1 second'
        THEN g.current_window_used_units ELSE 0 END,
      'burstResetAt', (CASE WHEN CURRENT_TIMESTAMP < g.current_window_started_at + p.burst_window_seconds * INTERVAL '1 second'
        THEN g.current_window_started_at ELSE NULL END) + p.burst_window_seconds * INTERVAL '1 second',
      'weeklyLimitUnits', p.weekly_limit_units,
      'rollingWeekUsedUnits', CASE WHEN p.weekly_limit_units IS NOT NULL THEN weekly.used ELSE NULL END,
      'rollingWeekNextExpiryAt', CASE WHEN p.weekly_limit_units IS NOT NULL THEN weekly.earliest + INTERVAL '7 days' ELSE NULL END,
      'agingReservations', aging.count,
      'oldestReservationAt', aging.oldest
    ) AS event
  FROM finite_private_grants g
  JOIN finite_private_limit_profiles p ON p.id = g.limit_profile_id
  LEFT JOIN LATERAL (
    SELECT COALESCE(SUM(COALESCE(r.settled_usage_units, r.reserved_usage_units)), 0) AS used,
      MIN(r.created_at) AS earliest FROM finite_private_reservations r
    WHERE p.weekly_limit_units IS NOT NULL AND r.grant_id = g.id AND r.status <> 'denied'
      AND r.created_at BETWEEN CURRENT_TIMESTAMP - INTERVAL '7 days' AND CURRENT_TIMESTAMP
  ) weekly ON true
  LEFT JOIN LATERAL (
    SELECT COUNT(*) AS count, MIN(r.created_at) AS oldest FROM finite_private_reservations r
    WHERE r.grant_id = g.id AND r.status = 'reserved'
      AND r.created_at < CURRENT_TIMESTAMP - INTERVAL '15 minutes'
  ) aging ON true
  WHERE g.status = 'active'
  ORDER BY g.id LIMIT 1001
) record;
