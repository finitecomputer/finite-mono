use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::AgentdError;
use crate::ledger::{
    Ledger, StartupSpecializationApplyIntent, StartupSpecializationPhase, hex_digest,
};

pub const VISION_CONFIG_PATH: &str = "auxiliary.vision";
pub const FINITECHAT_TOOLSETS_CONFIG_PATH: &str = "platform_toolsets.finitechat";
pub const MODEL_CONFIG_PATH: &str = "model";
pub const TELEGRAM_CONFIG_PATH: &str = "gateway.platforms.telegram";
pub const DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL: &str =
    "nemotron-3-nano-omni-30b-a3b-reasoning-nvfp4-fast";
pub const DEFAULT_MULTIMODAL_SPECIALIZATION_WORKER_URL: &str =
    match option_env!("FINITE_AGENTD_MULTIMODAL_WORKER_URL") {
        Some(value) => value,
        None => "https://specialization.finite.vip/v1",
    };
pub const DEFAULT_MULTIMODAL_SPECIALIZATION_BUNDLE: &str = "finite-private-multimodal-v1";
#[cfg(not(test))]
const CONFIG_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const CONFIG_LOCK_TIMEOUT: Duration = Duration::from_millis(500);
const CONFIG_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationCapabilitiesV1 {
    pub image: bool,
    pub audio: bool,
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationPromptVersionsV1 {
    pub image: String,
    pub audio: String,
    pub video: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationNormalizationLimitsV1 {
    pub max_images: u16,
    pub max_inline_bytes: u64,
    pub max_download_bytes: u64,
    pub max_output_chars: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultimodalSpecializationDesiredStateV1 {
    pub proposal_id: String,
    pub model_alias: String,
    pub worker_base_url: String,
    pub capabilities: SpecializationCapabilitiesV1,
    pub prompt_versions: SpecializationPromptVersionsV1,
    pub normalization_limits: SpecializationNormalizationLimitsV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_api_key: Option<String>,
}

impl std::fmt::Debug for MultimodalSpecializationDesiredStateV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MultimodalSpecializationDesiredStateV1")
            .field("proposal_id", &self.proposal_id)
            .field("model_alias", &self.model_alias)
            .field("worker_base_url", &self.worker_base_url)
            .field("capabilities", &self.capabilities)
            .field("prompt_versions", &self.prompt_versions)
            .field("normalization_limits", &self.normalization_limits)
            .field("worker_api_key", &"<redacted>")
            .finish()
    }
}

impl MultimodalSpecializationDesiredStateV1 {
    pub fn canonical(proposal_id: impl Into<String>) -> Self {
        Self {
            proposal_id: proposal_id.into(),
            model_alias: DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL.to_owned(),
            worker_base_url: DEFAULT_MULTIMODAL_SPECIALIZATION_WORKER_URL.to_owned(),
            capabilities: SpecializationCapabilitiesV1 {
                image: true,
                audio: true,
                video: true,
            },
            prompt_versions: SpecializationPromptVersionsV1 {
                image: "finite-multimodal-image-v1".to_owned(),
                audio: "finite-multimodal-audio-v1".to_owned(),
                video: "finite-multimodal-video-v1".to_owned(),
            },
            normalization_limits: SpecializationNormalizationLimitsV1 {
                max_images: 8,
                max_inline_bytes: 16 * 1024 * 1024,
                max_download_bytes: 32 * 1024 * 1024,
                max_output_chars: 32 * 1024,
            },
            worker_api_key: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationReconcileResultV1 {
    pub proposal_id: String,
    pub applied: bool,
    pub already_applied: bool,
    pub effective_matches_desired: bool,
    pub model_alias: String,
    pub worker_base_url: String,
    pub capabilities: SpecializationCapabilitiesV1,
    pub prompt_versions: SpecializationPromptVersionsV1,
    pub normalization_limits: SpecializationNormalizationLimitsV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOfferPolicyV1 {
    ApplyIfUnset,
    ReplaceWithConfirmation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesConfigOfferV1 {
    pub proposal_id: String,
    pub path: String,
    pub policy: ConfigOfferPolicyV1,
    #[serde(default)]
    pub approved: bool,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesConfigRollbackV1 {
    pub proposal_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigPreviewV1 {
    pub proposal_id: String,
    pub path: String,
    pub policy: ConfigOfferPolicyV1,
    pub current: Value,
    pub proposed: Value,
    pub ownership: String,
    pub would_apply: bool,
    pub requires_confirmation: bool,
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigApplyResultV1 {
    pub proposal_id: String,
    pub path: String,
    pub applied: bool,
    pub already_applied: bool,
    pub restart_required: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    path: PathBuf,
    ledger: Ledger,
    transaction_lock: std::sync::Arc<std::sync::Mutex<()>>,
    #[cfg(test)]
    fail_after_persist: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct ConfigTransactionGuard<'a> {
    _process_guard: std::sync::MutexGuard<'a, ()>,
    lock_file: File,
}

impl Drop for ConfigTransactionGuard<'_> {
    fn drop(&mut self) {
        let _ = fs4::fs_std::FileExt::unlock(&self.lock_file);
    }
}

#[derive(Default)]
struct FinitechatToolsetAdditions {
    video: bool,
}

impl ConfigManager {
    pub fn new(path: impl Into<PathBuf>, ledger: Ledger) -> Self {
        Self {
            path: path.into(),
            ledger,
            transaction_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            #[cfg(test)]
            fail_after_persist: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn current_value(&self, path: &str) -> Result<Value, AgentdError> {
        let (_bytes, document) = self.load_document()?;
        Ok(value_at_path(&document, path)
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub fn preview(&self, offer: &HermesConfigOfferV1) -> Result<ConfigPreviewV1, AgentdError> {
        validate_offer(offer)?;
        let (_bytes, document) = self.load_document()?;
        let current = value_at_path(&document, &offer.path)
            .cloned()
            .unwrap_or(Value::Null);
        let current_hash = value_hash(&current)?;
        let ownership = self.ledger.config_ownership(&offer.path)?;
        let owned_match = ownership
            .as_ref()
            .is_some_and(|record| record.applied_hash == current_hash);
        let unset = value_is_unset(&offer.path, &current);
        let requires_confirmation =
            offer.policy == ConfigOfferPolicyV1::ReplaceWithConfirmation && !offer.approved;
        let conflict = match offer.policy {
            ConfigOfferPolicyV1::ApplyIfUnset if !unset && !owned_match => Some(
                "Hermes or the user already owns this field; Finite will not overwrite it"
                    .to_owned(),
            ),
            ConfigOfferPolicyV1::ReplaceWithConfirmation if !offer.approved => {
                Some("explicit confirmation is required before replacement".to_owned())
            }
            _ => None,
        };
        Ok(ConfigPreviewV1 {
            proposal_id: offer.proposal_id.clone(),
            path: offer.path.clone(),
            policy: offer.policy,
            current: redact_value(&current),
            proposed: redact_value(&offer.value),
            ownership: if owned_match {
                "finite".to_owned()
            } else if unset {
                "unset".to_owned()
            } else {
                "custom".to_owned()
            },
            would_apply: conflict.is_none(),
            requires_confirmation,
            conflict,
        })
    }

    pub fn apply(
        &self,
        offer: &HermesConfigOfferV1,
        validate: impl FnOnce() -> Result<(), AgentdError>,
    ) -> Result<ConfigApplyResultV1, AgentdError> {
        let _guard = self.lock_transaction()?;
        let preview = self.preview(offer)?;
        if let Some(history) = self.ledger.config_history(&offer.proposal_id)? {
            if history.path != offer.path {
                return Err(AgentdError::ConfigConflict(
                    "proposal id was already used for another configuration path".to_owned(),
                ));
            }
            if history.rolled_back {
                return Err(AgentdError::ConfigConflict(
                    "rolled-back proposal ids cannot be reused".to_owned(),
                ));
            }
            let (_bytes, document) = self.load_document()?;
            let current = value_at_path(&document, &offer.path)
                .cloned()
                .unwrap_or(Value::Null);
            if value_hash(&current)? == history.applied_hash {
                return Ok(ConfigApplyResultV1 {
                    proposal_id: offer.proposal_id.clone(),
                    path: offer.path.clone(),
                    applied: false,
                    already_applied: true,
                    restart_required: false,
                });
            }
            return Err(AgentdError::ConfigConflict(
                "the configuration changed after this proposal was applied".to_owned(),
            ));
        }
        if let Some(conflict) = preview.conflict {
            return Err(AgentdError::ConfigConflict(conflict));
        }

        let (before_bytes, mut document) = self.load_document()?;
        set_value_at_path(&mut document, &offer.path, offer.value.clone())?;
        let rendered = serde_yaml::to_string(&document)?;
        self.checked_atomic_write(&before_bytes, rendered.as_bytes())?;
        if let Err(error) = validate() {
            self.checked_atomic_write(rendered.as_bytes(), &before_bytes)?;
            return Err(error);
        }
        let applied_hash = value_hash(&offer.value)?;
        if let Err(error) = self.ledger.record_config_apply(
            &offer.proposal_id,
            &offer.path,
            &before_bytes,
            &applied_hash,
        ) {
            self.checked_atomic_write(rendered.as_bytes(), &before_bytes)?;
            return Err(error);
        }
        Ok(ConfigApplyResultV1 {
            proposal_id: offer.proposal_id.clone(),
            path: offer.path.clone(),
            applied: true,
            already_applied: false,
            restart_required: true,
        })
    }

    pub fn rollback(
        &self,
        request: &HermesConfigRollbackV1,
        validate: impl FnOnce() -> Result<(), AgentdError>,
    ) -> Result<ConfigApplyResultV1, AgentdError> {
        let _guard = self.lock_transaction()?;
        let history = self
            .ledger
            .config_history(&request.proposal_id)?
            .ok_or_else(|| AgentdError::ConfigConflict("proposal was not applied".to_owned()))?;
        if history.rolled_back {
            return Ok(ConfigApplyResultV1 {
                proposal_id: history.proposal_id,
                path: history.path,
                applied: false,
                already_applied: true,
                restart_required: false,
            });
        }
        let (current_bytes, document) = self.load_document()?;
        let current = value_at_path(&document, &history.path)
            .cloned()
            .unwrap_or(Value::Null);
        if value_hash(&current)? != history.applied_hash {
            return Err(AgentdError::ConfigConflict(
                "Hermes or the user changed this field after Finite applied it; rollback will not overwrite that change"
                    .to_owned(),
            ));
        }
        self.checked_atomic_write(&current_bytes, &history.before_bytes)?;
        if let Err(error) = validate() {
            self.checked_atomic_write(&history.before_bytes, &current_bytes)?;
            return Err(error);
        }
        if let Err(error) = self
            .ledger
            .record_config_rollback(&history.proposal_id, &history.path)
        {
            self.checked_atomic_write(&history.before_bytes, &current_bytes)?;
            return Err(error);
        }
        Ok(ConfigApplyResultV1 {
            proposal_id: history.proposal_id,
            path: history.path,
            applied: true,
            already_applied: false,
            restart_required: true,
        })
    }

    pub fn reconcile_multimodal_specialization(
        &self,
        desired: &MultimodalSpecializationDesiredStateV1,
        activate: impl FnOnce() -> Result<(), AgentdError>,
    ) -> Result<SpecializationReconcileResultV1, AgentdError> {
        let _guard = self.lock_transaction()?;
        validate_multimodal_desired_state(desired)?;
        let (before_bytes, mut document) = self.load_document()?;
        let current = value_at_path(&document, VISION_CONFIG_PATH)
            .cloned()
            .unwrap_or(Value::Null);
        let target = multimodal_specialization_target(desired, &current)?;
        let result = || SpecializationReconcileResultV1 {
            proposal_id: desired.proposal_id.clone(),
            applied: current != target,
            already_applied: current == target,
            effective_matches_desired: false,
            model_alias: desired.model_alias.clone(),
            worker_base_url: desired.worker_base_url.clone(),
            capabilities: desired.capabilities.clone(),
            prompt_versions: desired.prompt_versions.clone(),
            normalization_limits: desired.normalization_limits.clone(),
        };
        if current == target {
            return Ok(result());
        }
        if let Some(history) = self.ledger.config_history(&desired.proposal_id)? {
            return Err(AgentdError::ConfigConflict(format!(
                "specialization proposal {} was already applied to {}",
                history.proposal_id, history.path
            )));
        }

        let offer = HermesConfigOfferV1 {
            proposal_id: desired.proposal_id.clone(),
            path: VISION_CONFIG_PATH.to_owned(),
            policy: ConfigOfferPolicyV1::ReplaceWithConfirmation,
            approved: true,
            value: target.clone(),
        };
        validate_offer(&offer)?;
        set_value_at_path(&mut document, VISION_CONFIG_PATH, target.clone())?;
        let rendered = serde_yaml::to_string(&document)?;
        self.checked_atomic_write(&before_bytes, rendered.as_bytes())?;
        if let Err(error) = activate() {
            self.checked_atomic_write(rendered.as_bytes(), &before_bytes)?;
            return Err(error);
        }
        let effective = self.current_value(VISION_CONFIG_PATH)?;
        if effective != target {
            self.checked_atomic_write(rendered.as_bytes(), &before_bytes)?;
            return Err(AgentdError::Config(
                "Hermes specialization read-back did not match desired state; previous bytes were restored"
                    .to_owned(),
            ));
        }
        let applied_hash = value_hash(&target)?;
        if let Err(error) = self.ledger.record_config_apply(
            &desired.proposal_id,
            VISION_CONFIG_PATH,
            &before_bytes,
            &applied_hash,
        ) {
            self.checked_atomic_write(rendered.as_bytes(), &before_bytes)?;
            return Err(error);
        }
        Ok(result())
    }

    pub fn activate_platform_managed_multimodal_specialization(
        &self,
        desired: &MultimodalSpecializationDesiredStateV1,
        mut validate: impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<SpecializationReconcileResultV1, AgentdError> {
        let _guard = self.lock_transaction()?;
        self.activate_platform_managed_multimodal_specialization_unlocked(desired, &mut validate)
    }

    fn activate_platform_managed_multimodal_specialization_unlocked(
        &self,
        desired: &MultimodalSpecializationDesiredStateV1,
        validate: &mut impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<SpecializationReconcileResultV1, AgentdError> {
        validate_multimodal_desired_state(desired)?;
        self.recover_startup_specialization_transition(validate)?;
        let (before_bytes, mut document) = self.load_document()?;
        let current_vision = value_at_path(&document, VISION_CONFIG_PATH).cloned();
        let ownership = self.ledger.startup_specialization_state()?;
        if let Some(ownership) = ownership.as_ref()
            && (current_vision
                .as_ref()
                .map(value_hash)
                .transpose()?
                .as_deref()
                != Some(ownership.vision_applied_hash.as_str())
                || (ownership.video_toolset_added
                    && !finitechat_toolset_contains(&document, "video")))
        {
            return Err(AgentdError::ConfigConflict(
                "Hermes or the user changed a platform-managed specialization field; startup activation will not overwrite that drift"
                    .to_owned(),
            ));
        }

        let target = multimodal_specialization_provider_target(
            desired,
            current_vision.as_ref().unwrap_or(&Value::Null),
        )?;
        set_value_at_path(&mut document, VISION_CONFIG_PATH, target.clone())?;
        let toolset_additions = if desired.capabilities.video {
            ensure_finitechat_video_toolset(&mut document)?
        } else {
            FinitechatToolsetAdditions::default()
        };
        let rendered = serde_yaml::to_string(&document)?;
        if before_bytes == rendered.as_bytes()
            && ownership
                .as_ref()
                .is_some_and(|state| state.proposal_id == desired.proposal_id)
        {
            return Ok(startup_specialization_result(desired, false));
        }
        if ownership
            .as_ref()
            .is_some_and(|state| state.previous_applied_bytes.is_some())
        {
            self.rollback_unverified_startup_multimodal_specialization_unlocked(validate)?;
            return self
                .activate_platform_managed_multimodal_specialization_unlocked(desired, validate);
        }

        let original_bytes = ownership
            .as_ref()
            .map(|state| state.before_bytes.clone())
            .unwrap_or_else(|| before_bytes.clone());
        let vision_before = ownership
            .as_ref()
            .map(|state| state.vision_before.clone())
            .unwrap_or_else(|| current_vision.clone());
        self.ledger
            .begin_startup_specialization_apply(&StartupSpecializationApplyIntent {
                proposal_id: &desired.proposal_id,
                before_bytes: &original_bytes,
                transition_before_bytes: &before_bytes,
                vision_before: vision_before.as_ref(),
                video_toolset_added: toolset_additions.video,
                exact_restore_eligible: true,
                applied_bytes: rendered.as_bytes(),
                vision_applied_hash: &value_hash(&target)?,
            })?;

        if let Err(error) = self.checked_atomic_write(&before_bytes, rendered.as_bytes()) {
            if matches!(error, AgentdError::ConfigConflict(_)) {
                self.ledger.cancel_startup_specialization_apply()?;
                return Err(error);
            }
            if let Err(restore_error) =
                self.checked_atomic_write(rendered.as_bytes(), &before_bytes)
            {
                return Err(AgentdError::Config(format!(
                    "startup specialization write failed ({error}); byte-identical rollback failed ({restore_error}) and the durable activation intent was retained"
                )));
            }
            self.ledger.cancel_startup_specialization_apply()?;
            return Err(error);
        }
        if let Err(error) = validate() {
            self.restore_transaction_bytes(
                rendered.as_bytes(),
                &before_bytes,
                &error,
                "startup specialization validation",
            )?;
            self.ledger.cancel_startup_specialization_apply()?;
            return Err(error);
        }
        if !self.startup_multimodal_specialization_matches(desired)? {
            let error = AgentdError::Config(
                "Hermes specialization read-back did not match the provider and video toolset contract"
                    .to_owned(),
            );
            self.restore_transaction_bytes(
                rendered.as_bytes(),
                &before_bytes,
                &error,
                "startup specialization read-back",
            )?;
            self.ledger.cancel_startup_specialization_apply()?;
            return Err(error);
        }
        if let Err(error) = self.ledger.finish_startup_specialization_apply() {
            self.restore_transaction_bytes(
                rendered.as_bytes(),
                &before_bytes,
                &error,
                "startup specialization ledger commit",
            )?;
            self.ledger.cancel_startup_specialization_apply()?;
            return Err(error);
        }
        Ok(startup_specialization_result(desired, true))
    }

    pub fn deactivate_startup_multimodal_specialization(
        &self,
        mut validate: impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<bool, AgentdError> {
        let _guard = self.lock_transaction()?;
        self.deactivate_startup_multimodal_specialization_unlocked(&mut validate)
    }

    fn deactivate_startup_multimodal_specialization_unlocked(
        &self,
        validate: &mut impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<bool, AgentdError> {
        self.recover_startup_specialization_transition(validate)?;
        let Some(ownership) = self.ledger.startup_specialization_state()? else {
            return Ok(false);
        };
        let (before_bytes, mut document) = self.load_document()?;
        let current_vision = value_at_path(&document, VISION_CONFIG_PATH).cloned();
        if current_vision
            .as_ref()
            .map(value_hash)
            .transpose()?
            .as_deref()
            != Some(ownership.vision_applied_hash.as_str())
            || (ownership.video_toolset_added && !finitechat_toolset_contains(&document, "video"))
        {
            return Err(AgentdError::ConfigConflict(
                "Hermes or the user changed a platform-managed specialization field; removal will not overwrite that drift"
                    .to_owned(),
            ));
        }
        self.ledger
            .begin_startup_specialization_removal(&before_bytes)?;
        restore_optional_value_at_path(&mut document, VISION_CONFIG_PATH, ownership.vision_before)?;
        if ownership.video_toolset_added {
            remove_finitechat_toolset(&mut document, "video")?;
        }
        let rendered = serde_yaml::to_string(&document)?;
        let restored_bytes =
            if ownership.exact_restore_eligible && before_bytes == ownership.applied_bytes {
                ownership.before_bytes
            } else {
                rendered.into_bytes()
            };
        if let Err(error) = self.checked_atomic_write(&before_bytes, &restored_bytes) {
            if matches!(error, AgentdError::ConfigConflict(_)) {
                self.ledger.cancel_startup_specialization_removal()?;
                return Err(error);
            }
            if let Err(restore_error) = self.checked_atomic_write(&restored_bytes, &before_bytes) {
                return Err(AgentdError::Config(format!(
                    "startup specialization removal write failed ({error}); byte-identical rollback failed ({restore_error}) and the durable removal intent was retained"
                )));
            }
            self.ledger.cancel_startup_specialization_removal()?;
            return Err(error);
        }
        if let Err(error) = validate() {
            self.restore_transaction_bytes(
                &restored_bytes,
                &before_bytes,
                &error,
                "startup specialization removal validation",
            )?;
            self.ledger.cancel_startup_specialization_removal()?;
            return Err(error);
        }
        if let Err(error) = self.ledger.clear_startup_specialization() {
            self.restore_transaction_bytes(
                &restored_bytes,
                &before_bytes,
                &error,
                "startup specialization removal ledger commit",
            )?;
            self.ledger.cancel_startup_specialization_removal()?;
            return Err(error);
        }
        Ok(true)
    }

    pub fn confirm_startup_multimodal_specialization_semantics(&self) -> Result<(), AgentdError> {
        let _guard = self.lock_transaction()?;
        self.ledger.confirm_startup_specialization_semantics()
    }

    pub fn startup_specialization_cleanup_blocked(&self) -> bool {
        match self.ledger.startup_specialization_state() {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => false,
        }
    }

    pub fn rollback_unverified_startup_multimodal_specialization(
        &self,
        mut validate: impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<(), AgentdError> {
        let _guard = self.lock_transaction()?;
        self.rollback_unverified_startup_multimodal_specialization_unlocked(&mut validate)
    }

    fn rollback_unverified_startup_multimodal_specialization_unlocked(
        &self,
        validate: &mut impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<(), AgentdError> {
        let Some(state) = self.ledger.startup_specialization_state()? else {
            return Ok(());
        };
        let Some(previous_bytes) = state.previous_applied_bytes.as_ref() else {
            self.deactivate_startup_multimodal_specialization_unlocked(validate)?;
            return Ok(());
        };
        let (current_bytes, mut document) = self.load_document()?;
        let current_vision = value_at_path(&document, VISION_CONFIG_PATH).cloned();
        if current_vision
            .as_ref()
            .map(value_hash)
            .transpose()?
            .as_deref()
            != Some(state.vision_applied_hash.as_str())
            || (state.video_toolset_added && !finitechat_toolset_contains(&document, "video"))
        {
            return Err(AgentdError::ConfigConflict(
                "Hermes or the user changed a platform-managed specialization field; semantic rollback will not overwrite that drift"
                    .to_owned(),
            ));
        }
        let previous_document = serde_yaml::from_slice::<Value>(previous_bytes)?;
        copy_specialization_owned_fields(
            &mut document,
            &previous_document,
            state.video_toolset_added,
        )?;
        let rollback_bytes = if current_bytes == state.applied_bytes {
            previous_bytes.clone()
        } else {
            serde_yaml::to_string(&document)?.into_bytes()
        };
        self.ledger
            .begin_unverified_startup_specialization_rollback()?;
        if let Err(error) = self.checked_atomic_write(&current_bytes, &rollback_bytes) {
            if matches!(error, AgentdError::ConfigConflict(_)) {
                self.ledger
                    .cancel_unverified_startup_specialization_rollback()?;
                return Err(error);
            }
            self.restore_transaction_bytes(
                &rollback_bytes,
                &current_bytes,
                &error,
                "startup specialization semantic rollback write",
            )?;
            self.ledger
                .cancel_unverified_startup_specialization_rollback()?;
            return Err(error);
        }
        if let Err(error) = validate() {
            self.restore_transaction_bytes(
                &rollback_bytes,
                &current_bytes,
                &error,
                "startup specialization semantic rollback validation",
            )?;
            self.ledger
                .cancel_unverified_startup_specialization_rollback()?;
            return Err(error);
        }
        if let Err(error) = self
            .ledger
            .finish_unverified_startup_specialization_rollback(&rollback_bytes)
        {
            self.restore_transaction_bytes(
                &rollback_bytes,
                &current_bytes,
                &error,
                "startup specialization semantic rollback ledger commit",
            )?;
            self.ledger
                .cancel_unverified_startup_specialization_rollback()?;
            return Err(error);
        }
        Ok(())
    }

    fn recover_startup_specialization_transition(
        &self,
        validate: &mut impl FnMut() -> Result<(), AgentdError>,
    ) -> Result<(), AgentdError> {
        let Some(state) = self.ledger.startup_specialization_state()? else {
            return Ok(());
        };
        match state.phase {
            StartupSpecializationPhase::Active => Ok(()),
            StartupSpecializationPhase::Applying => {
                let current_bytes = fs::read(&self.path)?;
                let prior_bytes = state.transition_before_bytes.as_slice();
                let prior_document = serde_yaml::from_slice::<Value>(prior_bytes)?;
                let applied_document = serde_yaml::from_slice::<Value>(&state.applied_bytes)?;
                let mut current_document = serde_yaml::from_slice::<Value>(&current_bytes)?;
                let rollback_bytes;
                if current_bytes == prior_bytes {
                    self.checked_atomic_write(&current_bytes, &state.applied_bytes)?;
                    rollback_bytes = prior_bytes.to_vec();
                } else if current_bytes != state.applied_bytes {
                    let recovered_applied_bytes;
                    if specialization_owned_fields_match(
                        &current_document,
                        &prior_document,
                        state.video_toolset_added,
                    ) {
                        rollback_bytes = current_bytes.clone();
                        copy_specialization_owned_fields(
                            &mut current_document,
                            &applied_document,
                            state.video_toolset_added,
                        )?;
                        recovered_applied_bytes =
                            serde_yaml::to_string(&current_document)?.into_bytes();
                        self.checked_atomic_write(&current_bytes, &recovered_applied_bytes)?;
                    } else if specialization_owned_fields_match(
                        &current_document,
                        &applied_document,
                        state.video_toolset_added,
                    ) {
                        recovered_applied_bytes = current_bytes.clone();
                        copy_specialization_owned_fields(
                            &mut current_document,
                            &prior_document,
                            state.video_toolset_added,
                        )?;
                        rollback_bytes = serde_yaml::to_string(&current_document)?.into_bytes();
                    } else {
                        return Err(AgentdError::ConfigConflict(
                            "startup specialization activation was interrupted and managed fields drifted"
                                .to_owned(),
                        ));
                    }
                    self.ledger
                        .update_interrupted_startup_specialization_apply(
                            &recovered_applied_bytes,
                            &rollback_bytes,
                        )?;
                } else {
                    rollback_bytes = prior_bytes.to_vec();
                }
                let transaction_bytes = fs::read(&self.path)?;
                if let Err(error) = validate() {
                    self.restore_transaction_bytes(
                        &transaction_bytes,
                        &rollback_bytes,
                        &error,
                        "interrupted startup specialization validation",
                    )?;
                    self.ledger.cancel_startup_specialization_apply()?;
                    return Err(error);
                }
                self.ledger.finish_startup_specialization_apply()
            }
            StartupSpecializationPhase::Removing => {
                let (current_bytes, mut document) = self.load_document()?;
                let current_vision = value_at_path(&document, VISION_CONFIG_PATH).cloned();
                let already_restored = current_vision == state.vision_before
                    && (!state.video_toolset_added
                        || !finitechat_toolset_contains(&document, "video"));
                if !already_restored
                    && (current_vision
                        .as_ref()
                        .map(value_hash)
                        .transpose()?
                        .as_deref()
                        != Some(state.vision_applied_hash.as_str())
                        || (state.video_toolset_added
                            && !finitechat_toolset_contains(&document, "video")))
                {
                    return Err(AgentdError::ConfigConflict(
                        "startup specialization removal was interrupted and managed fields drifted"
                            .to_owned(),
                    ));
                }
                let active_rollback_bytes = if !already_restored {
                    current_bytes.clone()
                } else if state.exact_restore_eligible && current_bytes == state.before_bytes {
                    state.transition_before_bytes.clone()
                } else {
                    let applied_document = serde_yaml::from_slice::<Value>(&state.applied_bytes)?;
                    let mut active_document = document.clone();
                    copy_specialization_owned_fields(
                        &mut active_document,
                        &applied_document,
                        state.video_toolset_added,
                    )?;
                    serde_yaml::to_string(&active_document)?.into_bytes()
                };
                if !already_restored {
                    restore_optional_value_at_path(
                        &mut document,
                        VISION_CONFIG_PATH,
                        state.vision_before,
                    )?;
                    if state.video_toolset_added {
                        remove_finitechat_toolset(&mut document, "video")?;
                    }
                    let rendered = serde_yaml::to_string(&document)?;
                    let restored_bytes =
                        if state.exact_restore_eligible && current_bytes == state.applied_bytes {
                            state.before_bytes
                        } else {
                            rendered.into_bytes()
                        };
                    self.checked_atomic_write(&current_bytes, &restored_bytes)?;
                }
                let transaction_bytes = fs::read(&self.path)?;
                if let Err(error) = validate() {
                    self.restore_transaction_bytes(
                        &transaction_bytes,
                        &active_rollback_bytes,
                        &error,
                        "interrupted startup specialization removal validation",
                    )?;
                    self.ledger.cancel_startup_specialization_removal()?;
                    return Err(error);
                }
                self.ledger.clear_startup_specialization()
            }
            StartupSpecializationPhase::SemanticRollback => {
                let previous_bytes = state.previous_applied_bytes.as_ref().ok_or_else(|| {
                    AgentdError::Ledger(
                        "startup specialization semantic rollback is missing its prior generation"
                            .to_owned(),
                    )
                })?;
                let current_bytes = fs::read(&self.path)?;
                let previous_document = serde_yaml::from_slice::<Value>(previous_bytes)?;
                let applied_document = serde_yaml::from_slice::<Value>(&state.applied_bytes)?;
                let mut current_document = serde_yaml::from_slice::<Value>(&current_bytes)?;
                let failed_generation_bytes;
                let recovered_previous_bytes;
                if current_bytes == state.applied_bytes {
                    self.checked_atomic_write(&current_bytes, previous_bytes)?;
                    failed_generation_bytes = state.applied_bytes.clone();
                    recovered_previous_bytes = previous_bytes.clone();
                } else if current_bytes != *previous_bytes {
                    if specialization_owned_fields_match(
                        &current_document,
                        &applied_document,
                        state.video_toolset_added,
                    ) {
                        failed_generation_bytes = current_bytes.clone();
                        copy_specialization_owned_fields(
                            &mut current_document,
                            &previous_document,
                            state.video_toolset_added,
                        )?;
                        recovered_previous_bytes =
                            serde_yaml::to_string(&current_document)?.into_bytes();
                        self.checked_atomic_write(&current_bytes, &recovered_previous_bytes)?;
                    } else if specialization_owned_fields_match(
                        &current_document,
                        &previous_document,
                        state.video_toolset_added,
                    ) {
                        recovered_previous_bytes = current_bytes.clone();
                        copy_specialization_owned_fields(
                            &mut current_document,
                            &applied_document,
                            state.video_toolset_added,
                        )?;
                        failed_generation_bytes =
                            serde_yaml::to_string(&current_document)?.into_bytes();
                    } else {
                        return Err(AgentdError::ConfigConflict(
                            "startup specialization semantic rollback was interrupted and managed fields drifted"
                                .to_owned(),
                        ));
                    }
                } else {
                    failed_generation_bytes = state.applied_bytes.clone();
                    recovered_previous_bytes = previous_bytes.clone();
                }
                let transaction_bytes = fs::read(&self.path)?;
                if let Err(error) = validate() {
                    self.restore_transaction_bytes(
                        &transaction_bytes,
                        &failed_generation_bytes,
                        &error,
                        "interrupted semantic rollback validation",
                    )?;
                    self.ledger
                        .cancel_unverified_startup_specialization_rollback()?;
                    return Err(error);
                }
                self.ledger
                    .finish_unverified_startup_specialization_rollback(&recovered_previous_bytes)
            }
        }
    }

    pub fn multimodal_specialization_matches(
        &self,
        desired: &MultimodalSpecializationDesiredStateV1,
    ) -> Result<bool, AgentdError> {
        validate_multimodal_desired_state(desired)?;
        let current = self.current_value(VISION_CONFIG_PATH)?;
        let expected_state = json!({
            "capabilities": desired.capabilities,
            "prompt_versions": desired.prompt_versions,
            "normalization_limits": desired.normalization_limits,
        });
        let credential_matches =
            current
                .get("api_key")
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    desired
                        .worker_api_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|expected| !expected.is_empty())
                        .map(|expected| value == expected)
                        .unwrap_or_else(|| !value.trim().is_empty())
                });
        Ok(
            current.get("provider").and_then(Value::as_str) == Some("custom")
                && current.get("model").and_then(Value::as_str)
                    == Some(desired.model_alias.as_str())
                && current.get("base_url").and_then(Value::as_str)
                    == Some(desired.worker_base_url.as_str())
                && current.get("api_mode").and_then(Value::as_str) == Some("chat_completions")
                && credential_matches
                && current.pointer("/extra_body/finite_specialization") == Some(&expected_state),
        )
    }

    pub fn startup_multimodal_specialization_matches(
        &self,
        desired: &MultimodalSpecializationDesiredStateV1,
    ) -> Result<bool, AgentdError> {
        validate_multimodal_desired_state(desired)?;
        let (_bytes, document) = self.load_document()?;
        let current = value_at_path(&document, VISION_CONFIG_PATH)
            .cloned()
            .unwrap_or(Value::Null);
        Ok(
            current == multimodal_specialization_provider_target(desired, &current)?
                && (!desired.capabilities.video || finitechat_video_toolset_is_enabled(&document)),
        )
    }

    fn load_document(&self) -> Result<(Vec<u8>, Value), AgentdError> {
        let bytes = fs::read(&self.path)?;
        let document = serde_yaml::from_slice::<Value>(&bytes)?;
        if !document.is_object() {
            return Err(AgentdError::Config(
                "Hermes config root must be an object".to_owned(),
            ));
        }
        Ok((bytes, document))
    }

    fn lock_transaction(&self) -> Result<ConfigTransactionGuard<'_>, AgentdError> {
        let deadline = Instant::now() + CONFIG_LOCK_TIMEOUT;
        let process_guard = loop {
            match self.transaction_lock.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(AgentdError::Config(
                        "Hermes config transaction lock was poisoned".into(),
                    ));
                }
                Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(CONFIG_LOCK_POLL_INTERVAL);
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(AgentdError::ConfigConflict(
                        "Timed out waiting for the authoritative finite-agentd config writer"
                            .to_owned(),
                    ));
                }
            }
        };
        let lock_path = self.path.with_extension("yaml.finite-agentd.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        loop {
            match fs4::fs_std::FileExt::try_lock_exclusive(&lock_file) {
                Ok(true) => break,
                Ok(false) if Instant::now() < deadline => {
                    std::thread::sleep(CONFIG_LOCK_POLL_INTERVAL);
                }
                Ok(false) => {
                    return Err(AgentdError::ConfigConflict(
                        "Timed out waiting for another finite-agentd config writer".to_owned(),
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(ConfigTransactionGuard {
            _process_guard: process_guard,
            lock_file,
        })
    }

    fn checked_atomic_write(&self, expected_bytes: &[u8], bytes: &[u8]) -> Result<(), AgentdError> {
        if fs::read(&self.path)? != expected_bytes {
            return Err(AgentdError::ConfigConflict(
                "Hermes config changed during the transaction; the stale write was refused"
                    .to_owned(),
            ));
        }
        self.atomic_write(bytes)?;
        if fs::read(&self.path)? != bytes {
            return Err(AgentdError::ConfigConflict(
                "Hermes config changed immediately after the finite-agentd write".to_owned(),
            ));
        }
        Ok(())
    }

    fn restore_transaction_bytes(
        &self,
        expected_transaction_bytes: &[u8],
        restore_bytes: &[u8],
        original_error: &AgentdError,
        operation: &str,
    ) -> Result<(), AgentdError> {
        self.checked_atomic_write(expected_transaction_bytes, restore_bytes)
            .map_err(|restore_error| {
                AgentdError::Config(format!(
                    "{operation} failed ({original_error}); rollback was refused ({restore_error}) because the config changed, and the durable transition intent was retained"
                ))
            })
    }

    fn atomic_write(&self, bytes: &[u8]) -> Result<(), AgentdError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AgentdError::Config("Hermes config has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        let mode = fs::metadata(&self.path)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or(0o600)
            & 0o600;
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| AgentdError::Io(error.error))?;
        #[cfg(test)]
        if self
            .fail_after_persist
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AgentdError::Io(std::io::Error::other(
                "injected failure after config persist",
            )));
        }
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_atomic_write_after_persist(&self) {
        self.fail_after_persist
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn startup_specialization_result(
    desired: &MultimodalSpecializationDesiredStateV1,
    applied: bool,
) -> SpecializationReconcileResultV1 {
    SpecializationReconcileResultV1 {
        proposal_id: desired.proposal_id.clone(),
        applied,
        already_applied: !applied,
        effective_matches_desired: false,
        model_alias: desired.model_alias.clone(),
        worker_base_url: desired.worker_base_url.clone(),
        capabilities: desired.capabilities.clone(),
        prompt_versions: desired.prompt_versions.clone(),
        normalization_limits: desired.normalization_limits.clone(),
    }
}

fn ensure_finitechat_video_toolset(
    document: &mut Value,
) -> Result<FinitechatToolsetAdditions, AgentdError> {
    let root = document
        .as_object_mut()
        .ok_or_else(|| AgentdError::Config("Hermes config root must be an object".to_owned()))?;
    let platform_toolsets = match root.entry("platform_toolsets".to_owned()) {
        serde_json::map::Entry::Vacant(entry) => entry.insert(json!({})),
        serde_json::map::Entry::Occupied(entry) => entry.into_mut(),
    };
    let platform_toolsets = platform_toolsets
        .as_object_mut()
        .ok_or_else(|| AgentdError::Config("platform_toolsets must be an object".to_owned()))?;
    let Some(finitechat) = platform_toolsets.get_mut("finitechat") else {
        // An explicit platform list replaces Hermes's implicit catalog. Keep
        // the normal CLI-grade Finite Chat tools when opting into video on a
        // config that did not previously override this platform.
        platform_toolsets.insert("finitechat".to_owned(), json!(["hermes-cli", "video"]));
        return Ok(FinitechatToolsetAdditions { video: true });
    };
    let finitechat = finitechat.as_array_mut().ok_or_else(|| {
        AgentdError::Config("platform_toolsets.finitechat must be a list of strings".to_owned())
    })?;
    if !finitechat.iter().all(Value::is_string) {
        return Err(AgentdError::Config(
            "platform_toolsets.finitechat must be a list of strings".to_owned(),
        ));
    }
    let mut additions = FinitechatToolsetAdditions::default();
    if !finitechat
        .iter()
        .any(|toolset| toolset.as_str() == Some("video"))
    {
        finitechat.push(json!("video"));
        additions.video = true;
    }
    Ok(additions)
}

fn finitechat_video_toolset_is_enabled(document: &Value) -> bool {
    finitechat_toolset_contains(document, "video")
}

fn finitechat_toolset_contains(document: &Value, expected: &str) -> bool {
    value_at_path(document, FINITECHAT_TOOLSETS_CONFIG_PATH)
        .and_then(Value::as_array)
        .is_some_and(|toolsets| {
            toolsets.iter().all(Value::is_string)
                && toolsets
                    .iter()
                    .any(|toolset| toolset.as_str() == Some(expected))
        })
}

fn remove_finitechat_toolset(document: &mut Value, removed: &str) -> Result<(), AgentdError> {
    let Some(toolsets) = value_at_path_mut(document, FINITECHAT_TOOLSETS_CONFIG_PATH) else {
        return Ok(());
    };
    let toolsets = toolsets.as_array_mut().ok_or_else(|| {
        AgentdError::Config("platform_toolsets.finitechat must be a list of strings".to_owned())
    })?;
    let Some(index) = toolsets
        .iter()
        .position(|toolset| toolset.as_str() == Some(removed))
    else {
        return Ok(());
    };
    toolsets.remove(index);
    Ok(())
}

fn specialization_owned_fields_match(
    document: &Value,
    reference: &Value,
    video_toolset_owned: bool,
) -> bool {
    value_at_path(document, VISION_CONFIG_PATH) == value_at_path(reference, VISION_CONFIG_PATH)
        && (!video_toolset_owned
            || finitechat_toolset_contains(document, "video")
                == finitechat_toolset_contains(reference, "video"))
}

fn copy_specialization_owned_fields(
    document: &mut Value,
    reference: &Value,
    video_toolset_owned: bool,
) -> Result<(), AgentdError> {
    restore_optional_value_at_path(
        document,
        VISION_CONFIG_PATH,
        value_at_path(reference, VISION_CONFIG_PATH).cloned(),
    )?;
    if video_toolset_owned {
        if finitechat_toolset_contains(reference, "video") {
            ensure_finitechat_video_toolset(document)?;
        } else {
            remove_finitechat_toolset(document, "video")?;
        }
    }
    Ok(())
}

fn multimodal_specialization_target(
    desired: &MultimodalSpecializationDesiredStateV1,
    current: &Value,
) -> Result<Value, AgentdError> {
    let mut target = multimodal_specialization_provider_target(desired, current)?;
    target
        .as_object_mut()
        .expect("provider target is an object")
        .insert(
            "extra_body".to_owned(),
            json!({
                "finite_specialization": {
                    "capabilities": desired.capabilities,
                    "prompt_versions": desired.prompt_versions,
                    "normalization_limits": desired.normalization_limits,
                }
            }),
        );
    Ok(target)
}

fn multimodal_specialization_provider_target(
    desired: &MultimodalSpecializationDesiredStateV1,
    current: &Value,
) -> Result<Value, AgentdError> {
    let existing_api_key = desired
        .worker_api_key
        .as_deref()
        .or_else(|| current.get("api_key").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AgentdError::Config(
                "multimodal specialization requires an existing or supplied worker credential"
                    .to_owned(),
            )
        })?;
    Ok(json!({
        "provider": "custom",
        "model": desired.model_alias,
        "base_url": desired.worker_base_url,
        "api_key": existing_api_key,
        "api_mode": "chat_completions",
        "timeout": 120,
        "download_timeout": 30,
    }))
}

fn validate_multimodal_desired_state(
    desired: &MultimodalSpecializationDesiredStateV1,
) -> Result<(), AgentdError> {
    if desired.proposal_id.trim().is_empty() || desired.proposal_id.len() > 128 {
        return Err(AgentdError::InvalidPayload(
            "proposal_id must contain 1..128 bytes".to_owned(),
        ));
    }
    if desired.model_alias != DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL {
        return Err(AgentdError::InvalidPayload(
            "model_alias is not an approved multimodal specialization alias".to_owned(),
        ));
    }
    if desired.worker_base_url != DEFAULT_MULTIMODAL_SPECIALIZATION_WORKER_URL {
        return Err(AgentdError::InvalidPayload(
            "worker_base_url is not the canonical multimodal specialization endpoint".to_owned(),
        ));
    }
    let canonical = MultimodalSpecializationDesiredStateV1::canonical(&desired.proposal_id);
    if desired.prompt_versions != canonical.prompt_versions
        || desired.normalization_limits != canonical.normalization_limits
    {
        return Err(AgentdError::InvalidPayload(
            "multimodal prompt versions or normalization limits are not canonical".to_owned(),
        ));
    }
    Ok(())
}

fn validate_offer(offer: &HermesConfigOfferV1) -> Result<(), AgentdError> {
    if offer.proposal_id.trim().is_empty() || offer.proposal_id.len() > 128 {
        return Err(AgentdError::Config(
            "proposal_id must contain 1..128 bytes".to_owned(),
        ));
    }
    match offer.path.as_str() {
        VISION_CONFIG_PATH => validate_vision_value(&offer.value),
        MODEL_CONFIG_PATH => validate_model_value(&offer.value),
        TELEGRAM_CONFIG_PATH => validate_telegram_value(&offer.value),
        _ => Err(AgentdError::UnsupportedConfigPath(offer.path.clone())),
    }
}

fn validate_model_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("model must be an object".to_owned()))?;
    let allowed = ["default", "provider", "base_url", "api_key", "api_mode"];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "model field {key:?} is not allowlisted"
            )));
        }
    }
    for key in ["default", "provider"] {
        let valid = object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty() && value.len() <= 256);
        if !valid {
            return Err(AgentdError::Config(format!("model.{key} is required")));
        }
    }
    for key in ["base_url", "api_key", "api_mode"] {
        if let Some(value) = object.get(key)
            && !value.is_string()
        {
            return Err(AgentdError::Config(format!("model.{key} must be a string")));
        }
    }
    if let Some(mode) = object.get("api_mode").and_then(Value::as_str)
        && !["chat_completions", "codex_responses", "anthropic_messages"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "model.api_mode is unsupported".to_owned(),
        ));
    }
    Ok(())
}

fn validate_telegram_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("Telegram settings must be an object".to_owned()))?;
    let allowed = [
        "enabled",
        "token",
        "home_channel",
        "reply_to_mode",
        "gateway_restart_notification",
        "typing_indicator",
        "extra",
    ];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "Telegram field {key:?} is not allowlisted"
            )));
        }
    }
    if !object.get("enabled").is_some_and(Value::is_boolean) {
        return Err(AgentdError::Config(
            "Telegram enabled state is required".to_owned(),
        ));
    }
    if object.get("enabled") == Some(&Value::Bool(true)) {
        let token = object
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let valid = token.len() >= 32
            && token.len() <= 256
            && token.split_once(':').is_some_and(|(id, secret)| {
                id.bytes().all(|byte| byte.is_ascii_digit()) && !secret.is_empty()
            });
        if !valid {
            return Err(AgentdError::Config(
                "Telegram bot token is invalid".to_owned(),
            ));
        }
    }
    if let Some(mode) = object.get("reply_to_mode").and_then(Value::as_str)
        && !["off", "first", "all"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "Telegram reply mode is unsupported".to_owned(),
        ));
    }
    if let Some(home) = object.get("home_channel") {
        let home = home
            .as_object()
            .ok_or_else(|| AgentdError::Config("Telegram home channel is invalid".to_owned()))?;
        for key in ["platform", "chat_id", "name"] {
            if home
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(AgentdError::Config(format!(
                    "Telegram home channel {key} is required"
                )));
            }
        }
        if home.get("platform").and_then(Value::as_str) != Some("telegram") {
            return Err(AgentdError::Config(
                "Telegram home channel platform is invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_vision_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("auxiliary.vision must be an object".to_owned()))?;
    let allowed = [
        "provider",
        "model",
        "base_url",
        "api_key",
        "api_mode",
        "timeout",
        "download_timeout",
        "extra_body",
    ];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "auxiliary.vision field {key:?} is not allowlisted"
            )));
        }
    }
    let provider = object
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if provider.is_empty() || provider.len() > 128 {
        return Err(AgentdError::Config(
            "auxiliary.vision.provider is required".to_owned(),
        ));
    }
    for key in ["model", "base_url", "api_key"] {
        if let Some(value) = object.get(key)
            && !value.is_string()
        {
            return Err(AgentdError::Config(format!(
                "auxiliary.vision.{key} must be a string"
            )));
        }
    }
    if let Some(mode) = object.get("api_mode").and_then(Value::as_str)
        && !["chat_completions", "codex_responses", "anthropic_messages"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "auxiliary.vision.api_mode is unsupported".to_owned(),
        ));
    }
    for key in ["timeout", "download_timeout"] {
        if let Some(value) = object.get(key) {
            let Some(value) = value.as_u64() else {
                return Err(AgentdError::Config(format!(
                    "auxiliary.vision.{key} must be an integer"
                )));
            };
            if !(1..=900).contains(&value) {
                return Err(AgentdError::Config(format!(
                    "auxiliary.vision.{key} must be between 1 and 900 seconds"
                )));
            }
        }
    }
    if let Some(extra_body) = object.get("extra_body")
        && !extra_body.is_object()
    {
        return Err(AgentdError::Config(
            "auxiliary.vision.extra_body must be an object".to_owned(),
        ));
    }
    Ok(())
}

