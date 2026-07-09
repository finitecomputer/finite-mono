# ADR 0006: Agent Invite Flow — URL, QR, and Challenge PIN

Status: superseded 2026-07-08 by
[ADR 0010: Welcome-First Room Admission](0010-welcome-first-room-admission.md).
This document is retained as historical context for the abandoned invite-session
design; it is not the current protocol contract.

## Context

The flagship onboarding UX: an agent (Hermes running beside a `finitechat`
binary) prints a URL and a QR code. The user scans or pastes it into the
Finite Chat app, types a short challenge code shown on the agent's terminal,
and lands in an E2EE chat with the agent. Least friction possible; each agent
gets its own Nostr identity (npub) that exists only on its home server — no
public relay registration.

Prior art studied: **agentnoise** (nvk) pairs a White Noise phone app with a
desktop agent. Its QR encodes the bare npub; the *phone* initiates the MLS
group; a rotating 6-digit HMAC PIN typed as the **first message inside the
already-established chat** gates the agent's attention. Hermes itself uses
8-character approval codes with owner-side approval.

Two structural differences let us do better than both:

- We have ordered server rendezvous (ADR 0001), so the gate can run **before
  the MLS add** — an unverified stranger never enters the group at all. In
  agentnoise the stranger is in the group and merely ignored.
- We have invite codes carrying a room-server address (ADR 0005), so the
  invite is also the discovery mechanism — no relay-hint nprofiles needed.

This flow is also the first consumer of ADR 0005 items 1 and 2 (room-server
addressing on client room records; versioned invite format with the address
field first).

## Decision 1: the invite URL (invite code v1)

One canonical form, fielded and versioned:

```
finite://join?v=1&s=<server-url>&r=<room-id>&i=<invite-id>&t=<invite-token-hex>&a=<agent-npub>&n=<display-name>
```

- `v` — format version; parsers reject unknown versions. **`s` (the room
  server address) is the first-class field**, per ADR 0005: joining a room is
  discovering where it lives.
- `r` — the room the invite admits you to (the agent creates the room before
  inviting; it is creator/admin per ADR 0003 §2).
- `i` — invite session id (rendezvous key on the server).
- `t` — 16-byte random **invite token**, hex. A bearer secret: it lives only
  in the URL/QR, never alone on the server.
- `a` — the agent's account npub (NIP-19 bech32 of the account public key,
  which *is* the Finite `AccountId` in different clothing). Lets the app show
  "Chat with ⟨name⟩ (npub1…)" before joining and **verify the agent after
  joining** (the joined group must contain a member whose credential resolves
  to this account).
- `n` — optional human label, display-only, never trusted.

The QR code is simply this URL (terminal Unicode rendering, agentnoise-style
`qrcode` crate). The same string pastes into the app by hand. Apps may also
register the `finite://` scheme so tapping a link joins directly.

## Decision 2: two factors, verified by the inviter, gated before the add

The server hosting the rendezvous is **not** trusted to authenticate anyone
(ADR 0001 posture). Verification is inviter-side, and nothing joins the MLS
group until it passes.

- **Invite token** (in the URL): proves the joiner saw the QR/URL. High
  entropy, so the rendezvous server cannot forge a join request, and a
  6-digit brute force is useless without it.
