# Runtime health telemetry boundary

Runtime Management Pipe is outbound generic health and release telemetry.
Core owns desired lifecycle state; Runner owns compute operations. Product
features, signing, credentials, chat, skills and recovery are not RMP commands.

Current readiness observation is Runner-ferried: the Runner reads the guest's
`/contact` using a bounded, throttled, npub-pinned probe and posts standing
readiness reports to Core. See [runtime control](runtime-control-contract.md).
An unavailable observation must not be represented as healthy or authorize a
replacement Runtime.

TODO: a closed in-image RMP client and its conformance proof are tracked with
[runtime lifecycle](https://linear.app/finitecomputer/issue/FIN-21). Existing
heartbeat scaffolding does not establish that protocol's implementation.
