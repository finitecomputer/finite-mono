# Rust quality audit: size, layering, and the Core store

Date: 2026-07-24

Status: **IMPLEMENTED LOCALLY — NOT RELEASED OR DEPLOYED**

Scope: workspace-wide Rust audit for performance problems, unnecessary
abstractions, poor organization, and redundant implementations. Requested
target was a 25% reduction in non-test lines.

This audit does not profile production, mutate production state, change a
database schema, or delete a product feature. `docs/audits/rust-hot-path-perf-audit-2026-07-23.md`
was read first; its findings are not repeated here.

## Measurement baseline

Non-test lines exclude `tests/`, `benches/`, `examples/`, `fuzz/`, and
brace-matched inline `#[cfg(test)]` items. The counter reconciles exactly
against `git ls-files '*.rs' | xargs wc -l` (259,841 total at `1774b20`).

| | lines |
|---|---|
| non-test | 167,706 |
| test | 92,135 |
| total | 259,841 |
| 25% target | remove 41,926 → 125,780 |

Largest non-test crates: `finite-saas-core` 27,035, `finite-saas-runner`
12,985, `finite-core` 11,827, `finitechat-core` 11,363, `finitechat-client`
9,002.

## Verdict on the 25% target

**Not reachable without deleting shipped functionality**, and the evidence
points the opposite way from where the size appears to come from.

Two things were tested and both came back negative:

- **Copy-paste duplication is low.** A normalized 20-line clone detector over
  production code only (inline `#[cfg(test)]` masked) finds its largest
  hotspot at 17 duplicated windows. The initial run looked far worse only
  because it was counting test fixtures.
- **Dead `pub` API is near zero.** Of 315 `pub` items in `finite-saas-core`,
  exactly 2 have no production reference.

Two dead-feature hypotheses were investigated at length and **both were
wrong**; see "Investigated and kept" below.

Delivered: **167,706 → 161,945 non-test lines (−5,761, 3.4%)**, all of it in
`finite-saas-core`, where it is 21% of the crate. Workspace tests unchanged at
1,575 passing.

The size result is modest. The reliability result is not: removing the second
implementation immediately exposed a defect affecting every timestamp the API
returns (Finding 6), which had been invisible for the entire life of the
row-native store because the fake it was tested against echoed inputs back
instead of storing them.

## Finding 1 — `--dry-run` previewed against an empty database (FIXED)

**Path:** `core_store_for_mode` in
`finitecomputer-v2/crates/finite-saas-core/src/main.rs`.

**Before:** `--dry-run` swapped Postgres for a fresh, empty in-memory store.
It never connected to the database, so it never read production state.

```rust
ImportMode::DryRun => Ok(CoreStore::memory()),   // fresh, EMPTY store
ImportMode::Commit => postgres_store_from_env().await,
```

**Impact, measured by running the commands:**

- The three mutate-existing commands failed unconditionally, for any input,
  because the target row cannot exist in an empty store:

  ```text
  API KEY REVOKE  --dry-run => Err(finite private api key is invalid)
  GRANT REVOKE    --dry-run => Err(finite private grant was not found)
  WINDOW RESET    --dry-run => Err(finite private grant was not found)
  ```

  An operator checking a valid production key ID before revoking it was told
  the key was invalid. This is break-glass tooling per `admin-ops-v0.md`, so a
  preview that always fails either blocks a correct operation or trains the
  operator to ignore dry-run output.

- The create/upsert commands ran, but computed their answer against an empty
  database. `reconcile-imports` branches on existing rows, so every record
  looked new; the existing test asserted `created_candidates.len() == 1`
  where production might have reported an update or a skip.

There were dry-run tests for exactly the three commands that happen to work on
an empty store, and none for the three that were broken.

**Fix:** `PostgresCoreStore` gained a `dry_run` flag. Every write path in the
impl ends at one `finish(tx)` helper that rolls back instead of committing, so
a dry run executes the real SQL against real rows and discards the write.
`--dry-run` now requires `FC_CORE_DATABASE_URL`, which is the point. Schema
DDL is skipped in dry-run mode because it commits outside the transaction.

All 51 commit sites in `impl PostgresCoreStore` route through `finish`, so a
later method cannot silently escape dry-run.

