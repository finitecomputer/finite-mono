# ADR 0005: finite-lat hosts have explicit roles and safe initial placement

Status: accepted, 2026-07-20.

## Context

`finite-lat-1` currently combines the Finite control/app plane with existing
Kata Agent compute. A workload spike can therefore threaten Core, Postgres,
Chat, Sites, Brain, and Search. It also has single-disk root and `/data`.

`finite-lat-2` has healthy root and data RAID1, but it runs Finite's GitHub
Actions runners and is the only approved x86_64 production Nix builder.
Reinstalling it or adding Agent workloads would combine two scarce failure
domains and remove the known-good builder during the riskiest storage work.

Runtime placement is already expressed in provider-neutral hosting and
resource classes, and Core queues can be partitioned by source host. Current
Runner capacity is host/Runner-wide rather than resource-class-specific, and
the product path does not yet select a source host. Two undrained Runners would
therefore race for untargeted creation work. Cross-host Core and Runtime
contact also require a private non-loopback path.

## Decision

- `finite-lat-1` remains the control/app plane and lifecycle controller for
  existing Agents. After a proven handoff, it is drained for new creation but
  continues lifecycle operations for Runtimes already bound to it.
- `finite-lat-2` remains dedicated to CI and x86_64 production builds. It is
  not reinstalled, used for production Agents, or made the sole recovery
  target in this capacity/redundancy work.
- `finite-lat-3` is the blank-slate NixOS/RAID canary and then the sole creator
  for bounded new Standard Agents during the initial rollout.
- During the lat3 rollout, exactly one Standard Runner may accept new creation.
  A lat3 outage fails new admission closed; it does not automatically undrain
  lat1. This restriction can be relaxed for later qualified hosts only after
  every reservation persistently selects a source host and each Runner claims
  only work targeted to itself.
- Existing Agents remain bound to their source host. No automatic failover,
  migration, or opportunistic first-claim placement is introduced.
- Checkout, Launch Code redemption, and entitled Agent creation require a
  durable, transactional capacity reservation. Account signup/login and
  access to existing Agents remain available when capacity is full.
- Capacity observations and claims attach to an authoritative physical pool.
  A host-wide limit is not duplicated for every allowed resource class, and
  the effective cap is the smaller of operator policy and Runner report.
- The first accepted lat3 cap is at most six Standard Agents, promoted only
  through measured cohorts of one, four, and six after Kata proves the
  4-vCPU/8-GiB Standard contract. A larger value is a separate CPU-
  oversubscription decision, not capacity inferred from RAM or swap.
- Product admission stays generic over `HostingTier`, `RunnerClass`,
  `RuntimeResourceClass`, and source-host identity. Latitude/Kata and future
  Phala/TEE adapters implement the same boundary; provider names do not enter
  checkout or dashboard policy.
- Host-to-host Runner and Runtime traffic uses a WireGuard overlay over the
  two fixed public endpoints initially. Core Runner routes and Runtime ports
  are bound to and firewalled on the overlay, not exposed publicly. A future
  provider VLAN may change the underlay but does not replace authenticated
  WireGuard.

## Consequences

While queues are untargeted, adding capacity is an explicit operation: qualify
a host, register its provider-neutral classes and hard limits, close public
admission, drain the old creator, verify no creation is in flight, and undrain
the new creator. This is intentionally less automatic than a scheduler and has
an auditable rollback boundary.

After source-host reservations are proved, another qualified host can add
capacity beside lat3 without product-level provider branching or untargeted
first-claim races. The initial one-creator rule does not cap the eventual fleet.

`finite-lat-3` moves new-Agent Runtime compute and mutable storage off lat1
without moving existing user state. New Agents still load Core, Postgres, Chat,
Hosted Devices, Sites, backup coordination, and control paths on lat1. The
measured control-plane budget and fail-closed gates reduce but cannot eliminate
lat1 crash/outage risk. Lat3 becomes trusted customer capacity only after
storage, chat, backup, restore, private-routing, and admission gates pass. RAID
improves disk availability but does not make lat3 a Recovery Authority.

Future iOS, Electron, Brain, Sites, and confidential-compute work remains
independent because Agent identity, product protocols, and hosting tier do not
depend on a specific Latitude hostname.

## Rejected alternatives

- Reinstalling or adding Agent load to `finite-lat-2` now.
- Leaving lat1 and lat3 simultaneously undrained and relying on lease timing.
- Automatic lat3-to-lat1 fallback or existing-Agent migration.
- Exposing Core Runner APIs or Kata Runtime ports to the public Internet.
- Blocking WorkOS signup rather than the capacity-consuming operation.
- Counting swap as available Agent memory.
- Introducing Kubernetes solely to add the third host.
