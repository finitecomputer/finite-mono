//! Reversible storage residency. All operations use the existing Runtime lock,
//! exact Core leases and immutable Borg receipt; no archive is pruned here.
use super::*;
use crate::retirement::restore_recovery_zip;
use finite_saas_core::TrialArchiveSnapshot;

fn invalid(message: &str) -> RunnerError {
    RunnerError::RuntimeLaunch(message.into())
}

impl KataLauncher {
    fn require_encrypted_archive(&self, cwd: &Path) -> Result<(), RunnerError> {
        let config = self.retirement_config()?;
        let remote = url::Url::parse(&config.repository)
            .map_err(|_| invalid("Trial archives require an off-host SSH repository"))?;
        let host = remote.host_str().unwrap_or_default();
        if remote.scheme() != "ssh"
            || host.is_empty()
            || host == "localhost"
            || host == self.config.source_host_id
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
        {
            return Err(invalid("Trial archives require an off-host SSH repository"));
        }
        let command = self.borg_command(
            cwd,
            vec![
                "info".into(),
                "--json".into(),
                "--remote-path".into(),
                config.remote_path.as_os_str().into(),
                config.repository.clone().into(),
            ],
        )?;
        let output = self.execute(&command, self.config.command_timeout)?;
        if !output.status.success() {
            return Err(invalid("Archive encryption could not be verified"));
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| invalid("Invalid Borg repository metadata"))?;
        let mode = value
            .pointer("/encryption/mode")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !mode.starts_with("repokey") && !mode.starts_with("keyfile") {
            return Err(invalid(
                "Trial archives require an encrypted Borg repository",
            ));
        }
        Ok(())
    }

