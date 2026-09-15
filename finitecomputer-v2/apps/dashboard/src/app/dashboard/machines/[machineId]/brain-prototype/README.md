# FIN-89 Brain overview prototype

Selected design: categorized membership cards (A), approved by Austin on September 15.

Run from the repository root in the pinned development environment:

```sh
just dashboard brain-prototype
```

Open http://127.0.0.1:13089/dashboard/machines/runtime_web_design/brain-prototype.
The existing design fixture supplies the dashboard shell and its example agent, Moss.
The Brain page uses in-memory synthetic memberships and makes no Brain requests.

The page always represents the selected agent's Brain memberships and Folder metadata. The human user's memberships do not determine this list: the agent is the user's main interface to Brain.

The selected A cards show each Brain's folder count and up to three folder names. Click or keyboard-activate `+N more` to expand all names in the card; `Show less` collapses them. “Folders shown” refers to the metadata visible to the selected agent; it excludes linked folders and does not measure local sync. These are example names and counts, not live inventory.

Use the preview control to inspect empty, unavailable, stale, access-lost, and unavailable-folder-detail states. Missing folder details preserve the Brain's name and role without displaying a false zero count. Refresh shows loading and updates the successful-fetch time; the failure scenarios remain failures until another scenario is selected. Expand “Prototype state” for the full example state.

Only the selected card layout remains on this branch. The original three-layout study is preserved in commit `89c9806c`; old `?variant=` links now display the selected cards.

The route requires development mode, the existing local-account fixture switch, and the fixture machine ID. Production requests return not found, and production navigation stays disabled. No new authentication bypass or Brain integration is introduced.

This is a throwaway review artifact on `prototype/brain-overview`, tracked in [FIN-89](https://linear.app/finitecomputer/issue/FIN-89/add-a-basic-brain-overview-with-categorized-membership-cards-and). Austin selected A (categorized cards) and confirmed selected-agent scope on September 15. The authorized agent inventory path remains implementation work; sample memberships do not validate that contract. The dashboard must verify the user's access to the selected agent, then obtain that agent's Brain inventory. File counts and storage size are deferred.

## Integration investigation — September 15

The card data already exists. The connection from the dashboard to the selected
agent's Brain identity is the remaining dependency. Source checked at finite-mono
`f2a10ffe` and the repository's pinned Hermes revision
`29112bef099274229cadff79cdff7bf7b99c4b77`.

| Read | Current source | Meaning for this page |
| --- | --- | --- |
| Brain list | Brain server `routes/brains.rs::list_brains_handler` | Returns the request signer's Brains, including pending invitations. |
| Folder metadata | Brain server `responses.rs::metadata_response_for_actor` | Guest results are filtered; metadata visibility does not prove content decryption. |
| Dashboard Brain signing | Dashboard `lib/brain-hosted-client.ts` | Uses the human identity, so it cannot supply this agent inventory. |
| Agent Brain signing | Brain CLI `lib.rs::brain`, `http.rs::signed_json_request` | Existing `fbrain brain list --json` and `fbrain brain metadata <id> --json` use the local signer. |
| Native extension | [Pinned Hermes `hermes_cli/web_server.py`](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/hermes_cli/web_server.py#L19071) | Enabled user/bundled plugins can mount authenticated `/api/plugins/<name>/` routes. No Brain overview handler ships here. |

### Smallest candidate to prove

Reuse the authenticated agent connection from
[FIN-39](https://linear.app/finitecomputer/issue/FIN-39/provide-hermes-web-authentication-and-desktop-connection-details)
and a Brain-owned Hermes plugin with one fixed read-only overview operation.
The local handler would run the existing Brain CLI reads and return only Brain
ID, name, kind, role, and folder IDs/names. Keep invitations separate and discard
invite capabilities, member lists, grants, keys, and unrelated metadata.

The dashboard must authorize access to the current agent on every request. The
handler must use that agent's existing signer; the browser receives only the card
result. Bound process time, output size, and metadata fanout. A metadata service
failure means unknown folder details. Authorization loss clears affected data;
it must not be treated as an ordinary stale refresh.

First prove this with two synthetic agents whose Brain memberships differ from
each other and from their human controller. Include one guest with limited folder
metadata, a pending invitation, missing signer, and revoked access. Verify that
unauthenticated requests never invoke the CLI, agent switching discards late
responses, and disabled native access remains unavailable without being enabled
by opening this page.

This is a candidate, not an implemented or qualified connection. FIN-39 currently
holds connection implementation for the Iroh comparison; its first release is
default-off and admin-enabled. Reuse its settled connection and authorization
contract before enabling production Brain navigation. Native plugin support alone
does not prove dashboard viewer authorization or cross-agent isolation. Keep Brain
inventory out of Core status storage, Runtime Management Pipe, and chat commands.
