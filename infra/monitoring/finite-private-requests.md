# Finite Private request measurements

This is the measurement/storage layer of [issue #946](https://github.com/finitecomputer/finite-mono/issues/946).
The accounting cancellation fix is its prerequisite. Export to Loki, authenticated
Prometheus collection, and Grafana presentation are a separate follow-up.

## Contracts

- Limiter counters cover all inference arrivals, refusals, active work, and
  outcomes. Route token counts and timing histograms use bounded model/endpoint
  labels; request, key, Project, and runtime IDs are never Prometheus labels.
- First output includes reasoning, answer text, and tool arguments, not headers,
  keepalives, role-only messages, or tool names. First answer measures answer
  text only. Nonstreaming requests have duration only; all timings stop before
  settlement retries. Missing upstream token usage remains unavailable.
- `/metrics` retains liveness-only behavior until the dedicated
  `FINITE_PRIVATE_METRICS_TOKEN` is configured. Other credentials do not grant
  telemetry access. A later PR adds the matching authenticated scrape.
- Only Core-reserved requests have individual diagnostics. Core derives
  reservation-time attribution from trusted key-issue audit metadata; ambiguous
  ownership stays unknown. No raw keys, prompts, responses, or headers are kept.
- The service-authenticated diagnostic POST returns 204. Core timestamps one
  immutable row per reservation; retries cannot rewrite it or extend retention.
  Diagnostics do not copy grant balances or accounting state.
- Best-effort writes use eight background slots, a 250 ms connection/lock budget,
  and two-second SQL budget. Monitoring failure cannot change inference or
  admission/settlement. Old Core rejects the additive route without breaking
  new limiter requests; old limiters keep working against new Core.

## Retention and activation boundary

The schema ships with independent ten-minute cleanup and both Core database
backup exclusions. These must be active **before deploying the diagnostic
writer**. Cleanup removes diagnostics older than seven days, including when
requests and exports stop. It does not delete accounting records. Backups keep
the schema but exclude diagnostic data; restore yields intact accounting and
empty diagnostics. An older database dump cannot be repaired by later pruning.

This PR does not activate production telemetry. Before the separately authorized
Production Deploy, verify running Core/limiter versions, admission mode, operator
access, upstream usage schema, and external snapshot retention. Run
`scripts/finite-status` before and after. Retain the previous measured Tinfoil
release: restarting the enclave can interrupt in-flight inference. Roll back
compatible binaries and disable collection while keeping the additive schema;
use the [Postgres recovery runbook](../runbooks/postgres-backup-restore.md) for restoration.
