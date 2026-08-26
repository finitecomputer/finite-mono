# Legacy Hermes migration retrospective

The first live migrations proved that a box1 Hermes bot can move to a normal
Kata Runtime without converting its legacy identity or asking an owner to sort
files one at a time. The source stays frozen and recoverable while the target
receives a complete sealed home snapshot, converted Hermes state, and only the
behaviors that have passed target-side checks.

User-specific evidence stays private. Names, account addresses, production
identifiers, hashes, timelines, message content, and detailed receipts belong
in the organization Brain or a mode-0700 operator evidence directory.

## What worked

- A whole-home inventory and snapshot preserved data that the original
  allow-list did not know about.
- Real Hermes v0.14 export and v0.20 import tests caught compatibility errors
  that synthetic current-version fixtures could not.
- Identity and Chat hash fences kept the fresh target identity separate from
  the legacy bot identity.
- Offline rehearsal, an empty-target restore, and retained source state made
  rollback concrete.
- Skills, jobs, credentials, Brain access, messaging channels, and Sites could
  be restored after the data move without mixing external ownership into the
  importer.
- The typed Core lifecycle and relocation transactions preserved the Project,
  Runtime, durable state, and Agent Principal across host recovery.

## Paper cuts that changed the runbook

- Compress the complete snapshot before transfer. Many small files can make a
  simple stream much slower than its byte count suggests.
- Runtime work roots differ by host. Derive the path from deployed host
  configuration.
- A dashboard action can outlive the browser response. Record the request and
  wait for Core and Runner instead of clicking again.
- Live Kata exec is not a safe verification path after a lifecycle fault. Use
  external health evidence or stop the Runtime and inspect it through an
  isolated scratch container.
- Sites authentication writes an adjacent lock file. Mount the identity file
  read-only inside a writable scratch home, not the whole identity directory.
- A Git push can be partly accepted before the client reports failure. Repair
  it with a new commit instead of replaying the same push.
- A Sites mailbox is not proof of SaaS Project ownership. Record the
  authoritative SaaS login separately and never ask a secondary mailbox to
  create another Agent.
- A skill can pass collision checks and still be wrong for the target image.
  Validate its binaries, paths, configuration, and credential locations before
  activation. If a live check finds stale assumptions, quarantine the complete
  skill with a verified rollback copy.
- Runtime health and channel health are different. Test fresh Chat, text,
  voice, image handling, and every restored external channel independently.
- Display defaults can change between Hermes versions. Record the owner's
  commentary and voice-transcript preferences rather than assuming legacy
  behavior.
- Generated Python environments appear under project-specific paths, not one
  fixed directory. Classify nested `dev/**/.venv` and `dev/**/venv` trees as
  rebuildable while keeping arbitrary escaping project links fail-closed.
- Restored homes can contain chains of absolute `/home/node` symlinks. Resolve
  each hop against the restored root; host-path resolution creates false
  escapes, while accepting a chain without following it can hide a real one.

## Accepted limits

- Legacy and v2 identities remain different.
- Cache-only media stays available in the Recovery Set but is not made active
  automatically.
- Unsupported dynamic Sites remain private until the target has a supported
  app runner. Static fallbacks may be added without discarding app source.
- Account-email transfer is separate from data migration. It needs one tested
  Core transaction that preserves billing, Project, Runtime, durable-state,
  Chat, and organization bindings.

## Evidence required for each migration

The private journal must bind the source and target identities, manifests,
backup and restore proofs, lifecycle receipts, imported counts, Sites and
integration outcomes, behavior changes, observation decision, rollback state,
and owner acceptance. It must also record every deviation from the generic
runbook and whether that deviation became a runbook change, product issue, or
accepted limitation.

The public repository records only reusable contracts, tests, and operational
lessons. It is not the migration ledger.
