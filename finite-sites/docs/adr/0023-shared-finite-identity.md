# Shared Finite Identity Within One Finite Home

The Publishing Key is no longer minted or stored by `fsite` itself. It is the
shared Finite identity defined by the Finite Identity Contract v1
(https://github.com/finitecomputer/finite-identity): one Nostr key per Finite
Home, at `$FINITE_HOME/identity/identity.json` when `FINITE_HOME` is set and
`~/.finite/identity/identity.json` otherwise. Whichever Finite tool
(`finitechat`, `fsite`, `fbrain`, hosted runtimes) runs first in that home
mints the key under an exclusive lock; every other tool in the same home finds
it. This makes identity install-order symmetric across Finite tools instead of
one identity per tool. Human and agent homes remain distinct principals and do
not share an nsec.

## Decision

- `fsite` depends on the `finite-identity` crate, pinned by git rev, and
  loads the Publishing Key with `FiniteIdentity::load_or_generate` using
  `created_by = "fsite/<version>"`.
- The secret is derived into process memory per invocation
  (`expose_secret_bytes`) to sign NIP-98 requests and is never copied into
  fsite's own config store, per the contract.
- Hard cut: the legacy `~/.config/finite-sites/identity.env` location is not
  read and there is no fallback or per-tool location override
  (`FINITE_SITES_IDENTITY` is gone; `FINITE_HOME` is the only override).
- Per the finite-identity CLI conventions (CLI-CONVENTIONS.md in that repo),
  the shared verbs live under `auth`: `fsite auth status` shows the npub,
  file location, `created_by`, and `created_at` (`--output json` supported);
  `fsite auth import` adopts an existing `nsec1...` or 64-char hex secret
  exactly once and fails closed if a shared identity already exists, because
  replacing it would fork that principal's identity across tools. (These shipped
  briefly, unreleased, as `fsite identity [import]` and were renamed before
  release; there are no aliases.)
- `fsite auth import` reads the secret from stdin or `--file PATH`, never
  from a flag value, because argv leaks into `ps` output and shell history.
  A `--file` pointing at a legacy `identity.env` imports its
  `FINITE_SITES_USER_SECRET=hex` value; any other file content is treated as
  the secret string itself.
- Per-email keys (`~/.config/finite-sites/emails/*.env`) and pending
  email-link markers stay tool-specific and unchanged; only the primary
  Publishing Key moved.

## Consequences

- Users upgrading from the identity.env era either run the import once to
  keep their npub (and thus their Projects) or get a fresh identity on first
  run. There is deliberately no automatic migration shim.
- A key minted by another Finite tool is adopted as-is; `fsite whoami` and
  `fsite auth status` report whichever tool minted it via `created_by`.
- Identity file format, permissions (0700 dir / 0600 file), locking,
  version/kind gating, secret parsing (`ImportSecret::parse`), and import
  writing (`FiniteIdentity::import`) are owned by the `finite-identity`
  crate; fsite never writes the identity file itself, and after an import it
  round-trips the file through the crate loader to prove the contract
  accepts it.
