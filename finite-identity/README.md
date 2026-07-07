# finite-identity

Shared on-disk Nostr identity for Finite tools. Every Finite tool
(`finitechat`, `fsite`, `fbrain`, hosted agent runtimes) needs the user's
Nostr key; this crate makes identity install-order symmetric: whichever tool
runs first mints the key, and every other tool finds it. See
[SPEC.md](./SPEC.md) for the full contract (v1) and
[CLI-CONVENTIONS.md](./CLI-CONVENTIONS.md) for the `auth status` /
`auth import` verbs every Finite CLI exposes on top of this crate.

## Convention over configuration

There are no per-tool flags or environment variables for the identity
location. The identity root is `$FINITE_HOME/identity/` when `FINITE_HOME` is
set (hosted runtimes, tests), and `$HOME/.finite/identity/` otherwise.
Directories are created `0700` and the identity file `0600` on Unix; on
non-Unix, creation fails closed unless `FINITE_IDENTITY_ALLOW_INSECURE=1`.

| Path | Purpose |
| --- | --- |
| `$FINITE_HOME/identity/` | Identity root when `FINITE_HOME` is set |
| `$HOME/.finite/identity/` | Identity root otherwise |
| `<root>/identity.json` | The identity file (version-gated JSON) |
| `<root>/.lock` | Advisory lock taken around mint-if-absent |

## Install

Pin to a specific revision in your `Cargo.toml`:

```toml
[dependencies]
finite-identity = { git = "https://github.com/finitecomputer/finite-identity", rev = "<commit-sha>" }
```

## Usage

```rust
use finite_identity::{FiniteIdentity, IdentityPaths};

fn main() -> Result<(), finite_identity::Error> {
    let paths = IdentityPaths::resolve()?;

    // Mints under an exclusive lock if no identity exists yet;
    // use FiniteIdentity::load(&paths) if minting is not acceptable.
    let identity = FiniteIdentity::load_or_generate(&paths, "mytool/1.0.0")?;

    println!("hex:  {}", identity.public_key_hex());
    println!("npub: {}", identity.npub());

    // BIP340 Schnorr, deterministic (no aux rand), zero file reads per sign.
    let digest = [0u8; 32];
    let signature: [u8; 64] = identity.sign_schnorr(&digest);

    // For callers that must derive from the account key at runtime
    // (e.g. HKDF). Never copy this into your own config store.
    let _secret: [u8; 32] = identity.expose_secret_bytes();

    let _ = signature;
    Ok(())
}
```

### Importing an existing secret

For `auth import` (see [CLI-CONVENTIONS.md](./CLI-CONVENTIONS.md)): parse an
`nsec1...` or 64-hex string read from stdin or a file — never from an argv
flag — and adopt it under the same lock and atomic-write rules as minting.
Import refuses to overwrite an existing identity (`Error::AlreadyExists`).

```rust,no_run
use finite_identity::{FiniteIdentity, IdentityPaths, ImportSecret};

fn import(input: &str) -> Result<(), finite_identity::Error> {
    let paths = IdentityPaths::resolve()?;
    let secret = ImportSecret::parse(input)?; // nsec1... or 64 hex chars
    let identity = FiniteIdentity::import(&paths, secret, "mytool/1.0.0")?;
    println!("imported {} -> {}", identity.npub(), paths.identity_file().display());
    Ok(())
}
```

## Roadmap

v1 is a single locally-stored key; Frostr-based backup arrives as a new
`kind` in the same file (contract v2), and key rotation arrives on top of
Frostr — nothing in v1 forecloses that path.
