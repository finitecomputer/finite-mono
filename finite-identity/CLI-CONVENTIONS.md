# Finite CLI Auth Conventions

Every Finite CLI (`fsite`, `finitechat`, `fbrain`) exposes the same two auth
verbs, backed by this crate. CLIs MUST NOT reimplement secret parsing or
identity-file writing — `ImportSecret::parse` and `FiniteIdentity::import`
are the only path, so every tool refuses the same bad inputs and writes the
same file the same way.

## `auth status`

Shows the current identity without touching it:

- **npub** (NIP-19 display form; hex may be shown alongside)
- **identity file path** (resolved per SPEC.md; reflects `FINITE_HOME`)
- **created_by** and **created_at** from the file

Honors the CLI's existing JSON output convention (`--json` or
`--output json`, whichever is that CLI's house style) with the same four
fields. If no identity exists, say so and point at `auth import` or the
tool's normal first-run mint; do not mint from `status`.

## `auth import`

Adopts an existing secret as the Finite identity:

- Reads an `nsec1...` string or 64-char hex secret from **STDIN** or from a
  **file path argument** — NEVER from an argv flag value. Rationale: argv is
  visible in `ps` output to other users/processes and lands in shell
  history; a secret passed as `--nsec <value>` leaks both ways.
- Parses via `ImportSecret::parse` (trims whitespace, accepts upper/lower
  hex, rejects anything else with an error that never echoes the input).
- **Refuses to overwrite** an existing identity (`Error::AlreadyExists`);
  the existing file is left untouched. There is no `--force`; the user
  moves the old file aside by hand if they mean it.
- On success prints the imported **npub** and the identity file **location**.
  Never prints the secret back.

## Tool-specific auth subcommands

Tools may add their own verbs under `auth` alongside the shared pair —
fsite's `auth register` and `auth git` are the model — as long as `status`
and `import` behave as above and any verb that needs the key goes through
`finite-identity` rather than its own storage.
