# Finite Private requests and usage

This operator dashboard implements [issue #946](https://github.com/finitecomputer/finite-mono/issues/946).
It supplements the GPU/host dashboard with request traffic, token use and
limiter-observed performance. It is not a customer billing system.

## Reading the dashboard

All-traffic measurements count inference requests that reach the Finite Private
limiter. Health and metrics probes are excluded. Admission and completion are
separate events: an admitted request can later fail. Active requests describe
live work, not reservations that have not settled.

The request-detail table covers only Core-reserved requests within the last
seven days. Refused requests and degraded-allowlist requests appear in aggregate
counts, without individual detail rows. A Project, Agent Runtime or key filter
applies only where Core has a trusted association. Shared or unattributed
traffic must not silently acquire the selected Project's identity.

Input and output tokens are separate from weighted Finite Private Runaway Guard
usage units. Missing upstream usage is unavailable; estimated accounting does
not establish measured token counts. Collection cannot backfill token counts
that were never retained. A successful HTTP status alone does not establish a
successful stream or prove that the client received all output.

Token measurements use the upstream usage object's `prompt_tokens` and
`completion_tokens` (or Responses API `input_tokens` and `output_tokens`).
They do not add cached-token or reasoning-token subfields onto those totals,
which could double count overlapping categories. Validate the running model's
usage schema during rollout. Missing or partial usage reports must remain
explicitly unavailable rather than being inferred from chunk counts.

For streaming responses, first-output delay starts when the limiter accepts
the request and ends at its first meaningful reasoning, answer or tool output.
First-answer delay ends only at the first answer text. Headers, keepalives and
role-only events do not count. A stream without answer text has no first-answer
measurement. Nonstreaming requests expose total duration, not first-token
latency. These are limiter observations, not client-perceived latency or engine
token timestamps. No output content is retained to make these measurements.

Aggregate output throughput counts output tokens over wall-clock time.
Per-request token speed is currently unavailable; the dashboard does not
present throughput as exact engine decode speed. Final usage
can arrive only when a request ends. The one-minute freshness target begins
when the observation becomes available, not while final usage is still pending.

## Access and retention

Only Internal Operators may access monitoring. Verify Grafana users,
organization membership, roles and datasource permissions before activation.
Anonymous access being disabled is necessary but does not establish that all
existing accounts are authorized operators. No customer-facing query route or
shared dashboard snapshot is part of this feature.

The limiter's `FINITE_PRIVATE_METRICS_TOKEN` is a dedicated monitoring
credential. Store its value only in the existing deployment secret mechanism
and the collector's restricted credential file. Inference, upstream-model and
Core accounting credentials do not authorize metric reads. Leaving this
variable unset preserves the legacy liveness-only `/metrics` response; it
does not publish request telemetry anonymously. Provision the credential and
the matching authenticated scrape together during the authorized rollout.

Diagnostic details have a seven-day query window and explicit cleanup.
Preserve existing accounting records needed by the Runaway Guard. Prometheus
retains bounded aggregate series for up to 15 days, subject to its 20 GB cap.
This does not promise 15 days of per-key or per-Project detail. Never use raw
keys, request IDs, arbitrary error strings or unbounded identity values as
Prometheus labels. Do not retain prompts, responses or arbitrary headers.

Physical expiry, backups, journals and secondary copies must follow the
documented data path. A seven-day query cutoff alone is not proof that every
copy has been deleted. A restored diagnostic store must apply its expiry rules
before serving queries.

Diagnostics are disposable operational observations, outside the durable
Recovery Set. Both current Postgres dump paths—the six-hourly database dump
and the coordinated hosted Recovery Snapshot—must exclude diagnostic **table
data** while retaining its schema. Existing accounting, identity and chat
backup coverage is unchanged. Restore validation must show that Core runs
with an empty diagnostic table and the accounting ledger intact. This
exclusion must be active before diagnostics are enabled: later SQL deletion
cannot remove rows from an older dump or Borg archive.

Export diagnostic rows directly to Loki without logging their payloads to
journald. Apply a dedicated seven-day Loki stream policy; the general
fourteen-day policy is too long for these records. Loki compaction and its
configured deletion delay govern physical removal after expiry. No
repository-managed Loki backup was found. Provider disk snapshots or other
external copies must be checked during the production handoff, and any such
copies need the same diagnostic retention boundary.

## Collection data path

Prometheus scrapes the limiter every 15 seconds with the dedicated monitoring
credential. The receiver reads its copy from
`/etc/finite/monitoring/private-limiter-metrics-token`; the measured container
receives the matching `FINITE_PRIVATE_METRICS_TOKEN` secret. Provision the
values separately from source and verify permissions before activating the
scrape.

On `finite-lat-2`, `finite-private-request-diagnostics-exporter.service` runs
the [reporting helper](private_requests/export.py) every 15 seconds. It reads
bounded batches of unacknowledged diagnostic events through a local Postgres
role, sends them to the existing authenticated Loki ingress, and acknowledges
successful batches. It obtains `FINITE_LOGS_WRITE_USERNAME` and
`FINITE_LOGS_WRITE_PASSWORD` through the existing log-write environment file.
The monitoring VPS receives neither a Postgres connection nor a Core service
credential. Limiter diagnostic writes have a bound of eight concurrent tasks.
Core reporting uses a 250 ms connection-acquisition budget and a two-second
SQL budget so telemetry cannot hold shared database capacity indefinitely.
Attribution uses trusted key-issue audit metadata at reservation time; later
key reissue cannot transfer prior requests to a new Project or Agent Runtime.
Ambiguous historical ownership stays unattributed.

Immutable request events use the `finite-private-request-diagnostics` stream.
The separate `finite-private-guard-status` stream contains current grant
snapshots. Read the latest snapshot per grant; never sum repeated snapshots.
Weekly usage is a rolling seven-day window. An inactive burst window has no
scheduled reset until another admission starts a new window.

The exporter publishes collection start, last successful export, failure
count, backlog size/age and guard-snapshot completeness through the existing
node-exporter/Alloy path. Failed collection must leave the last-success time
unchanged. Independent `finite-private-request-diagnostics-prune.service`
cleanup runs every ten minutes, including when inference and export are idle.
Postgres row cleanup therefore follows the seven-day visibility cutoff by up
to one scheduled interval under normal operation. Loki also applies its
compaction interval and two-hour physical deletion delay.

## Compatibility and recovery contract

Keep these combinations in the release verification matrix:

| Limiter | Core | Required behavior |
| --- | --- | --- |
| Previous | Updated | Admission and settlement behave as before. Request detail remains unavailable for observations the previous limiter never sent. |
| Updated | Previous | Existing admission and settlement routes continue to work. An unsupported diagnostic route is a telemetry failure and cannot fail inference. |
| Updated | Updated | The new diagnostic write follows accounting independently. Retrying it cannot change the event or extend retention. |
| Updated | Restored database | Accounting history remains available. Diagnostic tables restore empty and resume collecting new observations. |

An exporter failure leaves unacknowledged events eligible for retry. A push
that succeeds just before a process crash may be replayed with the same stream,
timestamp and canonical payload. Preserve that event identity when changing
the exporter; changing a replayed event's content can produce an additional
Loki entry. Show export failures, backlog and stale data independently from
inference outcomes.

Usage queries must also deduplicate by reservation ID. In Loki 3.5.8, an
ambiguous batch replay can leave raw `count_over_time` or `sum_over_time`
results inflated even when the log table displays each identical event once.
For request counts, unwrap a constant one and take its per-reservation
maximum before summing; for immutable token or usage values, take the
per-reservation maximum before summing. Reservation IDs exist only as
temporary query fields, never persistent stream or Prometheus labels. Preserve
this query contract when editing panels.

Cleanup must continue without new inference requests and without a working
exporter. Accounting rows, quota calculations and durable chat state are not
part of diagnostic cleanup. Keep rollback additive: disable collection or
revert compatible binaries while retaining the schema.

## Local verification

With a disposable Loki instance listening on loopback, run
`just monitoring private-request-integration http://127.0.0.1:3310`.
The test uses a scratch local Postgres database and the dashboard's actual
queries to verify export replay, token totals, diagnostic expiry, and a backup
restore that excludes diagnostic data while preserving accounting. It does
not establish live operator membership or production release compatibility.

## Production handoff

Implementation and local verification do not activate the production feature.
Do not add a new dashboard to the production manifest until its initial
file/UID ownership provisioning is authorized and verified. The existing
dashboard CI path updates owned files; it does not establish new ownership.

Before a separately authorized Production Deploy:

1. Run `scripts/finite-status` and retain the baseline. Verify the running Core,
   limiter and model revisions, measured release, and admission mode through
   the canonical status path; extend its read-only probe if needed. The
   deployment changelog records flash-5 restoring usage-api admission on
   2026-08-29. A historical degraded-mode document is not live evidence.
2. Verify operator access, telemetry credential scope, retained history and
   upstream usage fields. Record credential names and locations only.
3. Preserve the deployed artifacts/configuration and identify the current
   Core/Postgres backup boundary. Follow the
   [Postgres recovery runbook](../runbooks/postgres-backup-restore.md) for a
   restore; restoring an older database is not routine binary rollback.
4. Deploy compatible Core/reporting support before relying on new limiter
   fields. Prove old limiter/new Core and new limiter/old Core behavior and
   failures of diagnostic storage. Accounting and inference remain primary.
5. Publish the limiter through the existing digest-pinned image and measured
   Tinfoil release process. An enclave relaunch can interrupt inference and
   reload the model; the monitoring-only GPU rollout's blast radius does not
   apply. Preserve the exact previous measured release and credentials.
6. Activate collection and initially provision the dashboard. Verify counts,
   tokens, timings, attribution, stale states and detail expiry against
   controlled requests. Run `scripts/finite-status` again and compare.

Rollback must disable the new collection/display path without deleting
accounting records or changing Runaway Guard policy. Keep additive schemas
while reverting compatible binaries; destructive down-migrations are not the
normal rollback path. Restore the prior measured limiter release only through
the authorized Tinfoil procedure. Engine queue/cache instrumentation is deferred.