- **Rotating challenge PIN** (on the agent's terminal, typed by the user):
  proves the joiner is looking at the agent's terminal *now* and consents
  *now*. Covers the case where the URL traveled over a weaker channel
  (email, SMS) and might have leaked: possession of the stale URL alone is
  not enough.

PIN derivation (agentnoise's window scheme, our domain separation):

```
window = unix_seconds / 30
pin    = u32_be(HMAC-SHA256(key=invite_token, msg="finite-invite-pin-v1" || be64(window))[0..4]) % 1_000_000
```

The joiner never sends the raw PIN. The join request carries a **proof**
binding the PIN and token to the joiner's exact identity and key material:

```
pin_proof = hex(HMAC-SHA256(
    key = invite_token || ascii(pin),
    msg = "finite-join-proof-v1" || account_id || 0x00 || device_id || 0x00 || sha256(key_package_bytes)))
```

The inviter recomputes the proof for the current window ±1 (clock skew) and
accepts on match. Consequences: the server can neither replay a proof for a
different key package, nor brute-force the 6-digit space (it lacks the
token), nor substitute the joiner's KeyPackage (the proof pins its hash).
Single-secret-channel honesty: when the QR is scanned in person, both
factors arrive together and the trust anchor is physical presence — that is
the intended UX, and it is strictly stronger than agentnoise's
public-npub-plus-post-join-PIN.

Joiner-side verification is mandatory too: after activating the Welcome, the
client checks the group contains a member whose validated credential matches
`a`. A hostile server that admits the user to a different group fails this
check and the client surfaces it and leaves.

## Decision 3: the join request carries the KeyPackage inline

MLS requires the inviter to hold the joiner's KeyPackage. Instead of the
joiner publishing into their standing inventory (home-server state, and in
the sharded future the *wrong* server), the join request **carries the
KeyPackage bytes inline** as single-use rendezvous material. The agent takes
it straight from the join request into the Add commit.

This keeps the ADR 0005 taxonomy clean (KeyPackage inventory routes stay
home-scoped and room-free), makes the invite flow work unchanged when the
room server is a stranger's self-hosted box, and removes a round trip.

## Decision 4: invite sessions — home-scoped ephemeral rendezvous

New durable server state, modeled on link sessions (create → upload → claim
lifecycle, op-logged, restart-safe), added to the ADR 0005 home-scoped list
as ephemeral state with a TTL (regenerable by construction — an expired
invite is replaced by printing a new one):

| Route | Caller | Effect |
| --- | --- | --- |
| `/invites` | inviter | create session: invite_id, room_id, inviter device, max_joins, ttl |
| `/invites/join` | joiner | submit join request: request_id, account/device, key_package bytes, pin_proof, display name; idempotent by request_id |
| `/invites/requests` | inviter | poll pending join requests |
| `/invites/respond` | inviter | mark a request accepted/rejected (bookkeeping for joiner UX; the real admission is the Add commit + Welcome) |
| `/invites/status` | joiner | poll own request state (pending/accepted/rejected/expired) |
| `/invites/expire` | inviter/ops | close a session |

The server stores all of it opaquely — it never sees the invite token,
never verifies a proof, and cannot mint a passing join request. Limits in
the finite style: join requests per invite (64), open invites per account
(256), KeyPackage bytes per request (existing `MAX_KEY_PACKAGE_PAYLOAD_BYTES`),
TTL capped at 7 days (default 24 h), `max_joins` capped at 64 (default 1).
Accepted/expired sessions fall under the retention horizon like everything
else.

The happy-path wire sequence, end to end:

```
agent:  POST /invites                      (after creating the room)
        print URL + QR + rotating PIN
user:   scan QR → app parses v1 fields → user types PIN
        POST /invites/join                 (KeyPackage inline, pin_proof)
agent:  POST /invites/requests → verify proof → POST /commits (Add + Welcome bundle)
        POST /invites/respond accepted
user:   POST /invites/status → accepted → POST /welcomes/claim → /welcomes/ack
        verify agent npub in member list → chatting
```

Every leg after the QR scan is machine-paced: the target is sub-second from
PIN entry to "you're in" on a local network.

## Decision 5: agent identity — an npub that never touches a relay

`hermes init` generates a standard Nostr keypair. The account public key is
already the Finite `AccountId` (hex form); npub is its NIP-19 display form.
The secret key is written 0600 inside the agent's data directory (Linux
containers have no keychain; operators can mount secrets however they like).
"Publishing" the identity to the home server is exactly the existing
KeyPackage publication plus account-room bootstrap — there is no relay,
no kind-0 profile event, no registration step. The npub in the invite URL is
the only place the identity needs to appear for a human.

## Decision 6: the bridge grows an onboarding surface

The `finitechat hermes` JSON bridge (ADR 0002; the subcommands are rebuilt
over the real client after the engine deletion) gains: `init`, `invite`
(create room + invite session, emit URL/QR/PIN), `pin` (current PIN for an
open invite, for headless re-display), and join-request processing inside
`poll` (verify proofs, commit adds, emit a joined event). The Python plugin
stays thin per ADR 0002: translate callbacks, never own crypto or state.

## Consequences

- The first-contact story matches the dream: one binary + one plugin, QR on
  the terminal, PIN, chatting — with verification *stronger* than the prior
  art it imitates and no public infrastructure dependencies.
- The invite code is the first shipped artifact of ADR 0005: room addressing
  is in the URL from day one, so per-room servers later require no invite
  format change.
- A new home-scoped ephemeral state (invite sessions) joins the closed list
  in ADR 0005 §5, with TTL as its migration story.
- The PIN gate runs before group admission, so a Finite room never contains
  an unverified member — there is no "ignored stranger inside the group"
  state to reason about.
