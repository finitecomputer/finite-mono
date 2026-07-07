---
name: monday-com-finite
description: Use when the user wants to inspect or update Monday.com boards, items, updates, or workspaces through Finite's machine-local Monday MCP integration. Only rely on it when the machine's Monday integration is connected from the dashboard.
---

# Monday.com via Finite

Use Monday through the platform-managed Monday MCP integration. Prefer the injected MCP tools over raw GraphQL `curl` or ad hoc token handling.

## First checks

- If Monday tools are missing, tell the human to connect Monday from the dashboard Integrations section for this machine.
- Do not ask for a shared Monday API key.
- Do not tell the human to paste a personal token into `~/.hermes/config.yaml`.

## Working style

- Start read-only: list boards, inspect columns, and understand board-specific status labels before mutating anything.
- When changing items or updates, make the smallest targeted change possible and summarize exactly what changed.
- If the human just connected Monday and the tools are still missing, wait for the machine restart to finish or start a fresh session after the integration activates.

## Good uses

- List boards, groups, items, and updates
- Inspect board schema before changing statuses or people columns
- Create or update items after confirming the target board and columns
- Read or post updates on behalf of the connected user
