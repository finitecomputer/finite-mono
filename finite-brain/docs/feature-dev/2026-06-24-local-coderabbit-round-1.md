# CodeRabbit Round: Local Branch Review 1

## Round

- Scope: local
- Round number: 1
- Command or trigger: `coderabbit review --agent --type all --base staging`
- Started: 2026-06-24T00:56:50Z
- Completed: 2026-06-24T00:56:50Z
- Availability: completed
- Fallback review thread: not needed

## Findings To Address

| Finding | Severity | Decision | Notes |
| --- | --- | --- | --- |
| Core identifiers and paths had no practical length ceilings. | medium | addressed | Added bounded `UserId`, `DisplayName`, and `SafeRelativePath` validation plus regression coverage. |
| Store accepted link timestamps as arbitrary printable strings. | medium | addressed | Added RFC3339 parsing via `time` and a negative test for invalid expiration timestamps. |
| Store used hard-coded runtime timestamps for sync/vault/folder creation. | medium | addressed | Added runtime UTC timestamps while keeping migration timestamps deterministic. |
| Folder ID JSON serialization used `expect`. | low | addressed | Returned `StoreError::Database` instead of panicking. |
| Bootstrap accepted unbounded folder/grant batches. | medium | addressed | Added explicit batch caps and an oversized-bootstrap regression test. |
| OKF export duplicate path guard checked the output file map too late. | medium | addressed | Added a dedicated bundle-path set and duplicate export regression test. |
| Sync pull limit was client-controlled at the route layer. | medium | addressed | Clamped route limits before calling the store. |
| Server auth clock was captured at startup. | high | addressed | Made the server clock dynamic by default with deterministic test override support. |
| Folder key grant fallback timestamps were fixed constants. | medium | addressed | Route-generated fallback grant timestamps now use the server timestamp. |
| Object writes checked folder access/key version before final store lock only. | high | addressed | Rechecked visibility and key version inside the locked submit path. |
| Database errors exposed internal details. | medium | addressed | Server now maps database failures to a generic `500` body. |

## Findings Not Addressed

| Finding | Reason |
| --- | --- |
| None | All local CodeRabbit findings were addressed. |

## Result

- Continue: yes
- Escalate: no
- Notes: Affected checks passed after fixes: `cargo fmt --check`; targeted core/store/server tests; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/smoke/bootstrap`, `/smoke/ui`, and `/smoke/ui.js` curl smoke.
