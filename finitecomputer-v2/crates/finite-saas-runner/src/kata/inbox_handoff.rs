use super::*;
use finitechat_hermes::inbox_compatibility::{self, READER_LABEL};

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

fn inbox(state_root: &Path) -> Result<Option<Vec<u8>>, RunnerError> {
    // /data/agent is the canonical Agent Runtime's durable Chat home.
    match std::fs::read(state_root.join("agent/hermes-inbox.json")) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RunnerError::RuntimeLaunch(format!(
            "chat inbox handoff check failed: {error}"
        ))),
    }
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
