# Agent Runtime Launch-Time Audit

Status: PROPOSED

Date: 2026-07-13

## What launch does

`finitechat/containers/agent/run_hermes_gateway.sh` performs local filesystem
work at first boot: it copies bundled skills, initializes Finite Chat state,
installs the local adapter, and reconciles configuration. It does not run a
package manager, compiler, or remote installer. Optional restore happens in
the container entrypoint before that script.

The externally visible launch path still combines distinct phases:

1. fetch/unpack the OCI image when it is not cached;
2. boot the Kata guest and start the runtime container;
3. seed and initialize Hermes/Finite Chat locally;
4. poll Core and publish runtime/profile readiness.

Current production journal messages do not timestamp those four boundaries,
so existing logs cannot assign a slow launch to one phase reliably.

## Measurements

- A successful cached production launch observed on 2026-07-10 took about
  3.45 seconds from the systemd start record to agentd's `launched` record.
- One older production attempt remained in the launch path for about 238
  seconds before stopping, but the journal has no phase marker that can say
  whether pull, Kata, Hermes, or Core polling owned that time.
- Three local cached container starts against an intentionally unreachable
  Core endpoint failed at profile publication in 1.05, 1.14, and 0.78 seconds
  (median 1.05 seconds). This is a lower-bound startup probe, not a successful
  end-to-end launch.
- The same-source local arm64 image before `gws` was 211,616,285 bytes; after
  `gws` it was 218,447,047 bytes. The compressed image grew 6,830,762 bytes,
  or 3.23%.
- The matched after-image lower-bound probes were 0.74, 0.74, and 0.74 seconds
  (median 0.74 seconds). Against the 1.05-second before median, this provides
  no evidence that `gws` materially slows a cached start. These noisy probes
  intentionally fail at an unreachable Core endpoint and cannot measure image
  pull or successful readiness.

## Recommendation

Add structured elapsed-time records at the existing orchestration boundaries
before optimizing launch code: image availability, guest/container running,
Hermes initialized, and Core/profile ready. This can be observability-only,
but it is outside this overnight queue. The evidence currently supports
preinstalling repeatedly downloaded tools for task latency; it does not show
that package installation is part of Agent launch latency.