**Regression added:** `dry_run_revoke_reads_a_real_key_and_leaves_it_active`
seeds a real grant and key, previews the revoke (proving it reads real state),
then asserts via `finite_private_admin_state` that the key is still `active`.
`dry_run_reconcile_reports_creation_and_persists_nothing` runs the same record
twice and asserts a creation both times — if the first had committed, the
second would report an update.

## Finding 2 — a vestigial storage engine is now a second implementation

**Path:** `BridgeCoreState` in `lib.rs`, `MemoryCoreStore` and the `CoreStore`
enum in `store.rs`.

This is the single largest structural problem in the workspace, and its
history explains it.

`BridgeCoreState` was **not** built as a test double. It was the original
storage engine. At v2 init (`a6cfc5b`, 2026-07-02) it derived
`Serialize, Deserialize` and `PostgresCoreStore` was **412 lines** that worked
by `lock_state()` → `load_state()` → mutate a `BridgeCoreState` in memory →
`persist_state()` / `delete_missing_rows()`. Postgres stored the whole state;
`BridgeCoreState` *was* the data model. There was exactly one implementation
of the business logic, and that design was coherent.

Four days later, `7fa7ed6` (2026-07-06) — *"Phase 2c: row-scope finite-private
ops, partition agent-creation queue, DELETE global lock + full-state rewrite"*
— replaced the blob model with row-native SQL for concurrency. The guard test
`postgres_store_never_uses_full_state_persistence` exists to prevent
regressing to it.

That rewrite created a second implementation of the business logic in SQL and
left the first one standing. `BridgeCoreState` then grew **1,889 → 4,307
lines** *after* it stopped being the storage engine. Every feature since has
been written twice.

Current cost:

| component | lines | role |
|---|---|---|
| `impl BridgeCoreState` | 4,307 | in-memory reimplementation, 103 methods |
| `impl MemoryCoreStore` | ~600 | lock-and-delegate wrapper |
| `impl CoreStore` | ~750 | hand-written two-arm match dispatch |
| **total** | **~5,650** | |

The layering is four deep for every operation: `CoreStore` match →
`MemoryCoreStore` → `.lock().await` → `BridgeCoreState`, in parallel with
`CoreStore` match → `PostgresCoreStore` → one of 71 generic `postgres_*` free
functions.

**Reliability consequence.** ~70 tests (25 in `api.rs` via `CoreStore::memory()`,
~46 in `lib.rs` driving `BridgeCoreState` directly) exercise the fake, not the
SQL that runs in production. Nothing keeps the two honest: there is no
conformance suite running the same scenarios against both backends. Where they
disagree, tests are green and production is wrong.

That divergence is not hypothetical. Finding 3 was discovered precisely because
Finding 1's fix moved one command onto the real store.

**Blocker removed.** Before this audit, deleting the in-memory backend meant
deleting `--dry-run`. After Finding 1, dry-run is backed by real Postgres, so
the in-memory path has no remaining production caller. The remaining work is a
test migration, not an architecture change.

**DONE.** `BridgeCoreState`, `MemoryCoreStore`, and the `CoreStore` dispatch
enum are deleted; `PostgresCoreStore` is now simply `CoreStore`. All 58
in-memory tests were ported onto the shared Postgres harness with their
assertions intact, plus the 24 in `api.rs`. Typed readers on `CoreStore` reuse
the production `select_` / `*_from_row` pair so a test decodes rows exactly as
production does, rather than degrading typed assertions to JSON comparisons.

Two tests forked an in-memory snapshot, which a database cannot do. Both were
restructured rather than weakened: the archive fail-closed test now runs its
rejection paths in sequence (a rejection mutates nothing), and the
failure-then-cancel branch became its own test with its own database.

**Reduction: 5,761 non-test lines.** Workspace tests: 1,575 passed, 0 failed —
the same count as before, so no coverage was traded away.

**The port paid for itself immediately.** See Finding 6.

## Finding 3 — a non-atomic friend-key issue (FIXED)

**Path:** `finite_private_friend_key_issue` in `main.rs`.

The CLI approved a grant and issued its key as **two separate store calls**,
each its own transaction. If the key issue failed, the committed grant was left
orphaned. The store already had `admin_issue_finite_private_friend_key` doing
both in one transaction for the dashboard path, so the CLI was a redundant,
weaker reimplementation of an operation that already existed.

