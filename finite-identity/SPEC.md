# Finite Identity Contract v1

Status: active contract. Changes require a version bump and sign-off from the
Finite team (Paul, Austin, Alex).

Launch constraint: v1's single local secret is not a complete Recoverability
Contract. No durable SaaS feature may rely on it as the only Recovery Authority;
the release must either restore the same key from tested recovery material or
provide an explicit identity-and-product-data migration path.

## Problem

Every Finite tool (`finitechat`, `fsite`, `fbrain`, hosted agent runtimes)
needs its identity owner's Nostr key. Today each tool can mint and store its own
key in its own location. This contract makes install-order symmetric within one
Finite Home: whichever Finite tool runs first mints the key; every other tool
in that same home finds it. A human Finite Chat home and an Agent Runtime home
are separate and never converge on one secret.

## Non-goals (deliberate, progressive)

- v1 ships a single locally-stored secret. Frostr-based backup arrives as a
  new `kind` in the same file (contract v2), and key rotation arrives on top
  of Frostr. Nothing in v1 may foreclose that path, but "future v2" is not a
  waiver for the first-slice recovery gate.
- No OS keychain in v1: signing happens in hot loops (finitechat) and must be
  non-interactive; ad-hoc-signed CLI binaries cannot use the macOS keychain
  without prompts. A keychain-backed storage backend may become an optional
  `kind` later.
- No per-service derived keys in v1: all tools use the account key directly.
  The file format reserves room for HKDF domain-separated derivation later.

## Location: convention over configuration

The identity root is:

```
$FINITE_HOME/identity/      if FINITE_HOME is set
$HOME/.finite/identity/     otherwise
```

Rules:

- `FINITE_HOME` is the ONLY override, intended for hosted runtimes and tests
  (e.g. `FINITE_HOME=/data/agent` puts identity on the durable mount). Tools
  MUST NOT add per-tool flags or envs for the identity location.
- `$HOME` resolution follows the platform (`dirs::home_dir()`); the relative
  layout under it is identical on macOS, Linux, and in Docker.
- Directories are created `0700`, the identity file `0600` (Unix). On
  non-Unix, creation fails closed unless `FINITE_IDENTITY_ALLOW_INSECURE=1`.

## File: `identity.json`

```json
{
  "version": 1,
  "kind": "nostr-secp256k1",
  "secret_hex": "<64 lowercase hex chars>",
  "public_key_hex": "<64 lowercase hex chars, x-only>",
  "created_at": "<RFC3339>",
  "created_by": "<tool name/version that minted or imported it>"
}
```

- `version` gates parsing: readers MUST refuse versions they do not know.
- `kind` gates key material interpretation: v1 readers MUST refuse unknown
  kinds (this is where `frostr-share` etc. arrive later).
- `public_key_hex` is derived, stored for cheap discovery, and MUST be
  verified against `secret_hex` on load (fail closed on mismatch).
- Unknown extra fields MUST be preserved on rewrite and ignored on read.

## Concurrency: mint-if-absent must be safe

Two tools starting concurrently on a fresh machine must converge on one key.
Writers take an exclusive advisory lock on `<identity-root>/.lock` around the
check-and-mint, write to a temp file in the same directory, fsync, and rename
into place. Readers that find no file may mint (same lock) via
`load_or_generate`; `load` never mints.

## Import (still contract v1)

The identity file may be created by `FiniteIdentity::import` (adopting a
user-supplied secret, e.g. an existing Nostr key) as well as by
`load_or_generate`. Import writes the same v1 format under the same lock and
atomic-write rules, and records `created_by`/`created_at` for the importing
tool exactly like minting does. Import REFUSES to overwrite an existing
identity file — even one this build cannot parse — with a distinct
`AlreadyExists` error. Racing `import` against `load_or_generate` therefore
has exactly one winner: if generation takes the lock first, import fails
with `AlreadyExists`; if import wins, `load_or_generate` adopts the imported
key. CLI-facing conventions for import live in
[CLI-CONVENTIONS.md](./CLI-CONVENTIONS.md).

## Runtime behavior

- The key is loaded into memory once and held; signing operations MUST NOT
  re-read the file. (finitechat signs in hot loops.)
- Tools display identity as `npub` (bech32) but store hex.
- Tools MUST NOT copy the secret into their own config stores; derive what
  they need at runtime.

## Hard cut

There are no migration shims. Legacy locations
(`~/.config/finite-sites/identity.env`, fbrain `auth.json`,
`FINITECHAT_HOME` account secrets) are not read. A user who wants to keep a
legacy key moves it by hand (each tool's release notes show the one-liner);
otherwise a fresh identity is minted at first run.

## Owner

This contract and the `finite-identity` crate own the shared Finite key and
Principal Resolution boundary. Future key kinds, backup, and rotation require
new Finite Identity contract versions; they do not depend on the retired
`finite-auth` experiment or imply a shared human-agent signer.
