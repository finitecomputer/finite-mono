# Retiring a big home

A retirement uploads an agent's whole home to rsync.net with borg and reads
it back, in one run of `finite-saas-runner.service` on the agent's host. A
home of up to about 120 GB takes hours: about 4 at a clean day's ~20 MB/s.
A run may last 12 hours (`RuntimeMaxSec` in
`infra/nixos/modules/hosted-hermes.nix`).

## Before

- Retire big homes one at a time per host, at a quiet hour. While one runs,
  that host's other Runner work (creations, restarts, upgrades, stops)
  waits for it, and its other agents' health reports read `stale`.
- A congested route to rsync.net (around 100 KB/s) can't carry a big home
  in any number of hours. Wait for a clean route.

## What happens

1. The mover's retire runs Core's `runtime-retire-exact` and waits 15
   minutes (`core.waitSeconds`, 900). A big home takes longer. Core says
   the request "timed out; the same request remains retryable", and the
   mover reports `failed` at step `retire`.
2. finite.chat keeps the retire command, so the move is not held. Each
   later mailbox run asks again: Core hands back the same request, still in
   flight, and waits another 15 minutes. A day after the retire began, the
   move is held (`RetireTimedOut`, or `ArchiveFailed` for a phase 2
   archive).
3. The runner keeps going. A run cut short (a deploy, the 12-hour cap, a
   stop) leaves its lease to lapse within the hour. Then a later run takes
   the same request, and borg resumes the upload from its last checkpoint
   (at most 5 minutes are lost). The readback starts over.
4. When the runner is done, Core's request ends `stopped` and Core holds
   the retirement's receipt.

The mover's wait stays at 15 minutes: a longer one would block a whole
mailbox run, and everyone else's moves behind it, for hours.

## Finish the move

Check that the runner is done, in Core's database on lat2
(`runuser -u postgres -- psql -d finite_core`):

```sql
SELECT request.status, snapshot.request_id AS receipt
FROM runtime_control_requests AS request
LEFT JOIN runtime_retirement_snapshots AS snapshot ON snapshot.request_id = request.id
WHERE request.project_id = '<old>' AND request.kind = 'destroy'
ORDER BY request.created_at DESC LIMIT 1;
```

It is done at `stopped` with a receipt. Then:

- If the move is still retiring, with its command, the next mailbox run
  finishes it.
- If the move is held, run `fragment operator retry <old>`. That gives it
  its retire command again, and the next mailbox run finishes it. While
  the mover's timers are off, run that mailbox by hand.

Either way the mover finds Core's receipt (`retired_receipt`, which it
reads from Core's database) and doesn't retire again. It marks the runtime
moved (`runtime-mark-moved-exact`) and reports `retired`, and the move is
done.

Don't retry before the runner is done. Core hands back the request still
in flight, that mailbox run waits 15 minutes for nothing, and the move's
day starts over.
