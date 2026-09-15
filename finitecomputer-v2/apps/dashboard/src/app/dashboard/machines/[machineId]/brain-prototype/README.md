# Brain overview prototype

This development-only page previews the approved categorized cards (A) for the
selected agent. It uses sample data for Moss and makes no Brain service requests.
[FIN-89](https://linear.app/finitecomputer/issue/FIN-89/add-a-basic-brain-overview-with-categorized-membership-cards-and)
owns the implementation plan and completion checks.

## Review locally

From the repository root in the pinned development environment:

```sh
just dashboard brain-prototype
```

Open http://127.0.0.1:13089/dashboard/machines/runtime_web_design/brain-prototype.

- Check Personal and Organization cards, access labels, and folder previews.
  Activate `+N more` with a mouse or keyboard; use `Show less` to collapse it.
- Use the preview control for empty, unavailable, stale, access-lost, and missing
  folder details. Missing details show an unavailable label rather than zero.
- Refresh updates the sample timestamp. Failure scenarios remain failures until
  another scenario is selected. Expand “Prototype state” to inspect the fixtures.

The route requires development mode, the existing local-account fixture switch,
and the fixture agent ID. Production requests return not found; production Brain
navigation remains disabled.

## Source evidence for implementation

Checked against finite-mono `f2a10ffe` and its pinned Hermes revision
`29112bef099274229cadff79cdff7bf7b99c4b77`:

| Source | Finding |
| --- | --- |
| `finite-brain/crates/finite-brain-server/src/routes/brains.rs::list_brains_handler` | `GET /v1/brains` lists the request signer's Brains and pending invitations. |
| `finite-brain/crates/finite-brain-server/src/responses.rs::metadata_response_for_actor` | Per-Brain metadata includes folder names. Guest results are filtered. Metadata visibility does not prove content decryption or local sync. |
| Dashboard `src/lib/brain-hosted-client.ts` | The hosted signer uses the human identity and cannot supply the selected agent's inventory. |
| `finite-brain/crates/finite-brain-cli/src/lib.rs::brain` and `src/http.rs::signed_json_request` | Existing `fbrain brain list --json` and `fbrain brain metadata <id> --json` use the local signer. |
| [Pinned Hermes `hermes_cli/web_server.py`](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/hermes_cli/web_server.py#L19071) | Enabled user/bundled plugins can mount authenticated `/api/plugins/<name>/` routes. This proves extension support, not a working Brain endpoint. |

Neither Brain read supplies file counts or a storage-size summary. The overview
uses only Brain and folder metadata; it does not need an export or file scan.

## Brain-document context

FiKnight reviewed the Organization Brain on September 15 at sequence 1806, with
no conflicts. Relevant sources:

- `Finite Mono LLM Wiki/topics/finite-mono/wiki/topics/brain-surface-and-viewers.md`
  and sibling `brain-single-file-viewer-plan.md`: August 14 viewer proposals.
- `audits/output/essentials-audit-2026-08-29/01-auth-kernel-rebase.md`: authentication
  audit context.

The viewer proposal preferred reads independent of agent compute. Austin's
September 15 decision accepts an online-agent requirement for this overview and
reuses FIN-39's connection. That decision is recorded in FIN-89. The Brain
endpoint and dashboard authorization checks remain implementation work.
