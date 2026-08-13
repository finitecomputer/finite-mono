# ADR-0046 Hands-On Test: Browser Invite + CLI Accept, Both Directions

Status: active runbook for the ADR-0046 principal-grants slice.

Two roles, one Mac:

- **Alice** drives a browser (the Brain Product Client in the devfinity
  dashboard).
- **Bob** drives a terminal (the `fbrain` CLI with his own identity).

The automated proof of this exact flow is
`scripts/devfinity-adr46-slice` (run via
`scripts/with-dev-env scripts/devfinity-adr46-slice-up`); this runbook is the
human-driven version of the same story.

## 0. Boot the stack

```sh
ADR46_HANDS_ON=1 scripts/with-dev-env scripts/devfinity-adr46-slice-up
```

This boots the disposable devfinity stack (Alice the fixture customer
`devfinity@finite.computer`, Bob the second fixture user `bob@finite.vip`),
launches Alice's real managed agent runtime, bootstraps Alice's Personal Brain
with a shared Folder and a seeded note, and provisions Bob's account, hosted
identity, and agent roster. It then stops **before** the automated acts and
waits.

Cold runs build the Rust workspace and the runtime image; expect tens of
minutes the first time. State and logs land in `.local-state/adr46-slice-<ts>/`.

When the stack is ready, the boot log prints a summary block with
`dashboard:  http://127.0.0.1:<port>/dashboard` (plus brain, core, and other
service URLs), and the hands-on gate prints:

- the dashboard URL (Alice's browser entry point),
- Alice's Personal Brain id and her agent's container id,
- Bob's CLI environment block (copy it into Bob's terminal, see step 1).

Leave this terminal running. Press Ctrl-C here at the end to shut the stack
down.

## 1. Bob: set up the CLI (once)

In a second terminal, paste the environment block the hands-on gate printed:

```sh
export FINITE_HOME="<state-dir>/bob-agent"
export FBRAIN_WORKING_TREE_ROOT="<state-dir>/bob-agent-tree"
export FINITE_BRAIN_SERVER_URL="http://127.0.0.1:<brain-port>"
export FINITE_BRAIN_PUBLIC_BASE_URL="http://127.0.0.1:<dashboard-port>"
export FINITE_IDENTITY_AUTHORITY="http://127.0.0.1:<identity-port>"
export PATH="<repo-root>/target/debug:$PATH"
```

Sanity check:

```sh
fbrain signer public-key
```

prints Bob's principal key (his CLI identity is what the slice calls BobAgent;
devfinity runs exactly one sandbox container, so Bob's side is a host-side
`fbrain` identity backed by seeded Core roster rows — everything the Brain
server sees is real).

For the plan calls Bob makes below, define this signing helper in his
terminal (it is the same NIP-98 round trip the slice uses):

```sh
bob_brain_curl() {
  local method="$1" path="$2" body="${3:-}"
  local -a tags=(--tag "u,$FINITE_BRAIN_PUBLIC_BASE_URL$path" --tag "method,$method")
  if [[ -n "$body" ]]; then
    tags+=(--tag "payload,$(printf '%s' "$body" | shasum -a 256 | cut -d' ' -f1)")
  fi
  local event
  event="$(fbrain signer sign --kind 27235 "${tags[@]}")" || return 1
  if [[ -n "$body" ]]; then
    curl -fsS -X "$method" \
      -H "authorization: Nostr $(printf '%s' "$event" | base64 | tr -d '\n')" \
      -H 'content-type: application/json' --data "$body" \
      "$FINITE_BRAIN_SERVER_URL$path"
  else
    curl -fsS -X "$method" \
      -H "authorization: Nostr $(printf '%s' "$event" | base64 | tr -d '\n')" \
      "$FINITE_BRAIN_SERVER_URL$path"
  fi
}
```

## 2. Alice: open her Brain in the browser

1. Open the `dashboard:` URL printed at boot. The fixture stack signs you in
   as Alice (`devfinity@finite.computer`) automatically.
2. Open her agent's machine page and choose the **Brain** tab (the direct
   route is `<dashboard-url>/machines/<machine-id>/brain`). Her Personal
   Brain loads unlocked; the `shared-with-bob` Folder already contains
   `alice-seed.md`.

## 3. Direction 1 — Alice invites Bob from the browser

**Alice (browser):**

1. Open **Settings → Invitations**.
2. Under **Invite by email**, enter `bob@finite.vip` and click
   **Preview plan**.
3. Observe the resolved plan: the account owner `bob@finite.vip`, his managed
   agent (`bob-sidekick-...@finite.vip`), and any exclusions with their
   reasons. No npubs appear anywhere in the copy.
4. Click **Invite bob@finite.vip and 1 agent**. Inviting is member
   administration (ADR-0046 Tier 2), so the browser signer commits the plan
   directly — the click is the approval, no Approval Card is created. Observe
   "Invitation sent — Invited bob@finite.vip and 1 agent". If the roster had
   drifted since the preview, the client re-previews and shows the fresh plan
   instead of committing the stale one.
5. Still in **Settings → Invitations**, observe two pending invitations. Each
   pending invitation's `publicInstructionsUrl` (`…/llms.txt`) now resolves
   without auth for npub invitations too — it prints the target npub, the
   invitation id, and the exact accept command.

(The Approval Card round trip remains for agent-initiated requests — an agent
files an invite-commit Approval request and a human signs the card; the
Access-panel **Approvals** list stays as the pending/history view. A browser
signer without Brain admin standing gets a **Request approval from an admin**
fallback on the same previewed plan instead of the direct-invite button.)

**Bob (CLI):** Bob has no Working Tree yet, so `invite brain list` reads the
invitations addressed to his own principal (`GET /v1/my-invitations`) instead
of the admin collection — the list contains only his pending invitations, so
the id to accept comes straight from it:

