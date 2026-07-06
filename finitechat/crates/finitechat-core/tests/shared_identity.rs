//! Shared Finite identity acquisition (Finite Identity Contract v1).
//!
//! When `OpenOptions.account_secret_hex` is `None`, the runtime resolves the
//! account key through `finite-identity`: `$FINITE_HOME/identity/identity.json`
//! is minted on first run and found by every later run (and by every other
//! Finite tool). These tests manipulate the process-global `FINITE_HOME`
//! environment variable, so they serialize on a mutex and restore it.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_core::{FiniteChatRuntime, OpenOptions};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Run `f` with `FINITE_HOME` pointing at `finite_home`, restoring the
/// previous value afterwards. Serialized: `FINITE_HOME` is process-global.
fn with_finite_home<T>(finite_home: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = env_lock().lock().unwrap();
    let previous = std::env::var_os("FINITE_HOME");
    // SAFETY: all tests that read or write FINITE_HOME hold `env_lock`.
    unsafe { std::env::set_var("FINITE_HOME", finite_home) };
    let result = f();
    // SAFETY: see above.
    unsafe {
        match previous {
            Some(value) => std::env::set_var("FINITE_HOME", value),
            None => std::env::remove_var("FINITE_HOME"),
        }
    }
    result
}

fn open_runtime(data_dir: &Path, device_id: &str) -> std::sync::Arc<FiniteChatRuntime> {
    FiniteChatRuntime::open(OpenOptions {
        data_dir: data_dir.display().to_string(),
        server_url: "http://127.0.0.1:1".to_owned(),
        device_id: device_id.to_owned(),
        account_secret_hex: None,
        now_unix_seconds: Some(1_800_000_000),
    })
    .expect("runtime opens against the shared identity")
}

#[test]
fn fresh_start_mints_shared_identity_at_finite_home() {
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    let data_dir = dir.path().join("store");

    let state = with_finite_home(&finite_home, || {
        open_runtime(&data_dir, "fresh-device").state().unwrap()
    });

    let identity_file = finite_home.join("identity").join("identity.json");
    assert!(
        identity_file.is_file(),
        "first run must mint {}",
        identity_file.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&identity_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The minted file and the runtime agree on the account.
    let minted = FiniteIdentity::load(&IdentityPaths::with_finite_home(&finite_home)).unwrap();
    assert_eq!(state.identity.account_id, minted.public_key_hex());
    assert_eq!(
        state.identity.account_secret_hex,
        hex::encode(minted.expose_secret_bytes())
    );
    assert!(minted.created_by().starts_with("finitechat "));

    // No copy of the secret lands in the tool's own store (hard cut of the
    // legacy account-secret.hex location).
    assert!(!data_dir.join("account-secret.hex").exists());
}

#[test]
fn preexisting_shared_identity_is_picked_up_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    let data_dir = dir.path().join("store");

    // Another Finite tool minted first (planted via the crate's own format).
    let planted = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(&finite_home),
        "other-finite-tool/9.9.9",
    )
    .unwrap();

    let state = with_finite_home(&finite_home, || {
        open_runtime(&data_dir, "adopting-device").state().unwrap()
    });

    // Same account key: same account id (the device credential's account id)
    // and the same npub display form.
    assert_eq!(state.identity.account_id, planted.public_key_hex());
    assert_eq!(
        finitechat_core::npub_from_account_id(state.identity.account_id.clone()).unwrap(),
        planted.npub()
    );

    // The file still names the original minter.
    let reloaded = FiniteIdentity::load(&IdentityPaths::with_finite_home(&finite_home)).unwrap();
    assert_eq!(reloaded.created_by(), "other-finite-tool/9.9.9");
}

#[test]
fn reopening_the_same_store_reuses_the_shared_identity() {
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    let data_dir = dir.path().join("store");

    let (first, second) = with_finite_home(&finite_home, || {
        let first = open_runtime(&data_dir, "device-a").state().unwrap();
        let second = open_runtime(&data_dir, "device-a").state().unwrap();
        (first, second)
    });

    assert_eq!(first.identity.account_id, second.identity.account_id);
    assert_eq!(first.identity.device_id, second.identity.device_id);
}
