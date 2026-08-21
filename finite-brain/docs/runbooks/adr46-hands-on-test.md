# ADR-0046 Hands-On Test: Hosted-Signed Invite + CLI Accept, Both Directions

Status: **retired 2026-08 (auth kernel cut).** The invitation-plan flow this
runbook exercises is deleted, and its drivers (`scripts/devfinity-adr46-slice`,
`scripts/devfinity-adr46-slice-up`) are removed. Retained for history; the
surviving invite flows are npub-targeted invitations and capability Invite
Tokens.

Two roles, one Mac:

- **Alice** drives her hosted chat signature through a terminal helper (the
  same NIP-98 round trip the old Brain browser client drove; the web client is
  deleted and a human Brain UI returns with the brain-surface viewer plan).
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

- Alice's hosted-signing environment block (hosted device URL plus her
  terminal helper — see step 2),
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

## 2. Alice: set up the hosted-signing helper (once)

Alice's acts sign Brain HTTP with her hosted chat device (her human
Principal key). In a third terminal, export the environment the hands-on
gate printed (hosted device URL and API token — the token also lives in the
boot terminal as `$FINITECHAT_HOSTED_API_TOKEN`) and define the helper:

```sh
export ALICE_SUBJECT="user_devfinity"
export FC_HOSTED_WEB_DEVICE_URL="http://127.0.0.1:<hosted-device-port>"
export FINITECHAT_HOSTED_API_TOKEN="<from the boot terminal>"
export FINITE_BRAIN_PUBLIC_BASE_URL="http://127.0.0.1:<dashboard-port>"
export FINITE_BRAIN_SERVER_URL="http://127.0.0.1:<brain-port>"

alice_brain_curl() {
  local method="$1" path="$2" body="${3:-}"
  local url="$FINITE_BRAIN_PUBLIC_BASE_URL$path"
  local input event auth
  input="$(node -e '
    const crypto = require("node:crypto");
    const [method, url, bodyText] = process.argv.slice(1);
    const tags = [["u", url], ["method", method.toUpperCase()],
      ["nonce", crypto.randomBytes(16).toString("hex")]];
    if (bodyText) tags.push(["payload", crypto.createHash("sha256").update(bodyText).digest("hex")]);
    process.stdout.write(JSON.stringify({
      method: method.toUpperCase(), url, bodyText,
      eventTemplate: { kind: 27235, created_at: Math.floor(Date.now() / 1000), tags, content: "" },
    }));
  ' "$method" "$url" "$body")"
  event="$(curl -fsS -H "authorization: Bearer $FINITECHAT_HOSTED_API_TOKEN" \
    -H "x-finite-workos-user-id: $ALICE_SUBJECT" -H 'content-type: application/json' \
    --data "$input" "$FC_HOSTED_WEB_DEVICE_URL/v1/brain/identity-provider")" || return 1
  auth="Nostr $(node -e '
    const signed = JSON.parse(require("node:fs").readFileSync(0, "utf8"));
    process.stdout.write(Buffer.from(JSON.stringify(signed), "utf8").toString("base64"));
  ' <<<"$event")"
  if [[ -n "$body" ]]; then
    curl -fsS -X "$method" -H "authorization: $auth" \
      -H 'content-type: application/json' --data "$body" "$FINITE_BRAIN_SERVER_URL$path"
  else
    curl -fsS -X "$method" -H "authorization: $auth" "$FINITE_BRAIN_SERVER_URL$path"
  fi
}
```

Sanity check (prints Alice's npub):

```sh
node -e 'process.stdout.write(JSON.stringify({version:"finite-brain-identity-provider-v1",operation:"identifyMember",input:null}))' |
  curl -fsS -H "authorization: Bearer $FINITECHAT_HOSTED_API_TOKEN" \
    -H "x-finite-workos-user-id: $ALICE_SUBJECT" -H 'content-type: application/json' \
    --data @- "$FC_HOSTED_WEB_DEVICE_URL/v1/brain/identity-provider"
```

## 3. Direction 1 — Alice invites Bob with her hosted signature

**Alice (terminal):**

1. Preview the plan for Bob's account:

```sh
alice_brain_curl POST "/v1/brains/<alice-brain-id>/invitations/preflight" \
  '{"target":"bob@finite.vip"}' | tee /tmp/alice-preflight.json
```

2. Observe the resolved plan: the account owner `bob@finite.vip`, his managed
   agent (`bob-sidekick-...@finite.vip`), and any exclusions with their
   reasons. No npubs appear anywhere in the copy.
3. Commit the plan directly. Inviting is member administration (ADR-0046
   Tier 2), so Alice's signature commits the plan — the commit is the
   approval, no Approval Card is created:

```sh
ALICE_PLAN_ID=$(node -e 'process.stdout.write(JSON.parse(require("node:fs").readFileSync("/tmp/alice-preflight.json","utf8")).planId)')
ALICE_PLAN_HASH=$(node -e 'process.stdout.write(JSON.parse(require("node:fs").readFileSync("/tmp/alice-preflight.json","utf8")).planHash)')
alice_brain_curl POST "/v1/brains/<alice-brain-id>/invitations/commit" \
  "{"planId":"$ALICE_PLAN_ID","planHash":"$ALICE_PLAN_HASH"}"
```

If the roster drifted since the preview, the commit returns 409 with a fresh
preflight — re-run step 3 with the new plan instead of committing the stale
one.
4. Each pending invitation's `publicInstructionsUrl` (`…/llms.txt`) resolves
   without auth for npub invitations too — it prints the target npub, the
   invitation id, and the exact accept command.

(The Approval Card round trip remains for agent-initiated requests — an agent
files an invite-commit Approval request and a human signs the card. A signer
without Brain admin standing falls back to requesting approval on the same
previewed plan instead of committing directly.)

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

## 4. Direction 2 — Bob invites Alice from the CLI, Alice accepts with her hosted signature

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

**Alice (terminal):**

1. Accept the invitation with her hosted signature:

```sh
alice_brain_curl POST "/v1/brain-invitation-links/<invite-code>/accept"
```

2. Observe the acceptance (`status: "accepted"`, an `acceptedAt` timestamp).
   If the roster narrowed since the invite was sent, the response names the
   excluded participants with reasons (still no npubs).

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

- Direction 1 (hosted-signature direct commit): Bob's principals show
  `origin_kind=invitation`, `origin_ref=<Alice's invitation plan id>`,
  delegated by Alice's hosted chat signature (her human Principal).
- Direction 2 (CLI direct commit): Alice's principals in Bob's Brain show
  `origin_kind=invitation`, `origin_ref=<Bob's plan id>`, delegated by Bob's
  CLI principal.

## 6. Teardown

Press Ctrl-C in the boot terminal. The stack and its containers come down;
state stays under `.local-state/adr46-slice-<ts>/` for inspection.
