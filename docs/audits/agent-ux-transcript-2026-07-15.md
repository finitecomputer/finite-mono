# Agent UX Transcript Audit — Image Generation And Sites Editing

Date: 2026-07-15

Status: guidance-only remediation; no product or Runtime code changed.

## Findings and changes

- **Image model selection:** The Agent explored unrelated inference services
  before discovering Hermes's supported image model setting. The image skill
  now routes quality requests directly to the native provider/model config,
  includes a compact model guide, and distinguishes one-off selection from a
  persistent default change.
- **Sites email access:** The Agent initially asked the human to copy a Sites
  token even though its connected Google Workspace mailbox could retrieve it.
  The Sites skill now tells the Agent to use that connection when the human
  authorizes it, verify the mailbox match, and ask for help only on failure or
  ambiguity.
- **Google Workspace:** The skill now states that explicit authorization for a
  current task includes retrieving and using the newest matching login email,
  without echoing the token.
- **Attachment delivery:** Current `main` already advertises Finite Chat's
  native `MEDIA:/absolute/path/to/file` contract; no new change was needed.

## Replay checks

1. Ask for a better anime image model. The Agent should use native image config
   without credential searches and restore a one-off model choice afterward.
2. Ask the Agent to edit a Site through an email grant, then tell it the
   connected mailbox has access. It should verify Google Workspace auth, fetch
   the fresh Sites message, redeem the token without exposing it, and continue.
3. Remove or mismatch mailbox access. The Agent should stop and ask the human
   for the code instead of guessing.
4. Ask for the generated file as an attachment. The Agent should use the native
   `MEDIA:` delivery contract.

`just skills check` is the static floor; replay on a disposable Agent is the
behavioral proof.
