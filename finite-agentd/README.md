# finite-agentd

`finite-agentd` is the narrow, runtime-resident Finite platform daemon owned by
one Agent Principal. It survives Hermes restarts, receives typed encrypted
runtime commands through Finite Chat, publishes command results and observed
state, and applies allowlisted agent-local changes with durable rollback.

In the production Kata layout, each Agent Runtime has its own `/data`. The
Agent's Finite Chat Device store and `finite-agentd`'s durable
`/data/agent/agentd/agentd.sqlite3` authorization/command ledger therefore do
not share storage with another Agent Runtime. `finite-agentd` independently
supervises the resident Finite Chat sidecar, health service, and Hermes with
null stdin. Hermes must be healthy to produce a new model reply; retained Chat
state and typed management commands such as `agent.owner.claim` are not Hermes
interactivity contracts.

This is distinct from the web user's Hosted Device on lat1. One
`finitechat-hosted-device` service hosts many verified WorkOS users in a
runtime map, with a separate identity and encrypted SQLite store for each
user. It is not one `finite-agentd` or one process per web user.

It is not Core, Runner, RMP, `fsite`, `fbrain`, or the Finite Chat server. It
never accepts arbitrary shell, argv, filesystem paths, YAML, or environment
edits from the platform.

The architectural decision and first-slice acceptance criteria are in
[`docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md`](../docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md).

The current daemon accepts these versioned command families over the Agent
Platform Channel:

- `agent.status.inspect`
- `agent.owner.claim`
- `agent.hermes.restart`
- `agent.chat.recover`
- `agent.connections.status`
- `agent.inference.apply`
- `agent.specialization.multimodal.reconcile`
- `agent.telegram.connect`, `agent.telegram.approve`, `agent.telegram.home`,
  and `agent.telegram.disconnect`
- `agent.google.apply` and `agent.google.disconnect`
- `agent.hermes.config.preview`
- `agent.hermes.config.apply`
- `agent.hermes.config.rollback`

Specialization reconciliation owns only the `auxiliary.vision` Hermes config
field. Its typed multimodal desired state includes the worker endpoint, canonical
model alias, independently declared image/audio/video capabilities, prompt
versions, and normalization limits. Existing worker credentials are retained
unless a replacement credential is supplied through the encrypted command.
Finite-applied values carry a durable pre-image and ownership hash; validation
failure restores the exact previous bytes, and later user/Hermes drift blocks
automatic rollback. Remote commands fail closed unless the sending Finite Chat
Principal is in the durable authorization ledger.

Specialization reconciliation is deliberately a model-profile operation. It
does not register model-named tools, intercept attachments, or add behavioral
instructions to the main agent. Hermes keeps its normal tool catalog and the
main model decides when to use a native capability. The current multimodal profile
backs Hermes's `vision_analyze` and `video_analyze` tools through
`auxiliary.vision`. The capability flags constrain requests accepted by the
worker; they do not create a missing Hermes tool surface. Semantic audio
interpretation therefore remains unavailable to the agent until Hermes has a
generic instruction-preserving audio-analysis capability. This profile-first
rule applies to every Finite specialization, not only multimodal or vision.

At runtime creation, the trusted Runner can declare
`FINITE_SPECIALIZATION_BUNDLE=finite-private-multimodal-v1` and provide the separate
`FINITE_SPECIALIZATION_WORKER_API_KEY`. After Hermes prepares `config.yaml` and
after the core supervisors and health surface start, `finite-agentd` applies
that platform-managed bundle in the background and restarts Hermes when the
configuration changes. This keeps chat startup independent from a contended
configuration lock or a failed validator. The profile is not reported effective
until reconciliation, restart, and semantic verification finish. The bundle applies even
when `auxiliary.vision` contains a prior custom profile. It owns only that
vision value and the required `video` membership in
`platform_toolsets.finitechat`: existing toolset entries are retained, and
removal restores the prior vision value and removes only the membership Finite
added. A durable transition journal preserves the original semantic field
pre-images across credential rotation and completes interrupted activation or
removal after restart. Automatic activation writes native Hermes provider and
toolset configuration only; it does not add capability or prompt-policy
metadata.
While running, finite-agentd is the single authoritative Hermes configuration
writer. All supported mutation paths share a bounded process mutex and OS
advisory lock across read, journal, write, validation, and commit. Each write
also checks its observed preimage and checks the committed bytes afterward.
Manual concurrent writes by a process that ignores the lock are unsupported:
the filesystem does not provide a portable universal compare-and-swap contract.
Observable mismatches fail closed, and later transactions detect remaining
field drift instead of claiming ownership of it.
Pre-journal releases did not persist an unforgeable startup-operation kind, so
their generic config history is never adopted as ownership. On upgrade, the
exact live Hermes config becomes the new journal baseline. This conservative
mixed-version rule may retain an older canonical vision/video value when the
new profile is later removed, but it never guesses authorship or deletes an
ambiguous user-owned value.
Runtime status reports the bundle identifier plus `desired`, `effective`, and
`cleanup_blocked` booleans without serializing the credential. Owned-field
drift blocks automatic cleanup and sets `cleanup_blocked` after a switch away,
but does not prevent chat and Hermes from starting. `effective` becomes true only
after the running Hermes catalog admits `video_analyze` and the installed
Hermes-native vision tool passes the fixed semantic probe for the current
Hermes process generation. Matching configuration bytes alone are not
sufficient, and a restart triggers a new probe.
Transient semantic failures restore the owned pre-images before rearming the
still-desired profile. Initial reconciliation failures and restart failures use
the same bounded policy; a failed restart first restores the prior managed
generation and attempts to restart Hermes on those bytes. Rearms use bounded
backoff and stop after three attempts per daemon process, leaving the runtime
visibly ineffective rather than restarting Hermes indefinitely; an agentd
restart or startup-configuration change opens a fresh verification cycle.

The Runner makes this admission solely from the canonical Finite Private
profile, currently `deepseek-v4-flash-0731`. It has no box, user, project, or
agent-name condition: every new runtime launched with that profile receives the
bundle when its Runner is Specialization-Ready. A missing or invalid credential
prevents that Runner from leasing new canonical Finite Private creation work;
it does not silently launch a runtime without the bundle. Existing-runtime
lifecycle Stop and Destroy remain available during that configuration failure;
canonical Restart, Recover, and Upgrade remain fenced.

A multimodal image reconciliation becomes effective only after Hermes restarts and
its installed `vision_analyze_tool` returns exact semantic output for a fixed
image through `auxiliary.vision`. The packaged probe uses the same
`HERMES_HOME` as the resident process and emits only a bounded pass/fail result;
it does not expose the worker credential or provider response.
`FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS` seeds that ledger when configured. For
the trusted internal-canary path only, the first `agent.owner.claim` may fill
an empty ledger; later claims and every other unauthorized command fail
closed. This is not the broader customer-admission authority that ADR 0003
still requires.

Durable ledger reopening, pending-command resume, and terminal-result replay
are covered locally. The remaining production evidence gaps are a live
lat1-plus-Kata composition gate, real child-death/signal/orphan coverage for
the supervisor, and off-host restore of the same Agent Device, ledger, and
retained data onto an empty target. Local Hermes CI runs the encrypted bridge
flow, but its wrapper can still synthesize the passing report artifact when the
richer in-test report hook is absent; that report is not independent
live-runtime evidence.
