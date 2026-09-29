-- Trial campaigns are separate from sponsored launch codes. Reservations are
-- released only by verified Stripe expiry, never by a local timeout which could
-- race a completed checkout whose webhook has not arrived yet.
CREATE TABLE IF NOT EXISTS trial_campaigns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    code_hash TEXT NOT NULL UNIQUE,
    seat_limit INTEGER NOT NULL CHECK (seat_limit BETWEEN 1 AND 10000),
    trial_days INTEGER NOT NULL DEFAULT 7 CHECK (trial_days BETWEEN 1 AND 30),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by_workos_user_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS trial_redemptions (
    id TEXT PRIMARY KEY,
    campaign_id TEXT NOT NULL REFERENCES trial_campaigns(id),
    customer_org_id TEXT NOT NULL REFERENCES customer_orgs(id),
    attempt_id TEXT NOT NULL UNIQUE,
    stripe_session_id TEXT NOT NULL UNIQUE,
    stripe_subscription_id TEXT UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('reserved', 'redeemed', 'expired')),
    checkout_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    redeemed_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX IF NOT EXISTS trial_redemptions_one_trial_per_org
    ON trial_redemptions(customer_org_id) WHERE state <> 'expired';
CREATE INDEX IF NOT EXISTS trial_redemptions_campaign ON trial_redemptions(campaign_id);

-- Serialize expiry against reservation even if expiry arrives before the seat
-- insert. Keeping the outcome prevents an expired checkout from acquiring one.
CREATE TABLE IF NOT EXISTS trial_checkout_sessions (
    stripe_session_id TEXT PRIMARY KEY,
    stripe_customer_id TEXT NOT NULL,
    expired BOOLEAN NOT NULL DEFAULT FALSE
);
