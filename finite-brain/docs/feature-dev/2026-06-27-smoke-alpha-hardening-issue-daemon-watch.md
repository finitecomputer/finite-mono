# Add fbrain daemon watch loop for Brain Working Trees

## Parent

Parent PRD: #47

## What To Build

Add a resident foreground watch mode to `fbrain` that repeatedly runs the real
Brain Working Tree sync path. The watcher should be suitable for smoke-box
Agent Runtime use under a supervisor while remaining bounded and deterministic
in tests.

## Acceptance Criteria

- [x] `fbrain daemon watch` runs the same sync path as `daemon tick` and
  `sync now`.
- [x] The watcher records daemon start, tick, blocked, and stopped state in the
  existing local Agent state/activity model.
- [x] The command supports bounded options for tests and smoke, such as `--once`,
  `--max-ticks`, and `--poll-secs`.
- [x] Sync failures do not erase prior local state and are surfaced through
  `daemon status` or `daemon logs`.
- [x] Focused CLI tests cover bounded watch success and blocked watch behavior
  through the public CLI runner.

## Blocked By

None - can start immediately.