    pub(super) fn archive_trial_state(
        &mut self,
        lease: &RuntimeControlLease,
        renew: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<TrialArchiveSnapshot, RunnerError> {
        let principal = lease
            .archive_principal
            .clone()
            .ok_or_else(|| invalid("Core has not pinned the Agent Principal"))?;
        let plan = self.plan_for_control(lease)?;
        self.require_encrypted_archive(&self.config.work_root)?;
        let before = durable_state_manifest_sha256(&plan.state_root)?;
        // ArchiveTrial stops after verified remote readback, before removal.
        let receipt = self.retire_runtime(lease, renew)?;
        if durable_state_manifest_sha256(&plan.state_root)? != before {
            return Err(invalid("Archived source changed"));
        }
        self.cleanup_retirement_staging(&lease.request.id)?;
        Ok(TrialArchiveSnapshot {
            receipt,
            durable_state_manifest_sha256: before,
            agent_principal: principal,
        })
    }

    fn read_trial_archive(
        &self,
        snapshot: &TrialArchiveSnapshot,
        spec: &finite_saas_core::RuntimeSpecV1,
        renew: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<(PathBuf, VerifiedRecoveryZip), RunnerError> {
        let receipt = &snapshot.receipt;
        let config = self.retirement_config()?;
        if receipt.agent_runtime_id != spec.agent_runtime_id
            || receipt.project_id != spec.project_id
            || receipt.durable_state_id != spec.durable_state_id
            || receipt.runtime_artifact_id != spec.runtime_artifact_id
            || receipt.recovery_authority_id != config.recovery_authority_id
        {
            return Err(invalid(
                "Archive recovery authority or Runtime binding differs",
            ));
        }
        let context = RecoveryArtifactContext {
            request_id: receipt.request_id.clone(),
            project_id: spec.project_id.clone(),
            agent_runtime_id: spec.agent_runtime_id.clone(),
            durable_state_id: spec.durable_state_id.clone(),
            runtime_artifact_id: spec.runtime_artifact_id.clone(),
            runtime_image_digest: spec.runtime_image_digest.clone(),
            agent_principal: Some(snapshot.agent_principal.clone()),
        };
        let (root, zip, readback) = self.retirement_staging_paths(&receipt.request_id)?;
        self.require_encrypted_archive(&self.config.work_root)?;
        let verified = self.readback_retirement_archive(&root, &zip, &readback, &context, renew)?;
        if verified.zip_sha256 != receipt.zip_sha256
            || verified.zip_bytes != receipt.zip_bytes
            || verified.manifest_sha256 != receipt.manifest_sha256
        {
            return Err(invalid("Off-host archive no longer matches Core receipt"));
        }
        Ok((
            readback.join(
                zip.file_name()
                    .ok_or_else(|| invalid("Invalid archive filename"))?,
            ),
            verified,
        ))
    }

    pub(super) fn reclaim_trial_state(
        &mut self,
        lease: &RuntimeControlLease,
        renew: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError> {
        let snapshot = lease
            .trial_archive
            .as_ref()
            .ok_or_else(|| invalid("Reclaim requires a committed Core archive receipt"))?;
        let spec = control_runtime_spec(lease, RunnerClass::Kata)?
            .ok_or_else(|| invalid("Reclaim requires RuntimeSpec"))?;
        let plan = self.plan_for_control(lease)?;
        let _lock = self.acquire_runtime_operation_lock(&plan)?;
        let (_, verified) = self.read_trial_archive(snapshot, spec, renew)?;
        renew()?;
        let reservation_root = self.config.work_root.join("trial-reclaim-reservations");
        let reservation = reservation_root.join(&snapshot.receipt.request_id);
        persist_exact(
            &reservation,
            &serde_json::to_vec(snapshot).map_err(|_| invalid("Cannot encode archive receipt"))?,
        )?;
        if let Some(inspected) = self.inspect(&plan.container_name)? {
            self.validate_owned(&plan, &lease.runtime.project_id, &inspected)?;
            if inspected.state.status == "running" {
                return Err(invalid("Reclaim refuses running compute"));
            }
            self.wait_for_task_absence(&inspected.id, renew)?;
            self.remove_compute(&plan.container_name)?;
            self.wait_for_task_absence(&inspected.id, renew)?;
        }
        if self.inspect(&plan.container_name)?.is_some()
            || self
                .container_binding_durable_root(&plan.state_root)?
                .is_some()
        {
            return Err(invalid("Reclaim compute removal has not converged"));
        }
        let trash = plan
            .state_root
            .with_file_name(format!("trial-reclaim-{}", snapshot.receipt.request_id));
        if plan.state_root.symlink_metadata().is_ok() {
            if trash.symlink_metadata().is_ok() {
                return Err(invalid("Ambiguous reclaim directories"));
            }
            if source_manifest(&plan.state_root)
                .map_err(|_| invalid("Cannot verify reclaim source"))?
                != verified.manifest.entries
                || durable_state_manifest_sha256(&plan.state_root)?
                    != snapshot.durable_state_manifest_sha256
            {
                return Err(invalid(
                    "Reclaim source differs from verified recovery archive",
                ));
            }
            match durable_tree_is_quiescent_within(
                &plan.state_root,
                self.config.durable_tree_quiescence_window,
            )? {
                Quiescence::Quiet => {}
                _ => return Err(invalid("Reclaim source still has writers")),
            }
            renew()?;
            std::fs::rename(&plan.state_root, &trash)
                .map_err(|_| invalid("Cannot stage verified local reclamation"))?;
            sync_directory(
                plan.state_root
                    .parent()
                    .ok_or_else(|| invalid("Invalid durable root"))?,
            )?;
        }
        if trash.symlink_metadata().is_ok() {
            if !trash
                .symlink_metadata()
                .map_err(|_| invalid("Cannot inspect reclaim directory"))?
                .is_dir()
            {
                return Err(invalid("Reclaim path must be a directory"));
            }
            renew()?;
            std::fs::remove_dir_all(&trash)
                .map_err(|_| invalid("Local reclamation incomplete; archive retained"))?;
        }
        self.cleanup_retirement_staging(&snapshot.receipt.request_id)?;
        sync_directory(
            plan.state_root
                .parent()
                .ok_or_else(|| invalid("Invalid durable root"))?,
        )?;
        std::fs::remove_file(reservation)
            .map_err(|_| invalid("Cannot release reclaim capacity reservation"))?;
        sync_directory(&reservation_root)?;
        Ok(())
    }

    pub(super) fn restore_trial_state(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
        renew: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError> {
        let Some(snapshot) = lease
            .request
            .relocation
            .as_ref()
            .and_then(|r| r.v1().trial_archive.as_ref())
        else {
            return Ok(());
        };
        let spec = creation_runtime_spec(lease, RunnerClass::Kata)?
            .ok_or_else(|| invalid("Restore requires RuntimeSpec"))?;
        if self.inspect(&plan.container_name)?.is_none()
            && self
                .runner_capacity()
                .agent_creation_rejection_reason()
                .is_some()
        {
            return Err(invalid(
                "Restore target capacity is unavailable; retry on the pinned host",
            ));
        }
        if self.trial_restore_journal_matches(lease, plan)? {
            self.validate_trial_restore_target(lease, plan)?;
            self.cleanup_retirement_staging(&snapshot.receipt.request_id)?;
            return Ok(());
        }
        if self.inspect(&plan.container_name)?.is_some() {
            return Err(invalid("Restore target compute must be absent"));
        }
        if self
            .container_binding_durable_root(&plan.state_root)?
            .is_some()
        {
            return Err(invalid("Restore target directory is bound to compute"));
        }
        let (zip, verified) = self.read_trial_archive(snapshot, spec, renew)?;
        if plan.state_root.symlink_metadata().is_ok() {
            // A prior attempt may have atomically promoted the fully restored
            // tree. It is reusable only before any compute has changed it.
            if durable_state_manifest_sha256(&plan.state_root)?
                != snapshot.durable_state_manifest_sha256
            {
                return Err(invalid(
                    "Existing restore target differs; preserve it for recovery",
                ));
            }
            self.commit_trial_restore_journal(lease, plan)?;
            self.cleanup_retirement_staging(&snapshot.receipt.request_id)?;
            return Ok(());
        }
        let parent = plan
            .state_root
            .parent()
            .ok_or_else(|| invalid("Invalid durable root"))?;
        std::fs::create_dir_all(parent).map_err(|_| invalid("Cannot create restore parent"))?;
        if fs4::available_space(parent).map_err(|_| invalid("Cannot check restore capacity"))?
            < verified
                .manifest
                .total_file_bytes
                .saturating_add(64 * 1024 * 1024)
        {
            return Err(invalid(
                "Insufficient restore disk capacity; archive retained",
            ));
        }
        let staging = parent.join(format!("trial-restore-{}", lease.request.id));
        if staging.symlink_metadata().is_ok() {
            if !staging
                .symlink_metadata()
                .map_err(|_| invalid("Cannot inspect restore staging"))?
                .is_dir()
            {
                return Err(invalid("Restore staging must be a directory"));
            }
            std::fs::remove_dir_all(&staging)
                .map_err(|_| invalid("Cannot reset this operation's incomplete restore"))?;
        }
        renew()?;
        restore_recovery_zip(&zip, &staging, None)
            .map_err(|_| invalid("Recovery archive extraction failed; source archive retained"))?;
        if durable_state_manifest_sha256(&staging)? != snapshot.durable_state_manifest_sha256 {
            return Err(invalid("Restored durable tree does not match archive"));
        }
        if !staged_agent_identity_metadata(&staging)?.is_file() {
            return Err(invalid("Restored identity is not a regular file"));
        }
        renew()?;
        std::fs::rename(&staging, &plan.state_root)
            .map_err(|_| invalid("Cannot promote verified restore"))?;
        sync_directory(parent)?;
        self.commit_trial_restore_journal(lease, plan)?;
        self.cleanup_retirement_staging(&snapshot.receipt.request_id)?;
        Ok(())
    }
}

// Only host-owned metadata authorizes reuse of a restored, subsequently evolved
// tree. The immutable journal is durable before any guest can boot.
fn sync_directory(path: &Path) -> Result<(), RunnerError> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| invalid("Cannot persist trial storage boundary"))
}
fn persist_exact(path: &Path, bytes: &[u8]) -> Result<(), RunnerError> {
    if let Ok(metadata) = path.symlink_metadata() {
        if !metadata.is_file()
            || std::fs::read(path).map_err(|_| invalid("Cannot read storage journal"))? != bytes
        {
            return Err(invalid("Storage journal binding differs"));
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Invalid storage journal path"))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| invalid("Cannot create storage journal directory"))?;
    let temporary = path.with_extension("pending");
    write_secret_file(&temporary, bytes).map_err(|_| invalid("Cannot write storage journal"))?;
    std::fs::File::open(&temporary)
        .and_then(|f| f.sync_all())
        .map_err(|_| invalid("Cannot persist storage journal"))?;
    std::fs::rename(temporary, path).map_err(|_| invalid("Cannot commit storage journal"))?;
    sync_directory(parent)
}
impl KataLauncher {
    fn trial_restore_journal(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
    ) -> Result<(PathBuf, Vec<u8>), RunnerError> {
        let relocation = lease
            .request
            .relocation
            .as_ref()
            .ok_or_else(|| invalid("Missing restore binding"))?;
        if relocation.v1().trial_archive.is_none()
            || relocation.v1().target_source_host_id != self.config.source_host_id
        {
            return Err(invalid("Invalid trial restore target"));
        }
        let bytes=serde_json::to_vec(&serde_json::json!({"schema":1,"request":lease.request.id,"runtimeSpec":lease.request.runtime_spec,"relocation":relocation,"targetHost":self.config.source_host_id,"durableRoot":plan.state_root})).map_err(|_|invalid("Cannot encode restore binding"))?;
        Ok((
            plan.metadata_root
                .join(format!("trial-restore-{}.json", lease.request.id)),
            bytes,
        ))
    }
    pub(super) fn trial_restore_journal_matches(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
    ) -> Result<bool, RunnerError> {
        let (path, bytes) = self.trial_restore_journal(lease, plan)?;
        match path.symlink_metadata() {
            Ok(metadata) if metadata.is_file() => {
                if std::fs::read(path).map_err(|_| invalid("Cannot read restore journal"))? == bytes
                {
                    Ok(true)
                } else {
                    Err(invalid("Restore journal binding differs"))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            _ => Err(invalid("Invalid restore journal")),
        }
    }
    fn commit_trial_restore_journal(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
    ) -> Result<(), RunnerError> {
        let (path, bytes) = self.trial_restore_journal(lease, plan)?;
        persist_exact(&path, &bytes)
    }
    pub(super) fn validate_trial_restore_target(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
    ) -> Result<(), RunnerError> {
        if !plan
            .state_root
            .symlink_metadata()
            .map_err(|_| invalid("Restored state is missing"))?
            .is_dir()
            || !staged_agent_identity_metadata(&plan.state_root)?.is_file()
        {
            return Err(invalid("Invalid restored state or identity"));
        }
        if let Some((name, inspected)) = self.container_binding_durable_root(&plan.state_root)? {
            if name != plan.container_name {
                return Err(invalid("A different container binds restored state"));
            }
            self.validate_owned(plan, &lease.project.id, &inspected)?;
        } else if !matches!(
            durable_tree_is_quiescent_within(
                &plan.state_root,
                self.config.durable_tree_quiescence_window
            )?,
            Quiescence::Quiet
        ) {
            return Err(invalid("Restored state has an unexpected writer"));
        }
        Ok(())
    }
    pub(super) fn pause_trial_restore_state(
        &self,
        lease: &AgentCreationLease,
        plan: &KataLaunchPlan,
        renew: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError> {
        let Some(inspected) = self.inspect(&plan.container_name)? else {
            return Ok(());
        };
        if !self.trial_restore_journal_matches(lease, plan)? {
            return Err(invalid("Cannot pause target without restore journal"));
        }
        self.validate_owned(plan, &lease.project.id, &inspected)?;
        renew()?;
        if inspected.state.status == "running" {
            self.stop_compute(&plan.container_name)?;
        }
        Ok(())
    }
}

/// Conservatively double-count while compute still exists. Interrupted cleanup
/// cannot advertise capacity until both compute and local bytes are gone.
pub(super) fn reclaim_reservation_count(config: &KataConfig) -> Option<u32> {
    match std::fs::read_dir(config.work_root.join("trial-reclaim-reservations")) {
        Ok(entries) => {
            let mut count = 0u32;
            for entry in entries {
                let entry = entry.ok()?;
                if entry.file_type().ok()?.is_file() {
                    count = count.checked_add(1)?;
                } else {
                    return None;
                }
            }
            Some(count)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}
