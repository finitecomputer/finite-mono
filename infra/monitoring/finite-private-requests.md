# Finite Private requests and usage

[Issue #946](https://github.com/finitecomputer/finite-mono/issues/946) defines
this Internal Operator dashboard. It supplements GPU health with request
traffic, tokens, timings, and seven-day diagnostic metadata.

## Measurement contract

- Count all inference requests that reach the limiter, including refusals and
  degraded admission. Exclude health and metrics probes. Arrival, admission,
  and termination are distinct events; active requests are live work, not
  unsettled reservations.
- The detail table covers Core-reserved requests only. Refusals and degraded
  traffic appear in aggregate counts without individual rows. Project, Agent
  Runtime, key, and usage-user filters apply only to trusted Core attribution. All-traffic
  panels explicitly ignore these filters.
- Retain input/output tokens only when upstream reports them. Missing usage
  is unavailable, not zero or an accounting estimate. Do not add overlapping
  cached/reasoning-token subfields to the reported totals. Validate the live
  model's usage schema before activation; historical tokens cannot be backfilled.
- First-output delay ends at meaningful reasoning, answer, or tool-argument
  output; first-answer delay ends at answer text. Ignore headers, keepalives,
  role events, and tool names without arguments. Nonstreaming requests have
  duration only. Detect boundaries without storing content.
- Timings start at limiter request acceptance and stop before settlement
  retries. They measure limiter observations, not client delivery or exact
  engine token timestamps. Output tokens per wall-clock second is aggregate
  throughput; per-request decode speed remains unavailable.
- Target visibility within one minute after an observation becomes available.
  Final token counts arrive at termination, not while usage is still pending.
  Show missing data, collection age, export failures, and backlog explicitly.

The accounting ledger and Finite Private Runaway Guard remain authoritative.
Diagnostics do not copy grant balances, accounting state, or settled units.
Current guard/reset views, aging reservations, engine queue/cache metrics,
alerts, and customer billing are outside this delivery.

## Usage user attribution

Core snapshots the charged grant's opaque `user_id` into each new reservation's
`usage_user_id` in the admission transaction. Diagnostics copy that snapshot;
exported events carry `usageUserId` and `userAttribution=reservation_grant`.
Key reassignment, later ownership changes, and diagnostic retries cannot move
that recorded consumption to another user. This identifies the responsible
account, not the human initiating a turn: scheduled and background agent work
belongs to the same usage account.

For Internal Operators, diagnostics also snapshot the account's current email
and the attributed Project's current display name when the diagnostic is first
written. These are readable labels, not ownership authorities. They follow the
same seven-day diagnostic retention and backup exclusions; they are not added
to the durable reservation ledger. Export reads only these frozen diagnostic
fields, so renames cannot change an event on replay. Credentials and message
content are never exported.

Reservations from before collection or a previous Core binary have a null
snapshot. Their events carry null `usageUserId` and `userAttribution=unknown`;
existing Loki events without these fields are grouped as `unknown` too. There
is no current-ownership join or historical backfill. Account-only keys can have
known user attribution even when Project and runtime attribution are absent.

The Usage user filter applies to all retained-event panels, including request
counts and input/output tokens grouped by user. It never filters all-traffic
Prometheus panels. These are best-effort diagnostic totals, not a complete
billing ledger or a count of active people. Per-user IDs remain event fields,
not Prometheus or persistent Loki stream labels.

The **Who is using agents?** table shows account email, agent/Project name,
request count, last recorded request time, and attribution level for the selected
time range. Its count and last-time columns group by stable IDs and their recorded
display labels; a renamed account or Project can have separate historical rows.
The existing per-user total panels group only by stable user ID. Account-only
rows say **Not recorded** for the agent instead of guessing from current
ownership. The account link opens that user's request details. No snapshot means
**Unknown** or an opaque ID, including historical events without label fields.

## Access and data path

Verify that Grafana users, organization membership, roles, and datasource
permissions admit Internal Operators only. Disabling anonymous access alone
is insufficient. There is no customer API or shared dashboard snapshot.

The limiter exposes request metrics only with the dedicated
`FINITE_PRIVATE_METRICS_TOKEN`. Inference, model, and accounting credentials
must not authorize metric reads. Unconfigured deployments retain the previous
liveness-only response. Prometheus reads the matching credential from
`/etc/finite/monitoring/private-limiter-metrics-token` and scrapes every 15
seconds. Store values in the existing secret mechanism, never in source.

The limiter posts best-effort diagnostics to the service-authenticated Core
write route, which returns an acknowledgment. Eight background slots bound
limiter work; Core limits connection acquisition to 250 ms, SQL to two seconds,
and lock waits to 250 ms. Unsupported routes, full slots, and storage failures
must not affect admission, settlement, response bytes, or chat availability.

Core stamps immutable events with its own clock and derives attribution from
key-issue audit metadata at reservation time. Reissuing a key cannot transfer
old requests to its new owner; ambiguous ownership remains unattributed.
Retries cannot overwrite an event or extend its lifetime. No raw keys,
prompts, responses, or arbitrary headers are retained.

On `finite-lat-2`, the [exporter](private_requests/export.py) sends at most four
500-row batches per run, every 15 seconds, to the existing Loki ingress. Its
local database role can read diagnostics and update only export acknowledgments;
the same role prunes expired diagnostics. The exporter uses the existing
`FINITE_LOGS_WRITE_USERNAME`/`FINITE_LOGS_WRITE_PASSWORD` environment file.
The monitoring host receives no database or Core service credential.

The `finite-private-request-diagnostics` Loki stream is Grafana's only detail
source; there is no Core reporting API. Prometheus labels are bounded; request,
key, Project, and runtime IDs belong only in event fields. The table shows the
newest 1,000 matches; narrow its range and filters to inspect more.

## Retention and recovery

Diagnostic storage ships with its cleanup timer and backup exclusions. Activate
these controls before deploying the writer:

- Independent Postgres cleanup runs every ten minutes, including when requests
  or exports stop. Queries hide details older than seven days. Normal physical
  row removal follows by up to one cleanup interval.
- Both repository-managed Core dump paths exclude diagnostic **data**, retaining
  its schema and all accounting data. Later deletion cannot remove rows from
  an older dump or Borg archive; exclusions must precede diagnostic writes.
- The dedicated Loki stream uses seven-day retention instead of the general
  fourteen-day policy. Compaction and the two-hour deletion delay govern physical
  removal. Do not journal event payloads or create another spool.
- Check provider snapshots and other external copies during rollout; their
  retention cannot be established from this repository.
- Prometheus keeps aggregate history for up to 15 days, subject to its storage
  cap. Key/Project breakdowns follow the seven-day event window.

| Limiter | Core | Expected behavior |
| --- | --- | --- |
| Previous | Updated | Existing admission/settlement work; new details are unavailable. |
| Updated | Previous | Missing diagnostic route counts as telemetry failure; inference continues. |
| Updated | Updated | One immutable diagnostic per Core reservation, independent of accounting. |
| Updated | Restored | Accounting survives; diagnostics restore empty and resume on new requests. |

Loki pushes are acknowledged in Postgres only after success. An ambiguous
push may replay the same timestamp and canonical payload. Log queries suppress
identical duplicates, but Loki 3.5.8 metric sums can still double-count them.
Usage queries therefore take one `last_over_time` value per reservation before
summing; counts unwrap a constant one and apply the same rule. Do not use
`max_over_time`: Loki splits instant queries longer than one hour and merges a
pushed-down `sum` of `max_over_time` splits with `max`, reporting the busiest
hour instead of the window total. Loki does not split `last_over_time`. IDs are
temporary query fields, not persistent stream labels. Preserve this contract
when changing queries.
Grafana instant metric tables need the rows-to-fields transform to show every
returned key/Project instead of reducing a result to one bar.

Run `just monitoring private-request-integration http://127.0.0.1:3310` with a
disposable Loki instance. It uses real scratch Postgres to prove batch replay,
1h/24h/72h totals for every Loki instant panel, expiry, and restore with intact
accounting and empty diagnostics.

## Separate production activation

For usage-user attribution, pause the exporter timer and let any active export
finish successfully before upgrading Core. Then deploy Core's additive
migration/writer, the exporter query, and its narrow column-level grants for
`usage_user_id`, `usage_user_email`, and `project_display_name`, before resuming
export. This prevents the old exporter from
sending a new attributed row without its user field and later replaying that
same reservation with a different grouping. Investigate an interrupted or failed
export before proceeding; a remote success without its local acknowledgment is
ambiguous. No event rewrite or re-export of acknowledged history is required.

Existing limiters work with the new Core; old Core binaries can still insert
reservations with null attribution. The dashboard accepts both old and new event
shapes. For rollback, prefer reverting Core while keeping the new exporter;
retained snapshots continue to export correctly. If reverting the exporter too,
first drain its outbox successfully, pause its timer, and wait for the active
export to finish. Retain the additive nullable columns; do not drop them or rewrite old events.
The reservation snapshot follows the existing accounting backup policy, while
the diagnostic copy remains subject to seven-day expiry and dump exclusions.
No token retention extension or production backfill is implied.

Code review and merge do not authorize a Production Deploy. The dependency
order is accounting safety, measurements/storage with retention controls,
then exporter/Grafana configuration. The new dashboard stays outside the
production manifest until its file/UID ownership is provisioned.

1. Run `scripts/finite-status`; record the running Core, limiter/model release,
   admission mode, operator access, and upstream usage schema.
2. Preserve the deployed artifacts/configuration and backup boundary. Activate
   diagnostic backup exclusions and cleanup before the writer. Prove the
   mixed-version combinations above and check external snapshot retention.
3. Provision the dedicated metric credential and compatible Core/reporting
   support. Publish the limiter through the existing digest-pinned image and
   measured Tinfoil release process. An enclave relaunch can interrupt inference;
   this has a different blast radius from the GPU dashboard rollout.
4. Activate collection and initial dashboard ownership. Reconcile a controlled
   request across source, counters, and detail; check stale/missing states and
   expiry. Run `scripts/finite-status` again.

Rollback disables collection or reverts compatible binaries while retaining
the additive schema and accounting records. Preserve the exact previous measured
limiter release. Database restoration follows the [Postgres recovery runbook](../runbooks/postgres-backup-restore.md), not a destructive down-migration.
