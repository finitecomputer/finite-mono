//! Shared on-disk Nostr identity for Finite tools.
//!
//! Implements the [Finite Identity Contract v1](https://github.com/finitecomputer/finite-identity/blob/main/SPEC.md):
//! every Finite tool (`finitechat`, `fsite`, `fbrain`, hosted runtimes) reads
//! the same `identity.json` under `$FINITE_HOME/identity/` (or
//! `~/.finite/identity/`), so whichever tool runs first mints the user's key
//! and every other tool finds it.
//!
//! ```no_run
//! use finite_identity::{FiniteIdentity, IdentityPaths};
//!
//! let paths = IdentityPaths::resolve()?;
//! let identity = FiniteIdentity::load_or_generate(&paths, "example-tool/0.1.0")?;
//! println!("{}", identity.npub());
//! let signature = identity.sign_schnorr(&[0u8; 32]);
//! # let _ = signature;
//! # Ok::<(), finite_identity::Error>(())
//! ```

mod error;
mod identity;
pub mod npub;
pub mod nsec;
mod paths;

pub use error::Error;
pub use identity::{
    ALLOW_INSECURE_ENV, FORMAT_VERSION, FiniteIdentity, ImportSecret, KIND_NOSTR_SECP256K1,
};
pub use paths::{FINITE_HOME_ENV, IDENTITY_FILE_NAME, IdentityPaths, LOCK_FILE_NAME};
