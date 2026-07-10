# Finite Skills Runtime Delivery Contract

Status: first-slice bundling and explicit image-baseline sync implemented.

## Goal

New agents must know the Finite platform on their first turn, while deployed
agents must be able to pick up improved guidance without a Runtime roll or
reboot. This must stay a local skills workflow, not grow a fleet control plane.

## One Source, One Bundled Baseline

`finite-skills/skills` in this monorepo is the only editable source for the
Finite-managed baseline. Runtime images copy a tested bundle from that source
and configure Hermes to discover it before the Runtime becomes ready. A fresh
agent therefore works from its bundled baseline even when no distribution
service is reachable.

Component repos, dashboard code, Runtime templates, and distribution mirrors
must not carry editable copies. User-authored skills remain in the user's
writable skill area and are not part of the managed baseline.

## Existing Agents Update Explicitly

Existing agents update at their own pace through a one-shot command:

```text
finite skills sync
```

The user or agent invokes the command explicitly. The first slice reads only
the compatibility-tested `/runtime/finite-skills` tree bundled into the
currently running Runtime image. It performs no network request. It rejects
symlinks, non-file entries, malformed or duplicate skill metadata, and bundles
missing the canonical FiniteBrain or Finite Sites skills. It then stages,
fsyncs, and atomically exchanges the durable managed baseline. Ordinary
failures after the exchange restore the prior baseline. It does not require a
Runner operation or Runtime reboot.

Hermes tools that scan the configured external directory read the replacement
tree in place. Hermes 0.18.2 caches its slash-command name map, so newly added
or removed skill names become visible after the user invokes
`/reload-skills`; changed content at an existing path is already available.
An in-flight model turn is not retroactively changed.

## Coupling Wall

Finite Skills has no Core desired revision, automatic rollout, background
polling, Runtime Management Pipe capability, Runner transport, provider mount,
dashboard file editor, or forced reboot. Core may observe generic release
component versions through normal release telemetry, but it does not decide
when an existing agent syncs.

The image's bundled baseline remains compatibility-tested with its Finite
Product Release. A sync must reject an incompatible bundle without altering the
working baseline. User skills and intentional user overrides are never edited,
deleted, or rolled back by a baseline sync.

## First-Slice Proof

- A new offline Runtime sees the bundled Finite platform skills before its
  first Hermes turn.
- The exact bundled source and Hermes version are exercised together in the
  Runtime image canary.
- No skills update network request occurs; `finite skills sync` reads the
  current image bundle only.
- A successful sync changes future skill discovery without replacing the
  Runtime or rebooting Hermes; a failed sync leaves the prior baseline usable.
- User-authored skills survive sync, Runtime restart, and Runner replacement.
- Docker, Kata, and Phala expose the same baseline and sync command;
  adapters contain no skills logic.

## Current Gaps And Open Questions

The bundled baseline carries the canonical `fsite` 0.4.0 Finite Sites guidance.
Agents seeded from an older baseline can adopt it explicitly with
`finite skills sync` after the tested image bundle is present. Runtime restart
or image replacement still must not silently overwrite the durable baseline.

Component trees still carry historical/reference `SKILL.md` snapshots. They
are not deployment sources. The current Finite Sites and FiniteBrain deltas
have been reconciled here, and promotion checks must prevent a component copy
or retired command surface from becoming the managed baseline again.

Remote publication and cryptographic artifact verification are intentionally
out of the first slice: the promoted Runtime image is the trust and
compatibility boundary. If a future distribution source is added, it must keep
the same explicit local activation and coupling wall. Do not add polling,
automatic rollout, Core desired state, or Runner logic.

The directory exchange is atomic on the Linux Runtime filesystem. If the
process is killed before the exchange, the prior baseline remains active. If
it is killed after the exchange, the complete new baseline is active and a
hidden staging directory containing some or all of the prior baseline may
remain for later cleanup; Hermes never observes a mixed tree. This first slice
does not maintain a user-facing revision history or rollback command.

Recovery Snapshot coverage for user-authored skills belongs to the broader
recovery TODO. It is not part of the sync protocol and is not a first-slice
launch gate; restart on preserved state is the immediate requirement.
