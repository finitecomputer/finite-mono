# CLI Essentials

## Canonical Execution

Prefer the installed binary:

```bash
nostr-agent-interface cli <tool-or-command>
```

If that binary is unavailable and you are inside the checked-out product repo with built artifacts present, use:

```bash
node build/app/index.js cli <tool-or-command>
```

Do not use the legacy `nostr-mcp-server` alias for CLI workflows. That alias exists for MCP compatibility, not as the preferred CLI path.

## Discovery

Start from live contract data:

```bash
nostr-agent-interface cli list-tools --json
nostr-agent-interface cli <toolName> --help
```

When docs and examples drift, trust CLI help and the live contract from `list-tools --json`.

## Supported Invocation Styles

### 1. Schema-aware flags

Best for simple non-secret inputs.

```bash
nostr-agent-interface cli getProfile --pubkey npub...
nostr-agent-interface cli convertNip19 --input npub... --target-type hex --json
```

### 2. JSON positional object

Best for small structured payloads that are not sensitive.

```bash
nostr-agent-interface cli getProfile '{"pubkey":"npub..."}' --json
```

### 3. JSON via stdin

Best for secrets, nested objects, or larger payloads.

```bash
printf '%s' '{"input":"npub...","targetType":"hex"}' \
  | nostr-agent-interface cli convertNip19 --stdin --json
```

## Output Controls

Use `--json` when another tool, script, or agent step needs structured output.

Use `NOSTR_JSON_ONLY=true` together with `--json` when stderr noise could break parsing:

```bash
NOSTR_JSON_ONLY=true nostr-agent-interface cli list-tools --json
```

## Secret Handling

Do not put secrets such as `privateKey` or `authPrivateKey` in argv when stdin is available. Prefer:

```bash
printf '%s' '{"privateKey":"nsec...","content":"hello nostr"}' \
  | nostr-agent-interface cli postNote --stdin --json
```

In user-facing summaries, describe the action and result without echoing raw private keys unless the user explicitly asked to see them.
