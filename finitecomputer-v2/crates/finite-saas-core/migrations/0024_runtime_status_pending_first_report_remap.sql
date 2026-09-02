-- Remap the short-lived `pending_first_report` lifecycle latch (written by
-- the first cut of health-derived runtime status) to the representation that
-- replaced it: `online` with the stored health report cleared, which the
-- read-time projection derives as `unknown` until the runner's standing
-- poller first reports on the running incarnation. Rewriting the latch alone
-- would let a report stored for the previous incarnation speak again, so the
-- latch and the health columns move in one statement.
--
-- The attribution pin (`health_reporting_npub`) is kept: a pending row's pin
-- was pinned by the same runtime's earlier reports, and restart/upgrade keep
-- the principal. Nothing reads the old string after this runs; the enum parses
-- it as `unknown` only as forward-tolerance for unrelated future additions.
-- Idempotent: safe to reapply at every Core startup like the rest of the
-- schema concat.

UPDATE agent_runtimes
   SET host_facts = jsonb_set(host_facts, '{runtime_status}', to_jsonb('online'::text)),
       health_reported_at = NULL,
       health_observed_at = NULL,
       health_ready = NULL,
       health_reason = NULL,
       health_report_interval_seconds = NULL
 WHERE host_facts->>'runtime_status' = 'pending_first_report';
