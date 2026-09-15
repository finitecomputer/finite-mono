use std::cell::RefCell;
use std::path::PathBuf;

use super::*;

thread_local! {
    static FAILED_SYNC_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(super) fn before_parent_sync(path: &Path) -> std::io::Result<()> {
    if FAILED_SYNC_PATH.with(|failed| failed.borrow().as_deref() == Some(path)) {
        return Err(std::io::Error::other("injected parent sync failure"));
    }
    Ok(())
}

struct FailedParentSync;

impl FailedParentSync {
    fn at(path: PathBuf) -> Self {
        FAILED_SYNC_PATH.with(|failed| {
            assert!(failed.replace(Some(path)).is_none());
        });
        Self
    }
}

impl Drop for FailedParentSync {
    fn drop(&mut self) {
        FAILED_SYNC_PATH.with(|failed| failed.replace(None));
    }
}

#[test]
fn capture_retry_cannot_acknowledge_an_unsynced_reused_object() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let data = root_path.join("source");
    fs::create_dir(&data).unwrap();
    drop(finitesites_store::Store::open(&data.join("registry.db")).unwrap());
    let cookie = hex::encode(&[9; 32]); // Public synthetic signing key.
    fs::write(data.join("cookie-secret"), &cookie).unwrap();
    let repository = root_path.join("backup");
    // The cookie is the last new object in this empty-registry capture.
    let object = repository.join("objects").join(digest(cookie.as_bytes()));
    let fault = FailedParentSync::at(object.clone());

    let first = capture(&data, &repository, 200).unwrap_err();
    assert!(first.to_string().contains("injected parent sync failure"));
    assert_eq!(fs::read(&object).unwrap(), cookie.as_bytes());
    assert_eq!(fs::read_dir(repository.join("points")).unwrap().count(), 0);

    let retry = capture(&data, &repository, 200);
    assert!(
        retry.is_err(),
        "reused objects still require durable directory entries"
    );
    assert!(
        retry
            .unwrap_err()
            .to_string()
            .contains("injected parent sync failure")
    );
    assert_eq!(fs::read_dir(repository.join("points")).unwrap().count(), 0);

    drop(fault);
    let receipt = capture(&data, &repository, 200).unwrap();
    assert_eq!(receipt.new_objects, 0);
    let target = root.path().join("restored");
    restore(&repository, &receipt.id, &target).unwrap();
    assert_eq!(
        fs::read(target.join("cookie-secret")).unwrap(),
        cookie.as_bytes()
    );
}
