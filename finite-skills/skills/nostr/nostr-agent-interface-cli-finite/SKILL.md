---
name: nostr-agent-interface-cli-finite
description: Use when an agent needs to inspect, discover, or invoke Nostr Agent Interface CLI tools through `nostr-agent-interface cli`. Ideal for local shell-based Nostr workflows, schema-aware discovery, machine-readable CLI output, stdin-based secret handling, and read-before-write command selection. Do not use this skill for MCP-native or HTTP API-first tasks.
license: MIT
compatibility: Requires a local installation of nostr-agent-interface or a checked-out repo with build/app/index.js present.
metadata:
  author: AustinKelsay
  version: "0.1.0"
---

# Nostr Agent Interface CLI

## What This Skill Does

This skill teaches agents to use `nostr-agent-interface cli` as the primary interface for local shell-based Nostr work. It focuses on safe command selection, live schema discovery, structured output, stdin-based secret handling, and conservative behavior around destructive actions.

## When To Use

Use this skill when:

1. the user explicitly wants to use the Nostr Agent Interface CLI
2. the task is a local shell-based Nostr workflow
3. the agent needs to discover or inspect tool schemas before running commands
4. the task benefits from `--json` output for machine-readable downstream steps

## When Not To Use

Do not use this skill when:

1. the user wants MCP mode or an MCP-native client setup
2. the user wants the HTTP API instead of the CLI
3. the environment cannot run either `nostr-agent-interface` or `node build/app/index.js cli`

## Inputs You Need

Collect only what the selected CLI tool actually requires, such as:

1. a tool name
2. a pubkey, event id, relay list, or other tool inputs
3. whether the result should be plain text or `--json`
4. whether sensitive fields such as `privateKey` or `authPrivateKey` are involved

## Workflow

1. Prefer this command prefix:

```bash
nostr-agent-interface cli
```

2. If the binary is unavailable and you are clearly inside a checked-out product repo with built artifacts, fall back to:

```bash
node build/app/index.js cli
```

Do not assume source `.ts` entrypoints are directly runnable under `node`.

3. Discover the live tool contract first:

```bash
nostr-agent-interface cli list-tools --json
```

4. Inspect a specific tool when the input shape is not already clear:

```bash
nostr-agent-interface cli <toolName> --help
```

5. Prefer schema-aware flags for simple non-secret arguments.
6. Prefer `--stdin --json` for secrets, nested objects, or larger payloads.
7. Prefer `--json` for machine parsing.
8. Prefer `NOSTR_JSON_ONLY=true` with `--json` when stderr noise could break downstream parsing.
9. Use read-first workflows before writes.
10. Routine writes may proceed when the user intent is explicit.
11. Ask for extra confirmation before destructive or account-shaping actions such as `deleteEvent`, `deleteBlob`, `unfollow`, `setRelayList`, or `setBlossomServers`.
12. Normalize ambiguous keys or entities with `analyzeNip19` or `convertNip19` before mutating when needed.

## Validation

The skill has done its job when:

1. the selected command uses `nostr-agent-interface cli` or the documented repo-build fallback
2. discovery comes from `list-tools --json` or `<toolName> --help` rather than stale prose docs
3. `--json` is used when structured output is useful
4. secrets are passed through stdin rather than argv
5. destructive or account-shaping actions are not executed without extra confirmation
6. error reporting includes the exact tool error plus a sanitized argument summary

## Common Failure Modes

1. Binary missing:
   Use the repo-build fallback only when a checked-out product repo with `build/app/index.js` is clearly present.
2. Docs drift:
   Trust `list-tools --json` and `<toolName> --help`.
3. Relay-specific failures:
   Retry once with explicit relays when the selected tool supports them.
4. Wrong transport:
   If the request is really about MCP or HTTP API usage, switch away from this skill.

## References

Read only what you need:

1. `references/cli-essentials.md` for command forms, JSON/stdin usage, and secret handling
2. `references/tool-groups.md` for the grouped current tool surface
3. `references/workflows.md` for short end-to-end command templates
