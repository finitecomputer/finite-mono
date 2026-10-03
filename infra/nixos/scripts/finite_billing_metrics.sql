-- One organization is one billing account, including organizations whose billing
-- row is missing. Never join users, runtimes, payments, or individual usage.
BEGIN TRANSACTION READ ONLY;
SET LOCAL statement_timeout = '5s';
SET LOCAL lock_timeout = '1s';
WITH accounts AS (
    SELECT o.billing_class, b.customer_org_id, b.stripe_subscription_id,
           b.subscription_status,
           COALESCE(b.current_period_end,
                    t.redeemed_at + make_interval(days => c.trial_days)) AS trial_end
    FROM customer_orgs o
    LEFT JOIN customer_billing_accounts b ON b.customer_org_id = o.id
    -- The partial unique index permits at most one redeemed trial per org.
    LEFT JOIN trial_redemptions t ON t.customer_org_id = o.id AND t.state = 'redeemed'
    LEFT JOIN trial_campaigns c ON c.id = t.campaign_id
), classified AS (
    SELECT CASE
        WHEN customer_org_id IS NULL THEN 'missing_billing_account'
        WHEN billing_class IS NULL OR billing_class NOT IN ('standard', 'sponsored', 'grandfathered')
            THEN 'unknown'
        WHEN (stripe_subscription_id IS NULL) <> (subscription_status IS NULL)
          OR btrim(stripe_subscription_id) = ''
          OR subscription_status NOT IN ('active', 'trialing', 'incomplete',
              'incomplete_expired', 'past_due', 'canceled', 'unpaid', 'paused')
            THEN 'unknown'
        WHEN subscription_status = 'active' THEN 'subscribed'
        WHEN billing_class = 'sponsored' THEN 'sponsored'
        WHEN billing_class = 'grandfathered' THEN 'grandfathered'
        WHEN subscription_status = 'trialing' AND trial_end IS NULL THEN 'unknown'
        WHEN subscription_status = 'trialing' AND trial_end > CURRENT_TIMESTAMP THEN 'trial'
        WHEN subscription_status IN ('trialing', 'incomplete_expired', 'past_due', 'canceled', 'unpaid', 'paused')
            THEN 'expired_past_due'
        WHEN subscription_status = 'incomplete' THEN 'incomplete'
        WHEN subscription_status IS NULL AND stripe_subscription_id IS NULL THEN 'no_subscription'
        ELSE 'unknown'
    END AS status FROM accounts
), counts AS (
    SELECT status, count(*) AS count FROM classified GROUP BY status
)
SELECT json_build_object(
    'collected_at', floor(extract(epoch FROM CURRENT_TIMESTAMP))::bigint,
    'counts', COALESCE(json_object_agg(status, count), '{}'::json)
) FROM counts;
COMMIT;
