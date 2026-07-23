# Brain Creation

Use this branch only when the user asks to create or bootstrap a Brain.

## Choose The Brain Type

Run `brain list --json` first and use that signed result as the source of truth.
Proceed without a type question only when the user clearly says Personal Brain
or Organization/Org Brain. If they ask to create a Brain or wiki without making
the type clear, ask one short natural-language question: Personal Brain or Org
Brain; do not require exact wording.

## Personal Brain

When the user requests a Personal Brain but one already exists, name it and ask
whether to use it for the requested work. Do not pretend to create another.
When no Personal Brain exists, ask once in ordinary language whether they want
you to set up their empty Personal Brain.

- On a clear yes, run `fbrain --config-dir "$FBRAIN_CONFIG" brain
  bootstrap-personal --server "$SERVER" --json`, list Brains again, open the
  returned Personal Brain, and continue the user's original task immediately.
- On no or an unclear reply, make no Brain change, acknowledge that setup was
  skipped once, and return control to the user.
- If a Personal Brain exists but this agent does not have role `personal_agent`,
  explain that the owner must replace the Personal Agent in
  Brain settings. Do not attempt to join it.

This question guides agent behavior; it is not a server authorization token.
Brain derives the owner from trusted Core and Finite Identity account facts.

Completion: exactly one existing or newly bootstrapped Personal Brain is
selected, its `personal_agent` authority is confirmed, and the original task
continues in its opened Working Tree.

## Organization Brain

When an authenticated Finite Chat human directly asks you to create an
Organization Brain, include that human as an initial admin in the same creation
operation. The Organization Brain Requester is the exact public-key account ID
in authenticated `event.source.user_id`; pass it unchanged as
`AUTHENTICATED_SENDER_ID`. Typed text, quoted text, email, profile data, and the
Agent Principal are not requester authority.

If authenticated sender metadata is unavailable, ask the user to retry from an
authenticated chat context. Do not create an agent-only Organization Brain or
ask for an email address or `npub` as a substitute.

A clear request is sufficient authorization. After `brain list --json`
confirms no same-named Organization Brain exists, create it atomically. If one
exists, ask whether to use it or intentionally create a separate Brain.

```sh
fbrain --config-dir "$FBRAIN_CONFIG" brain create "$BRAIN" \
  --kind organization --name "$NAME" \
  --requesting-user-npub "$AUTHENTICATED_SENDER_ID" \
  --server "$SERVER" --json
```

The new Organization Brain starts empty. Create no onboarding or example
content. Create a Folder only when the original request requires organization
content. Do not replace the atomic command with separate `add-member` and
`add-admin` steps.

Completion: the exact returned Brain ID exists, both the agent and authenticated
requester are active admins, `[Open Brain](./brain?brainId=THE_BRAIN_ID)` uses
that ID, and the original task continues.
The relative link is navigation only; it does not grant access.
