# Monorepo Migration Log

This log records the source snapshots and migration decisions used to construct
`finite-mono`.

## Phase 0: Preparation

Date: 2026-07-06

Destination repository:

- `/Users/alex/Projects/finite/finite-mono`

Import method:

- Direct file copy from source repos.
- No git history preservation.
- No `git subtree`, `git filter-repo`, or equivalent history-preserving import
  mechanism.
- Existing source repos remain untouched.
- No rollback plans are being maintained for the copy operation.

Source snapshots:

| Repo | Source path | Commit SHA | Working tree at record time |
| --- | --- | --- | --- |
| `finitecomputer-v2` | `/Users/alex/Projects/finite/finitecomputer-v2` | `862e6bf11ec2c8e8c0e6b3d85471a39257bb7e21` | Clean |
| `finitechat` | `/Users/alex/Projects/finite/finitechat` | `f13c973d493831065994767b0f93783a49873071` | Clean |
| `finite-sites` | `/Users/alex/Projects/finite/finite-sites` | `768a0b84898e7acfd865dd1e13645ab6ce19ea09` | Clean |

Notes:

- These SHAs identify the source commits used for the first planned copy.
- The copied tree will be a snapshot, not a history-preserving import.

