# Finite Private main-route migration validation

This record summarizes the completed hosted main-chat model migration and a
subsequent read-only evidence refresh. It contains aggregate conclusions and
reusable validation requirements. Detailed account mappings, source ownership,
per-bot results, recovery locations, and raw reports remain in protected
operator records outside Git.

## Scope and completion

The inspected active hosted cohort has its main conversation model configured
as `glm-5-3-flash` through Finite Private. The operation changed the required
model fields and, where necessary, delivered an account-owned credential.
Existing auxiliary, delegated, fallback, and session-specific settings were
preserved. Completion therefore means main-model configuration; it does not
assert that every inference request uses Finite Private.

The model switch retained the source host, deployed runtime image, persistent
volume, workload identity, and conversation stores. It did not move bots to a
different compute host. A later host migration is a separate operation with
its own source/target ownership and recovery evidence.

## Validation retained from execution

- Selected active sources were matched to their persistent volumes and
  account-owned credentials. Ambiguous or missing prerequisites deferred the
  individual target. Retired source copies were excluded.
- The exact deployed packages were rehearsed with isolated synthetic prior
  conversation/tool history. The candidate model mapping was checked for
  preservation of unrelated configuration and exact config rollback.
- Each change guarded the source configuration hash, waited for an idle
  gateway, used the deployed launcher's supported draining lifecycle, and
  resumed the existing supervisor. Workloads were not recreated.
- Actual restarted gateway processes loaded the expected credentials. The
  deployed provider resolver selected the intended model, endpoint, and API
  mode. Configured channel states were inspected.
- Isolated continuation and harmless-tool checks completed, and their
  verification requests settled with actual usage. These probes used temporary
  homes without customer channel adapters; no operator test messages were
  sent into customer conversations.
- Final audits matched candidate configs, exact original backups, config
  owner/mode, workload and volume identities, container identities, and
  restart counts against the retained preparation evidence.

These checks establish the recorded migration behavior. They do not prove a
newly delivered real-user reply for every bot or all possible history, media,
integration, and session-override paths.

## Current evidence boundaries

The read-only refresh reconfirmed the main-route configuration for the active
cohort. For the legacy cohort examined in detail, configured and live-process
credentials matched active account-owned credentials, and model configuration,
backups, and workload identities matched the execution records.

Operational observations have changed since execution. Inspected channel state
includes a disconnected adapter, some credential-input bytes differ from their
original installed hashes, and normal-traffic accounting includes unresolved
reservations and estimated settlements alongside actual settlements. A
previously recorded unresolved reservation also remains unresolved. No cause
or customer-delivery outcome is inferred from those observations. No repair,
restart, credential issuance, or new inference test was performed during the
refresh.

Shared inference readiness reports the expected GLM model, thinking defaults,
and normal usage admission on the same serving release. Canonical platform
status provides healthy chat and recovery observations alongside a convergence
finding and collection gaps. This record does not assert that the entire
platform or every channel is healthy. A missing local rollout record is not
proof that no other operator is making a change.

## Capacity evidence

Retained GLM testing completed progressively larger short synthetic bursts
through 32 concurrent requests, with successful recovery checks, no test
request errors, and actual usage settlement for all verification traffic.
The largest burst recorded less than one second p95 first output and more
than 50 output tokens per second median decode per request.

Those results describe the tested workload on the recorded serving release.
First output includes reasoning and is not time to a useful visible answer.
They do not establish sustained, long-history, maximum-context, or full-fleet
capacity. Historical tests of another model are not interchangeable GLM proof.
The owner's accepted concurrency investigation is not reopened by this model
migration record. The read-only refresh did not rerun benchmarks.

## Minimal procedure for a newly identified target

1. Establish the exact active source, ownership, current model/config hash,
   effective credential, and absence of a competing channel consumer. Confirm
   current platform status using the canonical status command.
2. If the main route already matches, make no change. Otherwise save the exact
   original configuration and reuse the deployed-version proof, filling only
   target-specific compatibility gaps.
3. Change only required model fields. Include credential delivery only when
   that target requires it and its account ownership is established.
4. Wait for active work to finish and use the target's actual draining gateway
   lifecycle. Legacy versions use different signals; do not substitute a
   signal solely because another host used it.
5. Verify the effective route/key, continued synthetic history/tool behavior,
   expected channel state, and usage settlement before proceeding.
6. On failure, stop progression. Restore only the guarded configuration/input
   change after checking for intervening edits. Never restore an entire home
   or conversation database to undo a model selection.

Existing backups remain recovery evidence, not permission to overwrite newer
state. The supported restart must be checked again if the deployed image or
launcher has changed. Missing credentials, ownership ambiguity, unexpected
config drift, or unsupported compatibility defer the individual target.

## Publication boundary

Keep customer and Agent names, contact information, handles, account and
runtime identifiers, credential hashes, per-account counts or timestamps,
private content, infrastructure bindings, and sensitive paths out of this
document, commit messages, PR text, and review comments. Preserve detailed
evidence only in access-controlled operator storage. Public summaries should
retain validation methods, aggregate outcomes, limitations, and rollback
contracts without making an individual customer identifiable.

This documentation does not authorize another production operation.
