# FIN-37: direct Connections control

Status: local spike, not a production cutover or completion of FIN-37.

Connections must remain manageable when Finite Chat is unavailable. Use direct
HTTPS for the current hosted use cases. Keep agent-local settings and effects in
agentd, account/Project authorization in Core, and routing/compute in the Runner.
No new Python service, generic message bus, connector framework, desired-state
store, or WireGuard dependency is necessary for this experiment.

Austin's “Connections Management — Today vs. Desired State” is design input,
not an implementation specification. Outbound polling and schema-driven UI are
separate proposals; this spike does not implement them.

## Current proof

`ConnectionControl` owns the existing inference, Telegram, Google, and SimpleX
operations, including configuration validation and rollback. The existing chat
adapter calls the same implementation. Its wire envelopes, authorization,
request fingerprints, durable results, and startup behavior stay in that adapter.
Raw JSON bodies preserve typed deserialization, including duplicate-field errors.

`finite-agentd control-serve` starts a separate *mode of the same executable*.
It does not load `DaemonConfig`, call the chat preparation script, load a Device
identity, seed admission, or start a chat sidecar/health process. It supervises
only the supplied foreground Hermes launcher. Run it on scratch state instead
of `serve`; concurrent writers to the same home are not supported by this spike.
The production entrypoint continues using `serve`.

The service owns its entire public router:

- `GET /v1/runtimes/{runtime_id}/connections`: current redacted status.
- `POST /v1/runtimes/{runtime_id}/connections/commands`: current typed mutations.

POST body: `request_id`, `command`, `schema`, and JSON `body`. Existing command
and schema names are retained; there are no room, Device, or owner-claim fields.
Only the ten existing mutation commands are accepted. POST returns an `ok`
result; callers must inspect it, not treat HTTP 200 alone as operation success.
Transport/authentication/conflict failures use HTTP error status codes.

A runtime-specific bearer credential is loaded from a private file. It is not a
chat key or a Hermes Desktop token. The listener accepts loopback only; a local
TLS proxy forwards its whole router. No query-string credentials are accepted.
Atomic token-file replacement revokes the previous credential for subsequent
requests. Already accepted operations finish. Removing or making the file
readable by other users fails closed. Production provisioning is still missing.

Operations serialize with a bounded admission gate (busy returns 409). Client
disconnect does not cancel a mutation. Completed requests replay their durable
result; reuse with different bytes fails. If the process dies after an external
effect but before recording the outcome, the request fails closed on retry.
There is no claim of exactly-once provider effects. Inspect the current state
before issuing a new operation after an unknown outcome.

## State and compatibility boundary

The existing `agentd.sqlite3` remains authoritative for config ownership/history
and legacy authorized principals/command delivery. Only control mode creates an
additive `control_requests` table containing request IDs, hashes, and redacted
results. It stores no request bodies or bearer credentials. HTTPS configuration
proposal IDs are prefixed `https-`; legacy ledger rows retain their original IDs
and bytes. Existing config files and credentials are written by the same
ConnectionManager/ConfigManager methods and read by the same Hermes integrations.

Tests cover fresh state and an old-schema fixture with existing principal and
command rows. This is **not** proof of a complete production mixed-version
rollout, old-runtime routing, or recovery. No production state has been migrated.
Switching back to an old binary ignores the additive table, but cannot undo an
intentional connection change or provider-side revocation.

## Local evaluation

Run from the pinned dev environment:

```sh
scripts/with-dev-env cargo test -p finite-agentd --locked
scripts/with-dev-env cargo clippy -p finite-agentd --all-targets --locked -- -D warnings
```

For the real TLS integration test, set `FINITE_AGENTD_TEST_CADDY` to the pinned
Caddy executable and run:

```sh
scripts/with-dev-env cargo test -p finite-agentd --test direct_control --locked -- --include-ignored
```

The test creates its own temporary CA and verifies the server certificate.
It does not install a trust root or use insecure certificate bypasses. All
servers and data are local and temporary. The TLS mutation is Google disconnect;
this proves the transport and existing local effect, not live Google OAuth,
Telegram pairing, inference activation, or SimpleX reset against real services.
The supervised launcher in these tests is a process fixture, not a live Hermes
session. Existing agentd tests cover configuration rollback and supervision.

Manual experimentation uses `control-serve --listen 127.0.0.1:37634
--runtime-id <runtime> --token-file <private-file> --agent-home <scratch-agent>
--hermes-home <scratch-hermes> --hermes-command <foreground-launcher>`.
Generate a unique random credential of at least 32 bytes; never pass it on the
command line or commit it. The launcher must start Hermes without the existing
chat-dependent `run_hermes_gateway.sh`. No image or host enables this mode yet.

## Landing stack and exit gates

1. **Extract Connections operations.** Behavior-preserving base PR; no listener
   and no deployment changes. Can land independently of the experiment.
2. **Direct control spike (this document).** Stacked on the extraction. Keep
   draft until the next production boundaries are implemented and reviewed.
3. **Hosted routing, authority, and FIN-37 dashboard cutover.** Core verifies the
   current account/Project relationship and resolves the authoritative runtime;
   it must not acquire feature schemas or edit runtime files. Provision a
   separate revocable credential per runtime. Use `agents.lat3.finite.computer`
   (and corresponding hosts) with runner-local routing, never the private mesh.
   The production Runner is a oneshot reconciliation service, not an existing
   HTTP daemon: choose and implement routing explicitly, rather than assuming a
   listener is already present. Prefer integrating route publication into its
   existing reconciliation over adding another inventory polling service.
   Replace hosted-agent-controls' chat binding/owner-claim delivery with this
   authorized direct path. No automatic fallback through chat. Runtime startup
   must expose control even when chat bootstrap fails; retain current chat
   availability for users until the later cutover. One agentd process and one
   mutation owner must serve both paths during transition.
4. **Simplify #858 onto that path.** Hidden admin toggle plus separate Hermes
   Desktop connection credentials. Remove its new chat commands and replace the
   provisional Python inventory/proxy/ingress/launcher layering. Reuse the
   runner routing established above. Its existing diff is not a prerequisite
   for FIN-37 and should not land in its current form.
5. **Finish #845 and FIN-39.** Hot-reloading UI against real agents through the
   admin capability; native Desktop connect, reconnect, rotation and revocation;
   then user-facing Hermes chat cutover. Only remove Finite Chat after proving
   existing conversation access/recovery and every remaining dependency.

The first two branches form the actual `gh stack`. Add later branches only when
they contain their real dependency, rather than changing unrelated PR bases to
make a cosmetic stack. Keep #858 and #845 open as downstream work until rebased
and simplified. Each independently landable PR needs an explicit activation
boundary; don't merge half a live transport migration.

Before declaring FIN-37 done: prove new-agent and existing-agent dashboard
status and every current operation with chat unavailable; unauthorized and
cross-runtime requests; credential provisioning/rotation/retirement; stale
routing after replacement; new Core with old runtimes and rollback; loss of the
HTTP response after a real effect; settings/history preservation; and restoration
onto empty state. Use the canonical runtime image and the existing fleet-status
command for any later rollout. Deployment needs separate authorization.
