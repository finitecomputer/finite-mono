# Add smoke alpha backup, restore, and SilverBullet cutover handoff

## Parent

Parent PRD: #47

## What To Build

Add an operator runbook and local verifier for the first internal smoke alpha.
The runbook should explain how to back up and restore the Rust FiniteBrain
SQLite database, how to verify restored state, and how a Deployment loop should
replace/archive the old SilverBullet route with the Rust Product Client route.

## Acceptance Criteria

- [x] The runbook documents the Rust app service target, environment variables,
  port, SQLite database path, and health/client routes.
- [x] The runbook documents database-consistent backup and restore commands for
  SQLite, including integrity checks.
- [x] The runbook documents pre-cutover, cutover, rollback, and post-cutover
  verification for replacing the old SilverBullet route.
- [x] The runbook distinguishes Feature Dev outputs from Deployment-loop live
  operations.
- [x] A local verifier or scripted checklist proves the backup/restore path on a
  temporary SQLite database where practical.

## Blocked By

None - can start immediately.
