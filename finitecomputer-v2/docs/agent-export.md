# Agent export

A person with access to an Agent downloads everything it has as one zip from
the dashboard's Export page, or by typing `/export` in web chat, which opens
that page. The model never builds or sends the export, and no chat attachment
carries it, so no attachment size limits it.

## Path

```text
Browser -- native Hermes grant --> finite-export plugin: status, survey, export, delete
Browser -- same-origin GET ------> dashboard /api/agents/{runtime}/export/download
dashboard -- Core grant ---------> finite-export plugin: download (Range passes through)
```

The plugin (`finite-agentd/integrations/hermes/finite-export/`) ships in the
Hermes package beside the Brain and Sites plugins. Native Hermes authenticates
before routing, exactly as for those pages. A browser download cannot carry the
native bearer, so the dashboard fetches the file with an operation-local grant
and streams it through without buffering.

The plugin runs only `export.py` beside it, as `survey --everything` and
`build --everything`, in its own process group. Requests name brains and sites
from the latest survey and nothing else: no path, command or flag.

## State and limits

Everything lives under `$FINITE_HOME/exports/` on the Agent's disk: a `survey`
and an `export` folder, each with the job record, the exporter's output and its
exit status. The folder alone tells a running, finished, failed or interrupted
job apart, so a Hermes restart never loses a finished export or reports a dead
one as running. Staging happens there too, not in `/tmp`.

- One survey or export runs at a time; a new one replaces the last.
- A finished export is deleted seven days after it ends.
- The disk is shared with the host's other Agents. A survey starts only with
  room for a copy of the chat store plus 20 GiB; an export only when 20 GiB
  would stay free after its staging copy and its zip. A running survey or
  export stops, and frees its staging copy, if free space falls below 10 GiB,
  since sites and brains not yet on the Agent can't be sized in advance.
- A job whose record can't be written is stopped at once, so nothing runs
  untracked.

## What the zip holds

`finite-agent-export/1`, the format move.finite.place uses, with nothing left
out for size: chats, memories, personality, skills, scheduled jobs, the sites
the Agent owns or the person picks, the brains they pick, and every workspace
file including media. Credentials are handled the same way in every section:

- Files named like credentials (`.env`, keys, tokens) never go in.
- Memories and scheduled jobs go in with anything shaped like a credential,
  whole private keys included, replaced by `[REDACTED]`.
- Every other file is read: text in UTF-8, or UTF-16 with a byte order mark,
  at any size, and SQLite databases value by value. One that looks like it
  holds a credential, or can't be checked, stays out, as do all archives.
- Chats, and media and other binary files, go as they are. Anything pasted
  into a chat is in the export; the page and the zip's README say so.

Dependencies, build output and earlier exports stay out too. `manifest.json`
lists every file with its SHA-256, and everything left out and why.

`export.py` is the move guide's exporter, vendored once. Serve this copy from
move.finite.place rather than changing two.

## Compatibility and recovery

An Agent Runtime without the plugin answers 404, and the page shows "This agent
does not support this page yet." The feature reaches an Agent with a Runtime
rollout. Dashboard and Runtime can ship in either order. An export is the
person's copy, not a backup or part of the Recovery Set (ADR 0001), and exporting
never stops, retires or deletes the Agent.
