# PR CodeRabbit Fallback Round 1

## Scope

- Repo: `finitecomputer/finite-brain`
- PR: `https://github.com/finitecomputer/finite-brain/pull/15`
- Base: `staging`
- Head: `feature/rust-portable-v1-core`
- Trigger: `@coderabbit full review`
- Trigger comment: `https://github.com/finitecomputer/finite-brain/pull/15#issuecomment-4784920083`

## Availability

CodeRabbit did not post findings or checks after the PR trigger and polling.
Per the Feature Dev fallback path, a fresh Codex review agent was used instead.

- Reviewer: `Meitner`
- Review session: `019ef729-99cf-7312-8d11-e21f4d600f9e`
- Fixed point: `49b012e`
- Fix commit: `ce212e7`

## Findings Addressed

| Finding | Severity | Resolution |
| --- | --- | --- |
| Folder Key Grants were accepted without validating their NIP-59 gift wrap envelope. | high | Added server-side gift-wrap parsing and validation before accepting grant metadata. |
| Signed payload timestamp validation expected Unix-second strings instead of RFC3339 timestamps. | high | Switched server validation helpers and tests to RFC3339 timestamp values. |
| Sync pull leaked admin control records to non-admin members. | high | Made sync-record visibility type-aware so admin access changes are admin-only and grants are visible only to admins or the recipient. |
| Sync bootstrap omitted control records needed to recover Folder Key Grant state. | medium | Added bootstrap `controlRecords` and filtered them through the same visibility boundary as incremental sync. |
| Object sync append log stored only ciphertext or delete markers, dropping signed revision/tombstone event context. | medium | Preserved accepted signed revision and tombstone request payloads in the append log while keeping current-object APIs ciphertext-shaped. |
| Diff hygiene failed because several docs had blank EOF whitespace. | low | Removed the stray blank EOF lines. |

## Verification

- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build`
- `git diff --check`

## Outcome

All fallback review findings were addressed. No unresolved high-risk findings
remain from this PR review round.
