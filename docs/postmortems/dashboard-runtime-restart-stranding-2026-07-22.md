# Dashboard Runtime Restart Can Strand a Kata Agent

Status: confirmed production defect; fix not included in the 2026-07-22
Electron/OpenRouter release train.

## Summary

The owner-facing dashboard Restart action can leave an otherwise recoverable
Kata Agent stopped. Core correctly enqueues a bounded Runtime Restart request,
but the Kata adapter implements it as one `nerdctl restart --time ...` command.
If the guest exits and Kata/containerd then returns a transport error, the
Runner records the request as failed without reconciling the provider state or
bringing the same canonical container back online.

Waffle Prime demonstrated this failure on 2026-07-22. This incident is
independent of its OpenRouter configuration problem: the restart was attempted
as a repair after inference failed, but restarting compute cannot change how
Hermes resolves an OpenRouter credential.

## Production evidence

- Project: `project_eb9cf0d2ff2a3cacea18` (`Waffle Prime`).
- Runtime: `runtime_60a635e4c80b9cc9fd1b`.
- Provider handle: `finite-kata-7aa9e1ff84d7532e5a52` on `finite-lat-1`.
- Restart request: `runtime_ctl_cf914d65d0929a6d00d7`.
- The request failed at `2026-07-22T14:15:14Z` after `nerdctl` returned
  `ttrpc: closed`.
- The canonical container subsequently reported `Exited (255)`, its exact
  containerd task remained `STOPPED`, and its health endpoint timed out.
- Core retained the active Project/Runtime binding and marked the Runtime
  facts stale. The 38 MiB durable `/data` bind remained present.

The same class of delayed Kata task cleanup has appeared during Runtime
Upgrade and Retirement operations. Those workflows have their own retry and
reconciliation boundaries; ordinary Restart currently does not.

## Current control flow

The dashboard action checks the Project's advertised `restart` capability and
calls Core's owner-scoped Runtime Restart endpoint. The Runner validates the
leased Project, Runtime, source host, source machine, ownership labels, and
canonical container, then calls `nerdctl restart`. Any nonzero result returns
immediately from `KataLauncher::restart_runtime`. Core records a terminal
failure and stale status.

This means a command result is being treated as the provider outcome. With
Kata, those are not equivalent: the guest can have stopped successfully while
the later task teardown acknowledgement fails.

## User impact and recurrence

This is not Waffle-specific. Any owner who presses Restart while Kata produces
the same stop/teardown error can be left with a stopped Agent and a failed
request. Retrying from the dashboard is not a safe repair contract because the
canonical container may already be stopped and its task may still be
converging or stuck.

The dashboard currently reports the failed mutation, but it cannot distinguish
"the Agent remained online" from "the restart command failed after stopping
the Agent." Operators must inspect the exact provider state before repair.

## Approved bounded repair for Waffle Prime

Waffle's repair is an explicitly approved one-off operator action, not the
general product fix:

1. Revalidate the exact Project, Runtime, source host, source machine,
   canonical container ID, labels, and sole writable `/data` bind.
2. Require the exact retained task to be `STOPPED` and ensure no duplicate
   container owns the source machine or durable root.
3. Remove only that stale task record and start the same canonical container.
4. Require health and the unchanged Agent Principal before any Runtime Upgrade.
5. Stop on any disagreement or second provider failure.

No SQL rewrite, owner impersonation, replacement Runtime, data migration, or
broad container cleanup is authorized by this procedure.

## Minimum product fix

Keep the existing Core request and dashboard action. Change only the Kata
Restart implementation so it reconciles the exact canonical container after
the provider command returns:

- Capture the validated canonical container ID and Agent Principal before the
  restart.
- After either success or failure, inspect that same container rather than
  inferring state from the command exit code.
- If it is running, require health and the same Principal before completing.
- If it is stopped, distinguish an exact retained `STOPPED` task from an
  ambiguous or unreachable task state. Only the exact, unambiguous stopped
  case may receive bounded cleanup and start of the same container.
- If identity, ownership, task state, or durable-root ownership is ambiguous,
  fail closed and leave the request terminal with an operator-facing offline
  warning. Never launch replacement compute as part of Restart.

This requires no new lifecycle, migration, roster, or dashboard workflow.

## Acceptance criteria

- A normal dashboard Restart returns the same Runtime to healthy with the same
  Agent Principal and durable root.
- A simulated `ttrpc: closed` after the guest exits converges the exact stopped
  task and restarts the same container.
- A nonzero provider result after the container already returned healthy is
  reconciled as success only after health and Principal verification.
- Changed container ID, duplicate `/data` ownership, a running or unreachable
  task, or a missing Principal fails closed without starting anything.
- Core and the dashboard distinguish a terminal offline failure from a healthy
  restart; a second button press is not presented as recovery.
- Unit tests cover the provider result/state matrix, and one synthetic Kata
  test injects the post-stop transport failure against real task teardown.
- Existing browser users and Runtimes that do not advertise Restart retain
  their current behavior.

## Release relationship

PR #186 fixes the separate OpenRouter defect by storing
`OPENROUTER_API_KEY` in durable Hermes `.env` state and removing the ignored
inline model key. It does not change Runtime Restart. The release may proceed
with Waffle as the sole explicitly repaired and upgraded target; broader
Runtime rollout remains gated on the owner's successful Waffle acceptance.
