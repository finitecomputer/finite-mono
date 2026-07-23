-- Row-native serialization point for provider capacity whose external
-- inventory can lag an accepted Core creation lease. This is deliberately one
-- bounded runner-class row, not a global database lock or full-state snapshot.
CREATE TABLE IF NOT EXISTS runner_capacity_fences (
  runner_class TEXT PRIMARY KEY
    CHECK (runner_class IN ('phala')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO runner_capacity_fences (runner_class)
VALUES ('phala')
ON CONFLICT (runner_class) DO NOTHING;
