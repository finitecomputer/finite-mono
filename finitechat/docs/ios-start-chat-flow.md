# iOS Start Chat Flow

Status: proposal after the first successful local and remote Docker Hermes
phone canaries.

## Problem

The app currently presents "Scan" and "New" as separate choices. That makes the
user decide which protocol object they have before the app has inspected it.
For Finite Chat this is the wrong split: an invite URL, QR code, npub,
nprofile, pasted hex key, cached person, and future agent creation request are
all ways to start or resume a conversation.

## Product Rule

There should be one primary entry point: **Start Chat**.

The Start Chat flow owns discovery, paste, scan, direct-person chat, invite
join, and future hosted-agent creation. The user should never have to choose
"scan" versus "new" up front.

## Proposed Flow

1. The top-level compose/start button opens `StartChatSheet`.
2. The first field accepts anything chat-address-like:
   invite URL, QR result, npub, nprofile, hex public key, profile URL, or a
   plain search query for cached people.
3. Camera and paste are tools beside that field, not separate destinations.
4. As soon as the app recognizes a Finite invite, it scans it through the Rust
   runtime and routes to the pending room.
5. A pending invite room starts with a blank PIN entry. After PIN submission,
   the PIN form disappears and the room shows a waiting state until the agent
   admits the device.
6. As soon as the app recognizes a profile/person, it shows the resolved person
   row and a single start-chat action.
7. Cached people search and direct code entry live in the same sheet results
   list.
8. Future "create a hosted agent" lives in the same flow as a result/action,
   not as a separate top-level branch.

## Musts

- One top-level user decision: start a chat.
- Scan and paste are input methods, not destinations.
- PIN entry is blank for each invite attempt.
- PIN submission is single-flight.
- Waiting-for-agent-admission is visibly pending and non-actionable.
- Existing rooms should route directly to the room instead of creating a second
  path.
- Profile codes and cached people should share the same direct-chat creation
  action.

## Must Nots

- Do not keep a separate "Scan" sheet and "New Chat" sheet after this lands.
- Do not ask the user whether something is an invite or a profile code.
- Do not prefill a previously submitted PIN.
- Do not leave the Join button active after the join request has been sent.

## Evaluation

Add or keep tests that prove:

- Scanning/pasting an invite clears any stale PIN draft and opens the pending
  room.
- Submitting a PIN clears the draft and prevents another submit while the room
  status is waiting for admission.
- Scanning/pasting a profile code returns the same direct-chat target as
  selecting that cached person from the people list.
- Tapping the single Start Chat entry point can drive invite join, profile chat,
  and cached-person chat without launch flags.
- The phone canary invite/PIN flow still passes against local Hermes and remote
  Docker Hermes.

