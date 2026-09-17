# Runtime validation boundaries

Tests use the canonical pinned image and preserve the same Principal, Device,
Room and durable state across restart. Use `just dev saas-smoke` for the local
platform and the [image runbook](../../infra/runbooks/runtime-image.md) for
artifact qualification and authorized canaries. A stub adapter proves only its
local boundary; it cannot establish provider behavior or real model delivery.

## Required regression coverage

| Boundary | Proof |
| --- | --- |
| Resident Chat bridge | Real Rust process and inbound stream; bounded reconnect, no subprocess/polling fallback |
| Chat/Hosted Device outage | Reopen the same stores and resume ordered history without re-enrollment |
| Hermes interruption | Unacknowledged input recovers; acknowledged input is not duplicated |
| Runtime restart/image replacement | Preserve Principal, Room, `/data`, memory, workspace and user configuration |
| Core outage | Existing Chat and Hermes work continues; health reporting recovers separately |
| Skills | Fresh agents receive the image baseline; existing agents change only after explicit sync; user skills survive |
| Provider lifecycle | Exact-host leasing, stale-worker fencing, durable handle adoption and capability checks |
| Connections | Real authorized operation after setup; requester isolation and preserved state across restart |

For existing-state changes, exercise the actual supported older writer and
candidate reader, including rollback readers where promised. All-candidate
fixtures do not qualify a persisted format or provider upgrade.

## Recovery qualification

A Provider Durable Volume, restarted process, relocated tree or successful
archive upload is not an independent Recovery Set proof. Recovery qualification
restores onto an empty target with independently held keys, preserves readable
history and fences all old writers. Destructive drills require explicit
authorization and isolated targets. See [Hosted Web Chat recovery](../../infra/runbooks/hosted-web-chat-recovery.md).

TODO: broader Agent recovery qualification is tracked in
[FIN-62](https://linear.app/finitecomputer/issue/FIN-62).
