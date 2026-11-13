/// Holds host admission through credential provisioning and launch. This is
/// neither a Core lease nor a physical-memory reservation.
#[derive(Default)]
pub struct CreationAdmissionGuard {
    _host_lock: Option<std::fs::File>,
}

impl CreationAdmissionGuard {
    pub(crate) fn with_host_lock(file: std::fs::File) -> Self {
        Self {
            _host_lock: Some(file),
        }
    }
}