This surfaced only when Finding 1's fix moved dry-run onto real Postgres: the
rolled-back grant was invisible to the second call. That is exactly the class of
divergence Finding 2 predicts.

**Fix:** added `IssueFinitePrivateFriendKeyInput` and a single
`issue_finite_private_friend_key` store operation composing both steps against
one client, with the CLI's `project_id` / `agent_runtime_id` scoping that the
admin variant lacks. The CLI now calls it.

## Finding 4 — enum wire strings are spelled three times

**Path:** `lib.rs` lines ~6268–7110 in `finite-saas-core`, and 8 other files.

17 enums each declare their wire encoding three independent times: a
`#[serde(rename_all = "snake_case")]` derive, a hand-written `as_str()`, and a
`parse_*()` free function. Example:

```rust
#[serde(rename_all = "snake_case")]
pub enum BillingSubscriptionStatus { Incomplete, IncompleteExpired, ... }

impl BillingSubscriptionStatus {
    pub fn as_str(self) -> &'static str {
        match self { Self::IncompleteExpired => "incomplete_expired", ... }
    }
}

pub fn parse_billing_subscription_status(value: &str) -> Option<...> { ... }
```

The three must agree, and nothing enforces it. Adding a variant and updating
two of three sites yields a silent encoding mismatch between the JSON API and
the database column.

**DONE.** `wire_enum!` takes one `variant => string` list and generates the
enum, its serde impls, `as_str`, and `parse_*`. Adding a variant is one line in
one place.

Sequencing mattered: `enum_serde_as_str_and_parse_encodings_agree` was added
*first*, as the safety net, and verified non-vacuous by breaking one arm
(`past_due` → `pastdue`) and watching it fail. All 53 variants already agreed,
so the consolidation changed no encoding — and the test proves it.

**Reduction: 257 non-test lines.**

## Finding 6 — Core emitted timestamps it could not parse (FIXED)

Found by Finding 2's port, and invisible before it: the in-memory store echoed
request strings back verbatim, so no test ever compared a *stored* timestamp
against the API contract.

**159 TIMESTAMPTZ columns were read with a bare `::text` cast.** That renders
Postgres's own display format in the **server's** timezone:

```text
created_at: "2026-05-25 07:00:00-05"   ← what the API returned
created_at: "2026-05-25T12:00:00Z"     ← what the contract says
```

`parse_time` rejects the first with `InvalidTimestamp`, so those values did not
round-trip: Core emitted timestamps it could not itself read back. This was not
cosmetic and not test-only — `Project.created_at` on `/api/core/v1/me/projects`
is consumed by the Dashboard, and the value is wrong in both format *and*
apparent wall-clock whenever the server timezone is not UTC.

The same table's SELECT path already used an `rfc3339_col` helper, so one
struct was populated in two different formats depending on which query ran.

**Fix:** one `core_rfc3339` SQL function (migration `0016_rfc3339_reads.sql`).
Reads go through it instead of casting. It trims trailing zeros in the
fractional second so a value round-trips byte-for-byte with the RFC3339 string
Rust wrote.

**Also found by the port, not fixed:** a nonexistent `projectId` on
finite-private key issue returns 500 rather than a client error. `project_id`
and `agent_runtime_id` are foreign keys; the fake enforced no referential
integrity, so tests passed fabricated ids for the life of the crate.

## Finding 5 — organization: single files carrying whole subsystems

| file | lines |
|---|---|
| `finitechat-core/src/lib.rs` | 19,116 |
| `finite-saas-core/src/store.rs` | 13,372 |
| `finite-saas-core/src/lib.rs` | 13,249 |
| `finitechat-client/src/lib.rs` | 10,523 |

`finitechat-core` is 19,116 lines across effectively one file (the crate's only
other module is 398 lines), holding 290 non-test functions.

Splitting these into modules **does not reduce line count** and should not be
counted toward a size goal. It is worth doing for navigability and review
surface, but it is a separate, low-risk, mechanical change.

## Investigated and kept

Two dead-feature hypotheses looked strong and were both refuted. Recording them
so the next audit does not re-litigate them.

