-- One RFC3339 rendering for every TIMESTAMPTZ read.
--
-- Core's API contract is RFC3339 UTC, and `parse_time` rejects anything else.
-- A bare `col::text` renders Postgres's own display format in the SERVER's
-- timezone ("2026-05-25 07:00:00-05"), so those columns did not round-trip:
-- Core emitted timestamps it could not itself parse. Reads now go through this
-- function instead of casting.
--
-- Trailing zeros in the fractional second are dropped so a value round-trips
-- byte-for-byte with the RFC3339 string Rust wrote: "…T12:00:00Z" stays
-- "…T12:00:00Z", and "…:23.39094Z" does not come back as "…:23.390940Z".
CREATE OR REPLACE FUNCTION core_rfc3339(ts TIMESTAMPTZ)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
PARALLEL SAFE
AS $$
  SELECT CASE
    WHEN ts IS NULL THEN NULL
    ELSE replace(
      regexp_replace(
        to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
        '0+Z$',
        'Z'
      ),
      '.Z',
      'Z'
    )
  END
$$;
