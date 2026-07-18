# ADR 0007: Make Folders The LLM Wiki Scope

Status: Accepted

Date: 2026-07-02

## Context

LLM Wiki treats each topic as an isolated wiki with its own `config.md`,
`_index.md`, `log.md`, sources, compiled pages, inventory, datasets, and
outputs. FiniteBrain adds encrypted Vaults, Folder Keys, and Folder Access.

If FiniteBrain used one Vault-level wiki log or index, organization Vaults
could leak private work through titles, summaries, source hints, or activity
records from restricted areas. The same structure also helps personal Vaults:
one person still needs separate durable wiki scopes for work, projects, life,
learning, and archives.

## Decision

A FiniteBrain Vault is a namespace of many Folder-scoped LLM wikis. A Folder is
the wiki scope because it is the encrypted access boundary.

Vaults start empty under ADR 0021. A human or authorized agent explicitly
creates each Folder-scoped wiki and chooses its access mode; Brain does not
represent onboarding or access modes with permanent example Folders.

## Consequences

- Folder-local `_index.md` and `log.md` must describe only their own Folder.
- Root/global indexes must not reveal inaccessible Folder titles, summaries,
  source hints, or activity.
- Agent and client querying must filter by Folder access before content reaches
  the LLM.
- Cross-Folder synthesis should be written to the most restrictive appropriate
  Folder for the sources used.
- Local directories remain layout inside a Folder; they do not create new
  access boundaries.
