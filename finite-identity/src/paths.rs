use std::env;
use std::path::{Path, PathBuf};

use crate::Error;

/// The only environment override for the identity location, intended for
/// hosted runtimes and tests. Tools MUST NOT add per-tool location flags.
pub const FINITE_HOME_ENV: &str = "FINITE_HOME";

/// File name of the identity file inside the identity root.
pub const IDENTITY_FILE_NAME: &str = "identity.json";

/// File name of the advisory lock file inside the identity root.
pub const LOCK_FILE_NAME: &str = ".lock";

/// Resolved location of the identity root directory.
///
/// Per the contract this is `$FINITE_HOME/identity/` when `FINITE_HOME` is
/// set, and `$HOME/.finite/identity/` otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityPaths {
    root: PathBuf,
}

impl IdentityPaths {
    /// Resolve the identity root from the environment.
    ///
    /// Uses `$FINITE_HOME/identity/` if `FINITE_HOME` is set and non-empty,
    /// otherwise `$HOME/.finite/identity/` via the platform home directory.
    pub fn resolve() -> Result<Self, Error> {
        match env::var_os(FINITE_HOME_ENV) {
            Some(finite_home) if !finite_home.is_empty() => Ok(Self::with_finite_home(finite_home)),
            _ => {
                let home = dirs::home_dir().ok_or(Error::NoHomeDir)?;
                Ok(Self::with_finite_home(home.join(".finite")))
            }
        }
    }

    /// Build paths from an explicit Finite home directory, ignoring the
    /// environment. `IdentityPaths::with_finite_home(dir)` is exactly
    /// equivalent to resolving with `FINITE_HOME=dir`: the identity root is
    /// `dir/identity/`. Intended for tests and embedders that manage their
    /// own configuration.
    pub fn with_finite_home(finite_home: impl Into<PathBuf>) -> Self {
        Self {
            root: finite_home.into().join("identity"),
        }
    }

    /// The identity root directory (`.../identity`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the identity file (`<root>/identity.json`).
    pub fn identity_file(&self) -> PathBuf {
        self.root.join(IDENTITY_FILE_NAME)
    }

    /// Path of the advisory lock file (`<root>/.lock`).
    pub fn lock_file(&self) -> PathBuf {
        self.root.join(LOCK_FILE_NAME)
    }
}