fn value_is_unset(path: &str, value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    if path != VISION_CONFIG_PATH {
        return object.is_empty();
    }
    object.is_empty()
        || (object.len() == 1
            && object
                .get("provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(|provider| provider.is_empty() || provider == "auto"))
}

fn value_at_path<'a>(document: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = document;
    for part in path.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn value_at_path_mut<'a>(document: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut current = document;
    for part in path.split('.') {
        current = current.as_object_mut()?.get_mut(part)?;
    }
    Some(current)
}

fn set_value_at_path(document: &mut Value, path: &str, value: Value) -> Result<(), AgentdError> {
    let mut parts = path.split('.').peekable();
    let mut current = document;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let object = current.as_object_mut().ok_or_else(|| {
                AgentdError::Config(format!("configuration parent for {path} is not an object"))
            })?;
            object.insert(part.to_owned(), value);
            return Ok(());
        }
        let object = current.as_object_mut().ok_or_else(|| {
            AgentdError::Config(format!("configuration parent for {path} is not an object"))
        })?;
        current = object
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    Err(AgentdError::Config(
        "configuration path is empty".to_owned(),
    ))
}

fn restore_optional_value_at_path(
    document: &mut Value,
    path: &str,
    value: Option<Value>,
) -> Result<(), AgentdError> {
    if let Some(value) = value {
        return set_value_at_path(document, path, value);
    }
    let mut parts = path.split('.').peekable();
    let mut current = document;
    while let Some(part) = parts.next() {
        let object = current.as_object_mut().ok_or_else(|| {
            AgentdError::Config(format!("configuration parent for {path} is not an object"))
        })?;
        if parts.peek().is_none() {
            object.remove(part);
            return Ok(());
        }
        let Some(next) = object.get_mut(part) else {
            return Ok(());
        };
        current = next;
    }
    Ok(())
}