**Project-import subsystem (~1,605 lines of import-named functions).** No UI
renders `claimable_candidates` — the only dashboard value is `[]` in a test
fixture — and no runbook invokes `reconcile-imports`. But
`docs/legacy-migration-feasibility-2026-07-23.md` (opened 2026-07-23) names the
Core reconciliation path as still potentially providing *"deterministic
owner/Agent mapping for dry runs and cutover preparation"* and cites those very
tests as passing evidence. **Keep.** Finding 1 makes its dry runs meaningful
for the first time.

**Admin CLI (13 subcommands).** An initial search of `infra/ scripts/ docs/
.github/` returned zero callers. That search was wrong: it missed
`finitecomputer-v2/docs/`, `devfinity/`, and three subcommands. Actual callers:

| command | caller |
|---|---|
| `runtime-artifact-rollout` | `scripts/rollout-lat1-runtime-artifact` |
| `runtime-artifact-upsert` | `devfinity/src/lib.rs` |
| `runtime-retire-exact` | `docs/runs/runtime-retirement-readiness.md` |
| `runtime-archive-unrecoverable` | same runbook |
| `finite-private-friend-key-issue` | `admin-ops-v0.md`, used in practice |
| `finite-private-window-reset` | `admin-ops-v0.md` |

`admin-ops-v0.md:77` states the CLI subcommands *"remain as the break-glass
path"*. The commands without direct references are the documented fallback for
when the dashboard path fails, not dead code. **Keep all.**

## Checked and healthy

- **N+1 queries:** none in `finite-saas-core`, `finitesites-store`, or
  `finite-brain-store` store layers. The only awaited queries inside loops are
  in test harnesses.
- **Perf finding N1 from the 2026-07-23 audit is already fixed.** That audit
  describes `PostgresCoreStore.client: Arc<Mutex<Client>>` serializing all
  database work; the current code uses a deadpool `Pool` and carries a
  `postgres_pool_does_not_head_of_line_block_independent_reads` regression.
- **Dead `pub` surface:** 2 of 315 in `finite-saas-core`.

## Prioritized plan for the remaining reduction

1. ~~Delete the in-memory Core backend~~ — **done**, −5,761 lines.
2. ~~Consolidate enum codecs~~ — **done**, −257 lines.
3. **Apply the same reachability archaeology to `finite-core` (11,827) and
   `finite-saas-runner` (12,985).** Both predate the restart. The method that
   worked here: trace from real entry points — Dashboard `fetch` calls, runbook
   command names, `devfinity` process definitions — rather than from Rust
   references, which stay internally consistent even when a whole feature is
   unreachable.
4. **Return the 500-on-bad-foreign-key to a client error** (Finding 6).
5. **Split the four largest files into modules.** Zero line reduction; do it
   for review surface, and not as part of a size goal.

Items 1 and 2 are spent. A further 25% is not available from item 5, which
removes nothing. Item 3 is the only place a large number could still be
hiding, and it should be measured before it is promised.

## Observed, not caused: one timing-sensitive test

`kata::tests::kata_upgrade_retry_recovers_crash_after_canonical_moved_to_rollback`
in `finite-saas-runner` failed once in roughly six full-workspace runs and
passed 6/6 in isolation. `finite-saas-runner` is untouched by this work (empty
diff against `origin/main`), the test uses no Core store, and its launcher
polls readiness against `readiness_timeout` with `Instant::elapsed`. It is a
pre-existing timing sensitivity that shows up when a full parallel run
saturates the machine, not a regression from these changes. Worth a deflake
before it erodes trust in CI.

## Method note

Two heuristics were tried and abandoned during this work; both are recorded so
the next pass does not repeat them.

**Operation-name overlap is not coverage.** Classifying the in-memory tests by
"does this touch an operation no Postgres test covers" cleanly split them
31 port / 28 delete. Spot-checking the largest deletion candidate showed the
in-memory ledger test carried 37 assertions and 19 transition cases against its
Postgres counterpart's 5 and 5 — same subject, different depth. Deleting it
would have dropped real coverage. The two suites are complementary by
construction: `BridgeCoreState` was the original engine, so the behavioral
suite was written against it, while the `postgres_*` tests were added later by
Phase 2c for row-native concerns.

**Do not translate typed assertions into JSON.** The first port attempt
rewrote collection reads to `to_jsonb`, which would have silently converted
~200 typed assertions into string comparisons. Exposing the production
`select_` functions as typed test readers keeps assertions exactly as written.
