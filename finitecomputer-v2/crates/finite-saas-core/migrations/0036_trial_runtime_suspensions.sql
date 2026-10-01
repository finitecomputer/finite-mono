-- One authoritative, read-only access predicate for dashboard reads and queue
-- admission. Only redeemed event trials opt in; paid and exempt orgs retain
-- their existing access. The deadline is exclusive: there is no grace period.
CREATE OR REPLACE FUNCTION core_trial_access_blocked(org_id TEXT, at_time TIMESTAMPTZ)
RETURNS BOOLEAN LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM trial_redemptions t
        JOIN trial_campaigns c ON c.id = t.campaign_id
        JOIN customer_orgs o ON o.id = t.customer_org_id
        LEFT JOIN customer_billing_accounts b ON b.customer_org_id = o.id
        WHERE o.id = org_id AND t.state = 'redeemed' AND o.billing_class = 'standard'
          AND NOT COALESCE(
              b.subscription_status = 'active'
              OR (b.subscription_status = 'trialing'
                  AND COALESCE(b.current_period_end,
                      t.redeemed_at + make_interval(days => c.trial_days)) > at_time),
              FALSE)
    )
$$;

-- Tracks trial stops and explicit stop intent across failed operations/retries.
-- Payment must never start an agent its owner/operator asked to stop. No user
-- state is held here; deleting a marker never purges runtime/home/history data.
CREATE TABLE IF NOT EXISTS trial_runtime_suspensions (
    agent_runtime_id TEXT PRIMARY KEY REFERENCES agent_runtimes(id) ON DELETE CASCADE,
    stop_request_id TEXT NOT NULL REFERENCES runtime_control_requests(id) ON DELETE CASCADE,
    resume_request_id TEXT REFERENCES runtime_control_requests(id) ON DELETE SET NULL,
    resume_allowed BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE trial_runtime_suspensions
    ADD COLUMN IF NOT EXISTS resume_allowed BOOLEAN NOT NULL DEFAULT TRUE;