fn value_hash(value: &Value) -> Result<String, AgentdError> {
    Ok(hex_digest(&serde_json::to_vec(value)?))
}

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let lowered = key.to_ascii_lowercase();
                    let redacted = ["key", "token", "secret", "password", "credential"]
                        .iter()
                        .any(|needle| lowered.contains(needle));
                    (
                        key.clone(),
                        if redacted {
                            json!("<redacted>")
                        } else {
                            redact_value(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use serde_json::json;

    use super::*;

    const ORIGINAL: &str = "model: anthropic/claude\nauxiliary:\n  vision:\n    provider: auto\n";

    fn manager() -> (tempfile::TempDir, ConfigManager) {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.yaml");
        fs::write(&config_path, ORIGINAL).unwrap();
        let ledger = Ledger::open(directory.path().join("agentd.sqlite3")).unwrap();
        (directory, ConfigManager::new(config_path, ledger))
    }

    fn offer(proposal_id: &str) -> HermesConfigOfferV1 {
        HermesConfigOfferV1 {
            proposal_id: proposal_id.to_owned(),
            path: VISION_CONFIG_PATH.to_owned(),
            policy: ConfigOfferPolicyV1::ApplyIfUnset,
            approved: false,
            value: json!({
                "provider": "openai",
                "model": "gpt-5-mini",
                "api_key": "never-display-this"
            }),
        }
    }

    fn stage_legacy_startup_specialization(
        manager: &ConfigManager,
        desired: &MultimodalSpecializationDesiredStateV1,
    ) {
        let (before_bytes, mut document) = manager.load_document().unwrap();
        let current = value_at_path(&document, VISION_CONFIG_PATH)
            .cloned()
            .unwrap_or(Value::Null);
        let target = multimodal_specialization_provider_target(desired, &current).unwrap();
        set_value_at_path(&mut document, VISION_CONFIG_PATH, target.clone()).unwrap();
        if desired.capabilities.video {
            ensure_finitechat_video_toolset(&mut document).unwrap();
        }
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();
        manager
            .ledger
            .record_config_apply(
                &desired.proposal_id,
                VISION_CONFIG_PATH,
                &before_bytes,
                &value_hash(&target).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn preview_redacts_secrets_and_apply_is_idempotent() {
        let (_directory, manager) = manager();
        let offer = offer("vision-1");
        let preview = manager.preview(&offer).unwrap();
        assert_eq!(preview.ownership, "unset");
        assert!(preview.would_apply);
        assert_eq!(preview.proposed["api_key"], "<redacted>");

        let applied = manager.apply(&offer, || Ok(())).unwrap();
        assert!(applied.applied);
        assert!(applied.restart_required);
        let repeated = manager.apply(&offer, || Ok(())).unwrap();
        assert!(repeated.already_applied);
        assert!(!repeated.restart_required);
    }

    #[test]
    fn failed_validation_restores_exact_previous_bytes() {
        let (_directory, manager) = manager();
        let error = manager
            .apply(&offer("vision-invalid"), || {
                Err(AgentdError::Config("Hermes rejected it".to_owned()))
            })
            .unwrap_err();
        assert!(matches!(error, AgentdError::Config(_)));
        assert_eq!(fs::read(manager.path()).unwrap(), ORIGINAL.as_bytes());
        assert!(
            manager
                .ledger
                .config_history("vision-invalid")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn apply_if_unset_does_not_overwrite_custom_hermes_config() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n",
        )
        .unwrap();
        let preview = manager.preview(&offer("vision-custom")).unwrap();
        assert_eq!(preview.ownership, "custom");
        assert!(!preview.would_apply);
        assert!(matches!(
            manager.apply(&offer("vision-custom"), || Ok(())),
            Err(AgentdError::ConfigConflict(_))
        ));
    }

    #[test]
    fn apply_if_unset_preserves_auto_profiles_with_user_details() {
        let (_directory, manager) = manager();
        let original = "auxiliary:\n  vision:\n    provider: auto\n    model: user-selected-vision\n    timeout: 240\n";
        fs::write(manager.path(), original).unwrap();

        let preview = manager.preview(&offer("vision-auto-customized")).unwrap();

        assert_eq!(preview.ownership, "custom");
        assert!(!preview.would_apply);
        assert!(matches!(
            manager.apply(&offer("vision-auto-customized"), || Ok(())),
            Err(AgentdError::ConfigConflict(_))
        ));
        assert_eq!(fs::read(manager.path()).unwrap(), original.as_bytes());
    }

    #[test]
    fn rollback_is_exact_but_refuses_to_clobber_later_user_edits() {
        let (_directory, manager) = manager();
        manager.apply(&offer("vision-rollback"), || Ok(())).unwrap();
        let rollback = HermesConfigRollbackV1 {
            proposal_id: "vision-rollback".to_owned(),
        };
        let result = manager.rollback(&rollback, || Ok(())).unwrap();
        assert!(result.applied);
        assert_eq!(fs::read(manager.path()).unwrap(), ORIGINAL.as_bytes());

        manager.apply(&offer("vision-drift"), || Ok(())).unwrap();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: user-edited\n",
        )
        .unwrap();
        assert!(matches!(
            manager.rollback(
                &HermesConfigRollbackV1 {
                    proposal_id: "vision-drift".to_owned()
                },
                || Ok(())
            ),
            Err(AgentdError::ConfigConflict(_))
        ));
    }

    #[test]
    fn multimodal_specialization_replaces_only_vision_and_preserves_worker_credential() {
        let (directory, manager) = manager();
        let original = "model:\n  default: main-model\n  provider: custom\ngateway:\n  platforms:\n    telegram:\n      enabled: true\nauxiliary:\n  vision:\n    provider: custom\n    model: qwopus-old\n    base_url: http://old-worker/v1\n    api_key: worker-secret\n";
        fs::write(manager.path(), original).unwrap();
        let desired = MultimodalSpecializationDesiredStateV1::canonical("multimodal-reconcile-1");

        let result = manager
            .reconcile_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        assert!(result.applied);
        assert!(!result.effective_matches_desired);
        assert!(manager.multimodal_specialization_matches(&desired).unwrap());
        assert!(result.capabilities.image);
        assert!(result.capabilities.audio);
        assert!(result.capabilities.video);
        let document: Value = serde_yaml::from_slice(&fs::read(manager.path()).unwrap()).unwrap();
        assert_eq!(document["model"]["default"], "main-model");
        assert_eq!(
            document["gateway"]["platforms"]["telegram"]["enabled"],
            true
        );
        assert_eq!(
            document["auxiliary"]["vision"]["model"],
            DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL
        );
        assert_eq!(
            document["auxiliary"]["vision"]["base_url"],
            DEFAULT_MULTIMODAL_SPECIALIZATION_WORKER_URL
        );
        assert_eq!(document["auxiliary"]["vision"]["api_key"], "worker-secret");
        assert_eq!(
            document["auxiliary"]["vision"]["extra_body"]["finite_specialization"]["capabilities"],
            json!({ "image": true, "audio": true, "video": true })
        );

        let after_first_apply = fs::read(manager.path()).unwrap();
        let repeated = manager
            .reconcile_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(repeated.already_applied);
        assert_eq!(fs::read(manager.path()).unwrap(), after_first_apply);
        drop(directory);
    }

    #[test]
    fn multimodal_specialization_allows_only_the_canonical_model() {
        let mut desired = MultimodalSpecializationDesiredStateV1::canonical("canonical-model");
        validate_multimodal_desired_state(&desired).unwrap();

        desired.model_alias = "unapproved-model".to_owned();
        assert!(matches!(
            validate_multimodal_desired_state(&desired),
            Err(AgentdError::InvalidPayload(_))
        ));
    }

    #[test]
    fn startup_multimodal_activation_is_idempotent_and_read_back_verified() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-token-1");
        desired.worker_api_key = Some("dedicated-worker-secret".to_owned());

        let first = manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(first.applied);
        assert!(!first.effective_matches_desired);
        assert!(
            manager
                .startup_multimodal_specialization_matches(&desired)
                .unwrap()
        );
        assert!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["extra_body"].is_null(),
            "startup activation must remain a plain Hermes provider profile"
        );
        assert_eq!(
            manager
                .current_value(FINITECHAT_TOOLSETS_CONFIG_PATH)
                .unwrap(),
            json!(["hermes-cli", "video"]),
            "the startup bundle must preserve ordinary tools while opting Finite Chat into video"
        );

        let after_first_apply = fs::read(manager.path()).unwrap();
        let repeated = manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(repeated.already_applied);
        assert!(!repeated.effective_matches_desired);
        assert_eq!(fs::read(manager.path()).unwrap(), after_first_apply);
    }

    #[test]
    fn startup_multimodal_rejournals_a_changed_operation_identity() {
        let (_directory, manager) = manager();
        let mut previous =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-bundle-previous-release");
        previous.worker_api_key = Some("dedicated-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&previous, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        let previous_bytes = fs::read(manager.path()).unwrap();

        let mut desired = MultimodalSpecializationDesiredStateV1::canonical(
            "runtime-bundle-finite-private-multimodal-v1-current",
        );
        desired.worker_api_key = Some("dedicated-worker-secret".to_owned());
        let converged = manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        assert!(converged.applied, "operation identities are not aliases");
        assert_eq!(fs::read(manager.path()).unwrap(), previous_bytes);
        let state = manager
            .ledger
            .startup_specialization_state()
            .unwrap()
            .unwrap();
        assert_eq!(state.proposal_id, desired.proposal_id);
        assert_eq!(
            state.previous_proposal_id.as_deref(),
            Some(previous.proposal_id.as_str())
        );
    }

    #[test]
    fn startup_multimodal_activation_replaces_and_restores_user_owned_vision_profile() {
        let (_directory, manager) = manager();
        let original = "model: anthropic/claude\nunrelated:\n  theme: dark\nplatform_toolsets:\n  finitechat: [hermes-cli, custom]\nauxiliary:\n  vision:\n    provider: custom\n    model: user-selected-vision\n    base_url: https://vision.example/v1\n    api_key: user-secret\n";
        fs::write(manager.path(), original).unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-custom");
        desired.worker_api_key = Some("dedicated-worker-secret".to_owned());

        let activated = manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(activated.applied);
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL
        );
        assert_eq!(
            manager
                .current_value(FINITECHAT_TOOLSETS_CONFIG_PATH)
                .unwrap(),
            json!(["hermes-cli", "custom", "video"])
        );

        let (_bytes, mut edited) = manager.load_document().unwrap();
        set_value_at_path(&mut edited, "unrelated.theme", json!("light")).unwrap();
        value_at_path_mut(&mut edited, FINITECHAT_TOOLSETS_CONFIG_PATH)
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!("user-added-after-activation"));
        manager
            .atomic_write(serde_yaml::to_string(&edited).unwrap().as_bytes())
            .unwrap();
        let removed = manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        assert!(removed);
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            "user-selected-vision"
        );
        assert_eq!(
            manager
                .current_value(FINITECHAT_TOOLSETS_CONFIG_PATH)
                .unwrap(),
            json!(["hermes-cli", "custom", "user-added-after-activation"])
        );
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            json!("light"),
            "restoration must not overwrite unrelated edits"
        );
    }

    #[test]
    fn startup_multimodal_activation_adds_video_without_overwriting_user_toolsets() {
        let (_directory, manager) = manager();
        let original = "platform_toolsets:\n  finitechat: [hermes-cli, vision]\nauxiliary: {}\n";
        fs::write(manager.path(), original).unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-custom-tools");
        desired.worker_api_key = Some("dedicated-worker-secret".to_owned());

        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert_eq!(
            manager
                .current_value(FINITECHAT_TOOLSETS_CONFIG_PATH)
                .unwrap(),
            json!(["hermes-cli", "vision", "video"])
        );
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        assert_eq!(
            manager
                .current_value(FINITECHAT_TOOLSETS_CONFIG_PATH)
                .unwrap(),
            json!(["hermes-cli", "vision"])
        );
    }

    #[test]
    fn startup_multimodal_activation_refuses_toolset_drift() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-owned-tools");
        desired.worker_api_key = Some("dedicated-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        let (before, mut document) = manager.load_document().unwrap();
        set_value_at_path(&mut document, "platform_toolsets", json!({})).unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();
        assert_ne!(fs::read(manager.path()).unwrap(), before);

        assert!(matches!(
            manager.activate_platform_managed_multimodal_specialization(&desired, || Ok(())),
            Err(AgentdError::ConfigConflict(_))
        ));
    }

    #[test]
    fn startup_multimodal_activation_rotates_a_finite_owned_worker_credential() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    model: user-original\n    api_key: user-secret\n",
        )
        .unwrap();
        let mut first =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-token-1");
        first.worker_api_key = Some("worker-secret-one".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
            .unwrap();

        let mut rotated =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-token-2");
        rotated.worker_api_key = Some("worker-secret-two".to_owned());
        assert!(
            !manager
                .startup_multimodal_specialization_matches(&rotated)
                .unwrap()
        );
        let result = manager
            .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
            .unwrap();

        assert!(result.applied);
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["api_key"],
            "worker-secret-two"
        );
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            "user-original",
            "rotation must retain the original pre-managed value"
        );
    }

    #[test]
    fn failed_rotation_restores_previous_video_ownership_before_rearm() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "platform_toolsets:\n  finitechat: [video]\nauxiliary: {}\n",
        )
        .unwrap();
        let mut first =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-user-video-a");
        first.worker_api_key = Some("worker-secret-a".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        let (_bytes, mut document) = manager.load_document().unwrap();
        remove_finitechat_toolset(&mut document, "video").unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();

        let mut rotated =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-user-video-b");
        rotated.worker_api_key = Some("worker-secret-b".to_owned());
        assert!(
            manager
                .activate_platform_managed_multimodal_specialization(&rotated, || {
                    Err(AgentdError::Config(
                        "injected validation failure".to_owned(),
                    ))
                })
                .is_err()
        );
        let restored = manager
            .ledger
            .startup_specialization_state()
            .unwrap()
            .unwrap();
        assert!(!restored.video_toolset_added);
        assert_eq!(restored.proposal_id, first.proposal_id);
        assert!(!finitechat_toolset_contains(
            &manager.load_document().unwrap().1,
            "video"
        ));

        manager
            .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
            .unwrap();
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        assert!(!finitechat_toolset_contains(
            &manager.load_document().unwrap().1,
            "video"
        ));
    }

    #[test]
    fn second_rotation_rolls_back_unverified_generation_before_applying_desired() {
        let (_directory, manager) = manager();
        let mut first = MultimodalSpecializationDesiredStateV1::canonical("verified-generation-a");
        first.worker_api_key = Some("worker-secret-a".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        let first_bytes = fs::read(manager.path()).unwrap();

        let mut second =
            MultimodalSpecializationDesiredStateV1::canonical("unverified-generation-b");
        second.worker_api_key = Some("worker-secret-b".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&second, || Ok(()))
            .unwrap();
        let mut third =
            MultimodalSpecializationDesiredStateV1::canonical("unverified-generation-c");
        third.worker_api_key = Some("worker-secret-c".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&third, || Ok(()))
            .unwrap();
        assert!(
            manager
                .startup_multimodal_specialization_matches(&third)
                .unwrap()
        );

        manager
            .rollback_unverified_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        assert_eq!(fs::read(manager.path()).unwrap(), first_bytes);
        assert!(
            manager
                .startup_multimodal_specialization_matches(&first)
                .unwrap()
        );
    }

    #[test]
    fn startup_multimodal_removal_after_confirmed_rotation_preserves_unrelated_edits() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    model: user-original\n",
        )
        .unwrap();
        let mut first =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-remove-a");
        first.worker_api_key = Some("worker-secret-a".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        let (_bytes, mut document) = manager.load_document().unwrap();
        set_value_at_path(&mut document, "unrelated.theme", json!("preserved")).unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();

        let mut rotated =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-remove-b");
        rotated.worker_api_key = Some("worker-secret-b".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();

        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            "user-original"
        );
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            "preserved"
        );
    }

    #[test]
    fn startup_multimodal_removal_after_failed_rotation_preserves_unrelated_edits() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    model: user-original\n",
        )
        .unwrap();
        let mut first =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-failed-remove-a");
        first.worker_api_key = Some("worker-secret-a".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
            .unwrap();
        manager
            .confirm_startup_multimodal_specialization_semantics()
            .unwrap();
        let (_bytes, mut document) = manager.load_document().unwrap();
        set_value_at_path(&mut document, "unrelated.theme", json!("preserved")).unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();

        let mut rotated =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-failed-remove-b");
        rotated.worker_api_key = Some("worker-secret-b".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
            .unwrap();
        manager
            .rollback_unverified_startup_multimodal_specialization(|| Ok(()))
            .unwrap();
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();

        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            "user-original"
        );
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            "preserved"
        );
    }

    #[test]
    fn startup_specialization_journal_debug_redacts_current_and_prior_credentials() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    api_key: prior-user-secret\n",
        )
        .unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-debug-redaction");
        desired.worker_api_key = Some("current-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        let debug = format!(
            "{:?}",
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap()
        );
        assert!(!debug.contains("prior-user-secret"));
        assert!(!debug.contains("current-worker-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn startup_specialization_desired_state_debug_redacts_worker_credential() {
        let mut desired = MultimodalSpecializationDesiredStateV1::canonical("debug-redaction");
        desired.worker_api_key = Some("worker-secret-that-must-not-leak".to_owned());

        let debug = format!("{desired:?}");

        assert!(!debug.contains("worker-secret-that-must-not-leak"));
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("debug-redaction"));
    }

    #[test]
    fn startup_cleanup_status_fails_closed_when_ledger_is_unreadable() {
        let (directory, manager) = manager();
        fs::write(
            directory.path().join("agentd.sqlite3"),
            b"not a sqlite database",
        )
        .unwrap();

        assert!(manager.startup_specialization_cleanup_blocked());
    }

    #[test]
    fn startup_multimodal_treats_legacy_live_config_as_the_conservative_baseline() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    model: user-original\n    api_key: user-secret\n",
        )
        .unwrap();
        let mut legacy = MultimodalSpecializationDesiredStateV1::canonical(format!(
            "runtime-bundle-{DEFAULT_MULTIMODAL_SPECIALIZATION_BUNDLE}-legacy"
        ));
        legacy.worker_api_key = Some("legacy-worker-secret".to_owned());
        stage_legacy_startup_specialization(&manager, &legacy);
        let (_bytes, mut document) = manager.load_document().unwrap();
        assert!(finitechat_toolset_contains(&document, "video"));
        set_value_at_path(&mut document, "unrelated.theme", json!("preserved")).unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();
        let legacy_live_bytes = fs::read(manager.path()).unwrap();

        let mut startup =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-adopt");
        startup.worker_api_key = Some("current-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&startup, || Ok(()))
            .unwrap();
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();

        assert_eq!(fs::read(manager.path()).unwrap(), legacy_live_bytes);
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            "preserved"
        );
        assert!(
            finitechat_toolset_contains(&manager.load_document().unwrap().1, "video"),
            "ambiguous legacy video ownership must be preserved"
        );
    }

    #[test]
    fn startup_multimodal_legacy_live_baseline_preserves_user_owned_video_membership() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "platform_toolsets:\n  finitechat: [video]\nauxiliary:\n  vision:\n    provider: custom\n    model: user-original\n",
        )
        .unwrap();
        let mut legacy = MultimodalSpecializationDesiredStateV1::canonical(format!(
            "runtime-bundle-{DEFAULT_MULTIMODAL_SPECIALIZATION_BUNDLE}-user-video"
        ));
        legacy.worker_api_key = Some("legacy-worker-secret".to_owned());
        stage_legacy_startup_specialization(&manager, &legacy);

        let mut startup = MultimodalSpecializationDesiredStateV1::canonical(
            "runtime-multimodal-adopt-user-video",
        );
        startup.worker_api_key = Some("current-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&startup, || Ok(()))
            .unwrap();
        let state = manager
            .ledger
            .startup_specialization_state()
            .unwrap()
            .unwrap();
        assert!(!state.video_toolset_added);
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();

        assert!(finitechat_toolset_contains(
            &manager.load_document().unwrap().1,
            "video"
        ));
    }

    #[test]
    fn startup_multimodal_does_not_adopt_generic_config_ownership_as_legacy_specialization() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    model: user-original\n",
        )
        .unwrap();
        let mut forged_legacy =
            MultimodalSpecializationDesiredStateV1::canonical("forged-legacy-shape");
        forged_legacy.worker_api_key = Some("user-selected-secret".to_owned());
        let explicitly_selected =
            multimodal_specialization_provider_target(&forged_legacy, &Value::Null).unwrap();
        manager
            .apply(
                &HermesConfigOfferV1 {
                    proposal_id: format!(
                        "runtime-bundle-{DEFAULT_MULTIMODAL_SPECIALIZATION_BUNDLE}-forged"
                    ),
                    path: VISION_CONFIG_PATH.to_owned(),
                    policy: ConfigOfferPolicyV1::ReplaceWithConfirmation,
                    approved: true,
                    value: explicitly_selected.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        let (_bytes, mut document) = manager.load_document().unwrap();
        ensure_finitechat_video_toolset(&mut document).unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();

        let mut startup = MultimodalSpecializationDesiredStateV1::canonical(
            "runtime-multimodal-after-generic-config",
        );
        startup.worker_api_key = Some("current-worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&startup, || Ok(()))
            .unwrap();
        manager
            .deactivate_startup_multimodal_specialization(|| Ok(()))
            .unwrap();

        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap(),
            explicitly_selected
        );
        assert!(finitechat_toolset_contains(
            &manager.load_document().unwrap().1,
            "video"
        ));
    }

    #[test]
    fn cloned_config_managers_serialize_unrelated_live_writers() {
        let (_directory, manager) = manager();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let activation_manager = manager.clone();
        let activation_barrier = barrier.clone();
        let activation = std::thread::spawn(move || {
            let mut desired = MultimodalSpecializationDesiredStateV1::canonical(
                "concurrent-startup-specialization",
            );
            desired.worker_api_key = Some("worker-secret".to_owned());
            activation_barrier.wait();
            activation_manager
                .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
                .unwrap();
        });
        let config_manager = manager.clone();
        let config_barrier = barrier.clone();
        let config_write = std::thread::spawn(move || {
            config_barrier.wait();
            config_manager
                .apply(
                    &HermesConfigOfferV1 {
                        proposal_id: "concurrent-model-selection".to_owned(),
                        path: MODEL_CONFIG_PATH.to_owned(),
                        policy: ConfigOfferPolicyV1::ReplaceWithConfirmation,
                        approved: true,
                        value: json!({
                            "default": "openai/gpt-5-mini",
                            "provider": "openai",
                            "api_key": "user-model-secret"
                        }),
                    },
                    || Ok(()),
                )
                .unwrap();
        });
        barrier.wait();
        activation.join().unwrap();
        config_write.join().unwrap();

        assert_eq!(
            manager.current_value(MODEL_CONFIG_PATH).unwrap()["default"],
            "openai/gpt-5-mini"
        );
        let state = manager
            .ledger
            .startup_specialization_state()
            .unwrap()
            .unwrap();
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            DEFAULT_MULTIMODAL_SPECIALIZATION_MODEL
        );
        assert_eq!(state.phase, "active");
    }

    #[test]
    fn startup_multimodal_restoration_refuses_owned_field_drift() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-drift");
        desired.worker_api_key = Some("worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        let (_bytes, mut document) = manager.load_document().unwrap();
        set_value_at_path(
            &mut document,
            VISION_CONFIG_PATH,
            json!({"provider": "custom", "model": "user-drift"}),
        )
        .unwrap();
        manager
            .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
            .unwrap();

        assert!(matches!(
            manager.deactivate_startup_multimodal_specialization(|| Ok(())),
            Err(AgentdError::ConfigConflict(_))
        ));
        assert_eq!(
            manager.current_value(VISION_CONFIG_PATH).unwrap()["model"],
            "user-drift"
        );
    }

    #[test]
    fn startup_multimodal_validation_failure_restores_exact_previous_bytes() {
        let (_directory, manager) = manager();
        let original = b"# retain this formatting\nauxiliary: {vision: {provider: custom, model: user-vision}}\n";
        fs::write(manager.path(), original).unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-invalid");
        desired.worker_api_key = Some("worker-secret".to_owned());

        assert!(matches!(
            manager.activate_platform_managed_multimodal_specialization(&desired, || Err(
                AgentdError::Config("validation failed".to_owned())
            )),
            Err(AgentdError::Config(_))
        ));
        assert_eq!(fs::read(manager.path()).unwrap(), original);
        assert!(
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn startup_multimodal_post_persist_activation_error_restores_before_clearing_intent() {
        let (_directory, manager) = manager();
        let original = b"# exact bytes\nauxiliary: {}\n";
        fs::write(manager.path(), original).unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-persist-error");
        desired.worker_api_key = Some("worker-secret".to_owned());
        manager.fail_next_atomic_write_after_persist();

        assert!(
            manager
                .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read(manager.path()).unwrap(), original);
        assert!(
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn startup_multimodal_removal_validation_failure_keeps_managed_profile_active() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-remove-fails");
        desired.worker_api_key = Some("worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        let managed_bytes = fs::read(manager.path()).unwrap();

        assert!(
            manager
                .deactivate_startup_multimodal_specialization(|| Err(AgentdError::Config(
                    "validation failed".to_owned()
                )))
                .is_err()
        );
        assert_eq!(fs::read(manager.path()).unwrap(), managed_bytes);
        assert!(
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn startup_multimodal_post_persist_removal_error_restores_active_state() {
        let (_directory, manager) = manager();
        let mut desired = MultimodalSpecializationDesiredStateV1::canonical(
            "runtime-multimodal-remove-persist-error",
        );
        desired.worker_api_key = Some("worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        let managed_bytes = fs::read(manager.path()).unwrap();
        manager.fail_next_atomic_write_after_persist();

        assert!(
            manager
                .deactivate_startup_multimodal_specialization(|| Ok(()))
                .is_err()
        );
        assert_eq!(fs::read(manager.path()).unwrap(), managed_bytes);
        assert_eq!(
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap()
                .phase,
            "active"
        );
    }

    #[test]
    fn startup_multimodal_removal_and_reactivation_are_idempotent() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-cycle");
        desired.worker_api_key = Some("worker-secret".to_owned());
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(
            manager
                .deactivate_startup_multimodal_specialization(|| Ok(()))
                .unwrap()
        );
        assert!(
            !manager
                .deactivate_startup_multimodal_specialization(|| Ok(()))
                .unwrap()
        );
        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert!(
            manager
                .startup_multimodal_specialization_matches(&desired)
                .unwrap()
        );
    }

    #[test]
    fn startup_multimodal_clamps_a_legacy_world_readable_config_to_owner_only() {
        let (_directory, manager) = manager();
        fs::set_permissions(manager.path(), fs::Permissions::from_mode(0o644)).unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-private-mode");
        desired.worker_api_key = Some("worker-secret".to_owned());

        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        assert_eq!(fs::metadata(manager.path()).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn startup_multimodal_fails_closed_when_the_authoritative_writer_lock_is_contended() {
        let (_directory, manager) = manager();
        let lock_path = manager.path().with_extension("yaml.finite-agentd.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        lock_file.lock().unwrap();
        let desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-lock-timeout");

        let started = Instant::now();
        let error = manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap_err();

        assert!(matches!(error, AgentdError::ConfigConflict(_)));
        assert!(started.elapsed() < Duration::from_secs(2));
        lock_file.unlock().unwrap();
    }

    fn stage_startup_activation_intent(
        manager: &ConfigManager,
        desired: &MultimodalSpecializationDesiredStateV1,
    ) -> (Vec<u8>, Vec<u8>) {
        let (before_bytes, mut document) = manager.load_document().unwrap();
        let vision_before = value_at_path(&document, VISION_CONFIG_PATH).cloned();
        let target = multimodal_specialization_provider_target(
            desired,
            vision_before.as_ref().unwrap_or(&Value::Null),
        )
        .unwrap();
        set_value_at_path(&mut document, VISION_CONFIG_PATH, target.clone()).unwrap();
        let additions = ensure_finitechat_video_toolset(&mut document).unwrap();
        let applied_bytes = serde_yaml::to_string(&document).unwrap().into_bytes();
        manager
            .ledger
            .begin_startup_specialization_apply(&StartupSpecializationApplyIntent {
                proposal_id: &desired.proposal_id,
                before_bytes: &before_bytes,
                transition_before_bytes: &before_bytes,
                vision_before: vision_before.as_ref(),
                video_toolset_added: additions.video,
                exact_restore_eligible: true,
                applied_bytes: &applied_bytes,
                vision_applied_hash: &value_hash(&target).unwrap(),
            })
            .unwrap();
        (before_bytes, applied_bytes)
    }

    #[test]
    fn startup_multimodal_recovers_activation_crashes_on_both_sides_of_config_write() {
        for crash_after_write in [false, true] {
            let (_directory, manager) = manager();
            let mut desired =
                MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-apply-crash");
            desired.worker_api_key = Some("worker-secret".to_owned());
            let (_before, applied) = stage_startup_activation_intent(&manager, &desired);
            if crash_after_write {
                manager.atomic_write(&applied).unwrap();
            }

            manager
                .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
                .unwrap();

            assert_eq!(
                manager
                    .ledger
                    .startup_specialization_state()
                    .unwrap()
                    .unwrap()
                    .phase,
                "active"
            );
            assert!(
                manager
                    .startup_multimodal_specialization_matches(&desired)
                    .unwrap()
            );
        }
    }

    #[test]
    fn startup_multimodal_interrupted_apply_preserves_concurrent_unrelated_edits() {
        for crash_after_write in [false, true] {
            let (_directory, manager) = manager();
            let mut desired =
                MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-apply-edit");
            desired.worker_api_key = Some("worker-secret".to_owned());
            let (_before, applied) = stage_startup_activation_intent(&manager, &desired);
            if crash_after_write {
                manager.atomic_write(&applied).unwrap();
            }
            let (_bytes, mut document) = manager.load_document().unwrap();
            set_value_at_path(&mut document, "unrelated.theme", json!("preserved")).unwrap();
            manager
                .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
                .unwrap();

            manager
                .recover_startup_specialization_transition(&mut || Ok(()))
                .unwrap();

            assert_eq!(
                manager.current_value("unrelated.theme").unwrap(),
                "preserved"
            );
            let state = manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap();
            assert_eq!(state.phase, "active");
            assert!(!state.exact_restore_eligible);
            assert!(
                manager
                    .startup_multimodal_specialization_matches(&desired)
                    .unwrap()
            );
        }
    }

    #[test]
    fn startup_validation_rollback_refuses_to_overwrite_external_edit() {
        let (_directory, manager) = manager();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-validation-race");
        desired.worker_api_key = Some("worker-secret".to_owned());

        assert!(
            manager
                .activate_platform_managed_multimodal_specialization(&desired, || {
                    let (_bytes, mut document) = manager.load_document()?;
                    set_value_at_path(&mut document, "unrelated.theme", json!("preserved"))?;
                    manager.atomic_write(serde_yaml::to_string(&document)?.as_bytes())?;
                    Err(AgentdError::Config(
                        "injected validation failure".to_owned(),
                    ))
                })
                .is_err()
        );
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            "preserved"
        );
        assert_eq!(
            manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap()
                .phase,
            "applying"
        );

        manager
            .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
            .unwrap();
        assert_eq!(
            manager.current_value("unrelated.theme").unwrap(),
            "preserved"
        );
    }

    #[test]
    fn startup_multimodal_recovers_removal_crashes_on_both_sides_of_config_write() {
        for crash_after_write in [false, true] {
            let (_directory, manager) = manager();
            let original =
                b"# exact bytes\nauxiliary: {vision: {provider: custom, model: user-original}}\n";
            fs::write(manager.path(), original).unwrap();
            let mut desired = MultimodalSpecializationDesiredStateV1::canonical(
                "runtime-multimodal-remove-crash",
            );
            desired.worker_api_key = Some("worker-secret".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
                .unwrap();
            let managed_bytes = fs::read(manager.path()).unwrap();
            manager
                .ledger
                .begin_startup_specialization_removal(&managed_bytes)
                .unwrap();
            if crash_after_write {
                manager.atomic_write(original).unwrap();
            }

            assert!(
                !manager
                    .deactivate_startup_multimodal_specialization(|| Ok(()))
                    .unwrap()
            );
            assert_eq!(fs::read(manager.path()).unwrap(), original);
            assert!(
                manager
                    .ledger
                    .startup_specialization_state()
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn startup_multimodal_failed_removal_recovery_restores_managed_bytes_on_both_crash_sides() {
        for crash_after_write in [false, true] {
            let (_directory, manager) = manager();
            let original = b"auxiliary: {vision: {provider: custom, model: user-original}}\n";
            fs::write(manager.path(), original).unwrap();
            let mut desired = MultimodalSpecializationDesiredStateV1::canonical(
                "runtime-multimodal-remove-invalid",
            );
            desired.worker_api_key = Some("worker-secret".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&desired, || Ok(()))
                .unwrap();
            let managed_bytes = fs::read(manager.path()).unwrap();
            manager
                .ledger
                .begin_startup_specialization_removal(&managed_bytes)
                .unwrap();
            if crash_after_write {
                manager.atomic_write(original).unwrap();
            }
            let (_bytes, mut interrupted_document) = manager.load_document().unwrap();
            set_value_at_path(
                &mut interrupted_document,
                "unrelated.theme",
                json!("preserved"),
            )
            .unwrap();
            manager
                .atomic_write(
                    serde_yaml::to_string(&interrupted_document)
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap();
            let managed_document = serde_yaml::from_slice::<Value>(&managed_bytes).unwrap();
            copy_specialization_owned_fields(&mut interrupted_document, &managed_document, true)
                .unwrap();
            let expected_active_bytes = serde_yaml::to_string(&interrupted_document)
                .unwrap()
                .into_bytes();

            assert!(
                manager
                    .deactivate_startup_multimodal_specialization(|| Err(AgentdError::Config(
                        "restored config is invalid".to_owned()
                    )))
                    .is_err()
            );
            assert_eq!(fs::read(manager.path()).unwrap(), expected_active_bytes);
            assert_eq!(
                manager
                    .ledger
                    .startup_specialization_state()
                    .unwrap()
                    .unwrap()
                    .phase,
                "active"
            );
        }
    }

    #[test]
    fn startup_multimodal_recovers_semantic_rollback_at_every_commit_boundary() {
        for crash_point in [
            "before_config_write",
            "before_ledger_commit",
            "after_ledger_commit",
        ] {
            let (_directory, manager) = manager();
            let mut first =
                MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-rollback-a");
            first.worker_api_key = Some("worker-secret-a".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
                .unwrap();
            manager
                .confirm_startup_multimodal_specialization_semantics()
                .unwrap();

            let mut previous_bytes = fs::read(manager.path()).unwrap();
            previous_bytes.extend_from_slice(b"unrelated_after_generation_a: preserved\n");
            fs::write(manager.path(), &previous_bytes).unwrap();

            let mut rotated =
                MultimodalSpecializationDesiredStateV1::canonical("runtime-multimodal-rollback-b");
            rotated.worker_api_key = Some("worker-secret-b".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
                .unwrap();
            manager
                .ledger
                .begin_unverified_startup_specialization_rollback()
                .unwrap();
            if crash_point != "before_config_write" {
                manager.atomic_write(&previous_bytes).unwrap();
            }
            if crash_point == "after_ledger_commit" {
                manager
                    .ledger
                    .finish_unverified_startup_specialization_rollback(&previous_bytes)
                    .unwrap();
            }

            manager
                .recover_startup_specialization_transition(&mut || Ok(()))
                .unwrap();

            assert_eq!(fs::read(manager.path()).unwrap(), previous_bytes);
            let state = manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap();
            assert_eq!(state.phase, "active");
            assert!(state.previous_applied_bytes.is_none());
            assert!(
                manager
                    .startup_multimodal_specialization_matches(&first)
                    .unwrap()
            );
            assert!(
                !manager
                    .startup_multimodal_specialization_matches(&rotated)
                    .unwrap()
            );
        }
    }

    #[test]
    fn startup_multimodal_interrupted_semantic_rollback_preserves_concurrent_unrelated_edits() {
        for crash_after_write in [false, true] {
            let (_directory, manager) = manager();
            let mut first = MultimodalSpecializationDesiredStateV1::canonical(
                "runtime-multimodal-rollback-edit-a",
            );
            first.worker_api_key = Some("worker-secret-a".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&first, || Ok(()))
                .unwrap();
            manager
                .confirm_startup_multimodal_specialization_semantics()
                .unwrap();
            let previous_bytes = fs::read(manager.path()).unwrap();

            let mut rotated = MultimodalSpecializationDesiredStateV1::canonical(
                "runtime-multimodal-rollback-edit-b",
            );
            rotated.worker_api_key = Some("worker-secret-b".to_owned());
            manager
                .activate_platform_managed_multimodal_specialization(&rotated, || Ok(()))
                .unwrap();
            manager
                .ledger
                .begin_unverified_startup_specialization_rollback()
                .unwrap();
            if crash_after_write {
                manager.atomic_write(&previous_bytes).unwrap();
            }
            let (_bytes, mut document) = manager.load_document().unwrap();
            set_value_at_path(&mut document, "unrelated.theme", json!("preserved")).unwrap();
            manager
                .atomic_write(serde_yaml::to_string(&document).unwrap().as_bytes())
                .unwrap();

            manager
                .recover_startup_specialization_transition(&mut || Ok(()))
                .unwrap();

            assert_eq!(
                manager.current_value("unrelated.theme").unwrap(),
                "preserved"
            );
            let state = manager
                .ledger
                .startup_specialization_state()
                .unwrap()
                .unwrap();
            assert_eq!(state.phase, "active");
            assert!(!state.exact_restore_eligible);
            assert!(
                manager
                    .startup_multimodal_specialization_matches(&first)
                    .unwrap()
            );
        }
    }

    #[test]
    fn multimodal_image_capability_can_be_disabled_independently() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n    api_key: worker-secret\n",
        )
        .unwrap();
        let mut desired =
            MultimodalSpecializationDesiredStateV1::canonical("multimodal-image-disabled");
        desired.capabilities.image = false;

        manager
            .reconcile_multimodal_specialization(&desired, || Ok(()))
            .unwrap();

        assert!(manager.multimodal_specialization_matches(&desired).unwrap());
        let effective = manager.current_value(VISION_CONFIG_PATH).unwrap();
        assert_eq!(
            effective.pointer("/extra_body/finite_specialization/capabilities/image"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn multimodal_specialization_activation_failure_restores_exact_previous_bytes() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "model: anthropic/claude\nauxiliary:\n  vision:\n    provider: custom\n    model: qwopus-old\n    api_key: worker-secret\n",
        )
        .unwrap();
        let before = fs::read(manager.path()).unwrap();
        let desired =
            MultimodalSpecializationDesiredStateV1::canonical("multimodal-reconcile-fails");

        let error = manager
            .reconcile_multimodal_specialization(&desired, || {
                Err(AgentdError::Supervisor("Hermes reload failed".to_owned()))
            })
            .unwrap_err();

        assert!(matches!(error, AgentdError::Supervisor(_)));
        assert_eq!(fs::read(manager.path()).unwrap(), before);
        assert!(
            manager
                .ledger
                .config_history("multimodal-reconcile-fails")
                .unwrap()
                .is_none()
        );
    }
}