```sh
fbrain invite brain list --json   # pending invitations addressed to Bob's principal
fbrain invite brain accept --id <invitation-id from the list> --json
fbrain brain list --json          # Alice's Brain is now visible
fbrain open <alice-brain-id> --json
```

Observe: the acceptance flips to `accepted`, and Alice's Brain appears in
Bob's list.

**Alice's agent grants the Folder key** (Alice's brain administers Folder keys
through her agent; run against her container, id printed by the gate):

```sh
docker exec <alice-container> fbrain admin folder-access grant \
  --brain <alice-brain-id> --folder shared-with-bob \
  --target "$(fbrain signer public-key)" --json
```

**Bob reads Alice's note and writes back:**

```sh
cd "$FBRAIN_WORKING_TREE_ROOT/<alice-brain-id>"
fbrain sync now --json            # repeat until the file appears
cat shared-with-bob/alice-seed.md # "this page existed before Bob joined"
printf '# Bob was here\n\nBob read Alice’s seed note.\n' > shared-with-bob/bob-reply.md
fbrain sync now --json
```

**Alice's side observes the reply:**

```sh
docker exec -w /data/workspace/finitebrain/<alice-brain-id> <alice-container> \
  fbrain sync now --json
docker exec <alice-container> \
  cat /data/workspace/finitebrain/<alice-brain-id>/shared-with-bob/bob-reply.md
```

## 4. Direction 2 — Bob invites Alice from the CLI, Alice accepts in the browser

**Bob (CLI):** create his own Brain, seed a note, then preflight and commit
the plan for Alice's account (a direct admin commit — no approval card, Bob is
the only key holder of his own Brain):

```sh
fbrain brain bootstrap-personal --json        # note .brain.brainId
export BOB_BRAIN=<brainId>
fbrain folder create shared-with-alice --brain "$BOB_BRAIN" \
  --name "Shared with Alice" --path shared-with-alice --json
fbrain open "$BOB_BRAIN" --json
cd "$FBRAIN_WORKING_TREE_ROOT/$BOB_BRAIN"
fbrain sync now --json
printf '# Bob seed\n\nThis page existed before Alice joined.\n' > shared-with-alice/bob-seed.md
fbrain sync now --json
cd -

bob_brain_curl POST "/v1/brains/$BOB_BRAIN/invitations/preflight" \
  '{"target":"devfinity@finite.computer"}' | tee /tmp/bob-preflight.json
```

Observe the plan: human `devfinity@finite.computer`, Alice's managed agent,
the roster revision, and `planId`/`planHash`.

```sh
BOB_PLAN_ID=$(node -e 'process.stdout.write(JSON.parse(require("node:fs").readFileSync("/tmp/bob-preflight.json","utf8")).planId)')
BOB_PLAN_HASH=$(node -e 'process.stdout.write(JSON.parse(require("node:fs").readFileSync("/tmp/bob-preflight.json","utf8")).planHash)')
bob_brain_curl POST "/v1/brains/$BOB_BRAIN/invitations/commit" \
  "{\"planId\":\"$BOB_PLAN_ID\",\"planHash\":\"$BOB_PLAN_HASH\"}" | tee /tmp/bob-commit.json
```

Observe: `status: "committed"`, one accept-ready invitation per principal.
Copy Alice's Invite Code (the invitation whose `ref` is
`devfinity@finite.computer`) and hand it to Alice.

**Alice (browser):**

1. Open **Settings → Invitations** and expand **Join a Brain**.
2. Paste the Invite Code and click **Join Brain**.
3. Observe "Joined the selected Brain..." — if the roster narrowed since the
   invite was sent, the notice names the excluded participants with reasons
   (still no npubs). Bob's Brain becomes the selected Brain in her switcher.

**Alice's agent accepts and Bob wraps the Folder key** (so content flows):

```sh
# Alice's agent (container): accept its per-principal invitation
docker exec <alice-container> fbrain invite brain list --json
docker exec <alice-container> fbrain invite brain accept --id <alice-agent-invitation-id> --json

# Bob grants Alice's agent access to the shared Folder
docker exec <alice-container> fbrain signer public-key   # Alice's agent principal
fbrain admin folder-access grant --brain "$BOB_BRAIN" \
  --folder shared-with-alice --target <alice-agent-principal> --json

# Alice's agent syncs and reads Bob's note
docker exec <alice-container> fbrain open "$BOB_BRAIN" --json
docker exec -w /data/workspace/finitebrain/$BOB_BRAIN <alice-container> \
  fbrain sync now --json            # repeat until the file appears
docker exec <alice-container> \
  cat /data/workspace/finitebrain/$BOB_BRAIN/shared-with-alice/bob-seed.md
```

Observe: Alice's agent decrypts `bob-seed.md` — "This page existed before
Alice joined."

## 5. What "good" looks like (provenance)

Alice's Brain DB (read-only) shows the signed origins of every membership:

```sh
sqlite3 -readonly <state-dir>/finite-brain/finite-brain.sqlite3 \
  "SELECT user_id, delegated_by_npub, origin_kind, origin_ref FROM brain_members ORDER BY brain_id, user_id"
```

- Direction 1 (browser direct commit): Bob's principals show
  `origin_kind=invitation`, `origin_ref=<Alice's invitation plan id>`,
  delegated by Alice's browser signer (her hosted identity).
- Direction 2 (CLI direct commit): Alice's principals in Bob's Brain show
  `origin_kind=invitation`, `origin_ref=<Bob's plan id>`, delegated by Bob's
  CLI principal.

## 6. Teardown

Press Ctrl-C in the boot terminal. The stack and its containers come down;
state stays under `.local-state/adr46-slice-<ts>/` for inspection.
