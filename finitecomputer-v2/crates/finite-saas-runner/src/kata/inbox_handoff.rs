use super::*;
use finitechat_hermes::inbox_compatibility::{self, READER_LABEL};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

pub(super) fn check_chat_home(inspected: &KataInspect) -> Result<(), RunnerError> {
    if inspected
        .config
        .environment
        .iter()
        .filter_map(|entry| entry.strip_prefix("FINITECHAT_HOME="))
        .any(|home| home != "/data/agent")
    {
        return Err(RunnerError::RuntimeLaunch("chat inbox compatibility requires canonical FINITECHAT_HOME=/data/agent; runtime left intact".into()));
    }
    Ok(())
}

/// Operator bound on the root Runner's read of a guest-written inbox. The
/// sidecar bounds the acked ring (4,096 keys) and pending refusals (32, each
/// prepared reply at most 256 KiB) but not the ordinary event backlog, so
/// this does not cover every valid inbox. A larger inbox fails closed with a
/// fixed message and is not modified; the fleet preflight owns that edge.
const MAX_INBOX_BYTES: u64 = 64 * 1024 * 1024;

fn refused(reason: &str) -> RunnerError {
    RunnerError::RuntimeLaunch(format!(
        "chat inbox compatibility not established: {reason}; inbox not modified"
    ))
}

/// Reads `<state_root>/agent/hermes-inbox.json`, the canonical Agent Runtime's
/// durable Chat home (`/data/agent`). `state_root` is a host-owned path and may
/// resolve through host symlinks; the guest controls everything below it, so
/// `agent` and the inbox are opened relative to anchored descriptors without
/// following symlinks, and only a regular file is read, up to the bound.
fn inbox(state_root: &Path) -> Result<Option<Vec<u8>>, RunnerError> {
    let absent_or = |error: Errno, reason| match error {
        Errno::NOENT => Ok(None),
        _ => Err(refused(reason)),
    };
    let directory = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let root = match rustix::fs::open(state_root, directory, Mode::empty()) {
        Ok(root) => root,
        Err(error) => return absent_or(error, "the state root is unreadable"),
    };
    let agent =
        match rustix::fs::openat(&root, "agent", directory | OFlags::NOFOLLOW, Mode::empty()) {
            Ok(agent) => agent,
            Err(error) => return absent_or(error, "the chat home is not a directory"),
        };
    // Reject special files before opening them; the opened descriptor below
    // is still the authority if the entry is swapped in between.
    match rustix::fs::statat(&agent, "hermes-inbox.json", AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode).is_file() => {}
        Ok(_) => return Err(refused("the inbox is not a regular file")),
        Err(error) => return absent_or(error, "the inbox is unreadable"),
    }
    let file = match rustix::fs::openat(
        &agent,
        "hermes-inbox.json",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => std::fs::File::from(file),
        Err(error) => return absent_or(error, "the inbox is not a regular file"),
    };
    let metadata = file
        .metadata()
        .map_err(|_| refused("the inbox is unreadable"))?;
    if !metadata.file_type().is_file() {
        return Err(refused("the inbox is not a regular file"));
    }
    let oversized = || refused("the inbox exceeds the Runner's 64 MiB handoff bound");
    if metadata.len() > MAX_INBOX_BYTES {
        return Err(oversized());
    }
    // The file may still grow after fstat; the read bound is authoritative.
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INBOX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| refused("the inbox is unreadable"))?;
    if bytes.len() as u64 > MAX_INBOX_BYTES {
        return Err(oversized());
    }
    Ok(Some(bytes))
}

impl KataLauncher {
    pub(super) fn check_retained_chat_reader(
        &self,
        plan: &KataLaunchPlan,
    ) -> Result<(), RunnerError> {
        match self.inspect(&plan.container_name)? {
            Some(retained) => self.check_container_chat_reader(&retained),
            None => match inbox(&plan.state_root)? {
                Some(bytes) => {
                    inbox_compatibility::check(&bytes, None).map_err(RunnerError::RuntimeLaunch)
                }
                None => Ok(()),
            },
        }
    }
    pub(super) fn check_container_chat_reader(
        &self,
        inspected: &KataInspect,
    ) -> Result<(), RunnerError> {
        check_chat_home(inspected)?;
        for mount in &inspected.mounts {
            if mount.destination == Path::new("/data")
                && let Some(bytes) = inbox(&mount.source)?
            {
                inbox_compatibility::check(
                    &bytes,
                    inspected
                        .config
                        .labels
                        .get(READER_LABEL)
                        .map(String::as_str),
                )
                .map_err(RunnerError::RuntimeLaunch)?;
            }
        }
        Ok(())
    }

    pub(super) fn check_target_chat_reader(
        &self,
        state_root: &Path,
        image: &str,
    ) -> Result<(), RunnerError> {
        let Some(bytes) = inbox(state_root)? else {
            return Ok(());
        };
        if inbox_compatibility::check(&bytes, None).is_ok() {
            return Ok(());
        }
        // The target is already pulled and bound to an immutable digest by the
        // upgrade contract. Inspect its image label, not a mutable side file.
        let output = self.run_checked(
            self.command(vec![
                "image".into(),
                "inspect".into(),
                "--format".into(),
                "{{json .Config.Labels}}".into(),
                image.into(),
            ]),
            self.config.command_timeout,
        )?;
        let labels: Option<BTreeMap<String, String>> = serde_json::from_str(output.trim())
            .map_err(|error| {
                RunnerError::RuntimeLaunch(format!("invalid target image labels: {error}"))
            })?;
        inbox_compatibility::check(
            &bytes,
            labels
                .as_ref()
                .and_then(|labels| labels.get(READER_LABEL))
                .map(String::as_str),
        )
        .map_err(RunnerError::RuntimeLaunch)
    }
}

#[cfg(test)]
mod unsafe_reads;
