# Chat `@` references v1

Status: product direction approved, implementation in progress.

## Goal

Typing `@` in the Agent chat composer opens one mixed, keyboard-accessible
search over:

- Markdown files in the selected Agent's workspace;
- files already uploaded in the current chat;
- skills active in the selected Agent Runtime; and
- Finite Sites the Agent owns or may edit.

The feature saves the user from spelling out paths, URLs, and skill names. It
does not create account-wide memory search, semantic search, or new edit
authority.

## Reference behavior

| Kind | Search identity | Turn behavior |
| --- | --- | --- |
| Workspace file | Stable Agent-workspace-relative path plus observed fingerprint | Ask the Agent to open and use the path for this turn, verify the fingerprint, and ask before using a changed version. |
| Uploaded file | Existing encrypted Chat attachment | Reattach the immutable bytes through the ordinary attachment path. |
| Skill | Active skill name plus description | Explicitly require the Agent to load and attempt the skill; if it cannot, it explains why and offers an alternative. |
| Site | Finite Site output id and canonical public URL | Give the Agent the exact URL and site identity. The user's prose supplies the requested action; existing Sites authorization decides whether edits are allowed. |

References are one-turn inputs, matching ordinary attachments. The transcript
renders structured File, Skill, and Site labels so the user can distinguish
what was supplied. Skill references use a blue treatment, an icon, and a
visible `Skill` label; color is not the only distinction.

Workspace-file references retain the source fingerprint observed at selection.
The Agent verifies it before use. If the source changed, it tells the user and
asks whether to use the update; an old turn never silently claims the updated
file was the selected version.

## Search and interaction

- Bare `@` shows recent results.
- Filtering ranks exact names, then names, headings/text, paths, and recency.
- v1 uses lexical search. Semantic/vector search is deferred.
- Results keep all colliding kinds and paths visible; the user chooses one.
- Arrow keys move immediately, Enter or Tab selects, and Escape dismisses.
- A selected result becomes minimal `@name` text flowing inline with the
  surrounding sentence. The durable message still carries typed structured
  reference data, and deleting the inline text removes that reference.
- `@` opens only at the beginning of the draft or after whitespace or
  punctuation. It does not open inside an email address or fenced/inline code.
- Search must respond within 400 ms with local/current-chat results. Runtime
  results may fill in progressively without moving the active keyboard row.

## Trust boundaries

- The dashboard never receives arbitrary Runtime filesystem access. It sends a
  bounded lexical query over the encrypted Agent Platform Channel.
- `finite-agentd` searches only configured workspace and active-skill roots,
  never follows symlinks, returns bounded metadata/snippets, and accepts no
  caller-supplied path.
- Hidden workspace entries, files outside the workspace, and inactive
  catalog-only skills are excluded.
- Site selection conveys identity, not authority. Mutations still use the
  existing `fsite` authorization boundary.
- Existing Chat attachment encryption, retention, and deletion behavior remains
  authoritative for uploaded-file references.

## Acceptance

Typing `@pric` can select `plans/pricing-plan.md`; the same turn can select the
active `strategy-review` skill. The composer and transcript visibly distinguish
the File and Skill. The Agent opens the selected workspace file, loads and
attempts the selected skill, and clearly reports an unavailable skill or
missing Sites authority instead of silently proceeding as if it succeeded.
