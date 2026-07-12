//! Typed, read-only Phala account and inventory projections.
//!
//! The provider API has separate workspace, app, CVM, and revision surfaces.
//! Keep their wire schemas here so preflight can retain every row, join
//! locally, and emit only aggregate facts. Provider ids and account PII must
//! never enter the public preflight report. Metered usage remains deferred
//! until an official bounded query contract is pinned.

use crate::phala::{CvmInfo, FINITE_CVM_NAME_PREFIX};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error, Clone, Copy, Eq, PartialEq)]
pub enum InventoryContractError {
    #[error("Phala workspace identity did not match the configured fence")]
    WorkspaceMismatch,
    #[error("Phala workspace quota response was incomplete or insufficient")]
    InsufficientQuota,
    #[error("Phala app inventory contained an incomplete record")]
    IncompleteApp,
    #[error("Phala app and CVM inventory did not reconcile")]
    AppCvmMismatch,
    #[error("Phala app inventory did not match the private Cloud KMS policy")]
    AppPolicyMismatch,
    #[error("Phala revision inventory did not match its requested app")]
    RevisionMismatch,
    #[error("Phala revisions did not reconcile with the current Finite CVMs")]
    RevisionCvmMismatch,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct CurrentUserResponse {
    pub workspace: WorkspaceIdentity,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct WorkspaceIdentity {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub tier: String,
    pub role: String,
}

impl CurrentUserResponse {
    pub fn verify_workspace(
        &self,
        expected_id: &str,
        expected_slug: &str,
    ) -> Result<(), InventoryContractError> {
        if expected_id.trim().is_empty()
            || expected_slug.trim().is_empty()
            || self.workspace.id != expected_id
            || self.workspace.slug != expected_slug
        {
            return Err(InventoryContractError::WorkspaceMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct WorkspaceQuotas {
    pub team_slug: String,
    pub tier: String,
    pub quotas: QuotaSet,
    pub as_of: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct QuotaSet {
    pub vm_slots: QuotaValue,
    pub vcpu: QuotaValue,
    pub memory_mb: QuotaValue,
    pub disk_gb: QuotaValue,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct QuotaValue {
    pub limit: i64,
    pub remaining: i64,
}

impl WorkspaceQuotas {
    pub fn verify_capacity(
        &self,
        expected_slug: &str,
        vcpu: u32,
        memory_mb: u64,
        disk_gb: u32,
    ) -> Result<(), InventoryContractError> {
        let enough = |quota: &QuotaValue, required: u64| {
            quota.remaining == -1
                || u64::try_from(quota.remaining).is_ok_and(|remaining| remaining >= required)
        };
        if self.team_slug != expected_slug
            || !enough(&self.quotas.vm_slots, 1)
            || !enough(&self.quotas.vcpu, u64::from(vcpu))
            || !enough(&self.quotas.memory_mb, memory_mb)
            || !enough(&self.quotas.disk_gb, u64::from(disk_gb))
        {
            return Err(InventoryContractError::InsufficientQuota);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct AppsPage {
    pub dstack_apps: Vec<PhalaApp>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct PhalaApp {
    pub id: Option<String>,
    pub name: Option<String>,
    pub app_id: String,
    pub created_at: Option<String>,
    pub kms_type: Option<String>,
    pub app_provision_type: Option<String>,
    pub cvm_count: Option<u32>,
}

impl PhalaApp {
    fn validate_complete(&self) -> Result<(), InventoryContractError> {
        if self.app_id.trim().is_empty()
            || self
                .id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            || self
                .name
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            || self
                .created_at
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            || self
                .kms_type
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            || self.cvm_count.is_none()
        {
            return Err(InventoryContractError::IncompleteApp);
        }
        Ok(())
    }

    fn is_finite_named(&self) -> bool {
        self.name
            .as_deref()
            .is_some_and(|name| name.starts_with(FINITE_CVM_NAME_PREFIX))
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct RevisionsPage {
    pub revisions: Vec<AppRevision>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct AppRevision {
    pub revision_id: String,
    pub app_id: String,
    pub vm_uuid: Option<String>,
    pub compose_hash: Option<String>,
    pub created_at: String,
    pub operation_type: String,
}

impl RevisionsPage {
    pub fn verify_app(&self, app_id: &str) -> Result<(), InventoryContractError> {
        if self.revisions.iter().any(|revision| {
            revision.revision_id.trim().is_empty()
                || revision.app_id != app_id
                || revision.vm_uuid.as_deref().is_none_or(str::is_empty)
                || revision.compose_hash.as_deref().is_none_or(str::is_empty)
                || revision.created_at.trim().is_empty()
                || revision.operation_type.trim().is_empty()
        }) {
            return Err(InventoryContractError::RevisionMismatch);
        }
        Ok(())
    }
}

/// Provider-side projection only. Core ledger reconciliation is a separate
/// required input before admission; this type deliberately cannot claim it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FiniteProviderInventory {
    pub app_ids: BTreeSet<String>,
    pub cvm_count: u32,
}

impl FiniteProviderInventory {
    pub fn reconcile(apps: &[PhalaApp], cvms: &[CvmInfo]) -> Result<Self, InventoryContractError> {
        for app in apps {
            app.validate_complete()?;
        }
        let finite_cvms = cvms
            .iter()
            .filter(|cvm| cvm.name.starts_with(FINITE_CVM_NAME_PREFIX) && cvm.deleted_at.is_none())
            .collect::<Vec<_>>();
        let cvm_app_ids = finite_cvms
            .iter()
            .map(|cvm| {
                cvm.app_id
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .ok_or(InventoryContractError::AppCvmMismatch)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let app_ids = apps
            .iter()
            .filter(|app| app.is_finite_named() || cvm_app_ids.contains(app.app_id.as_str()))
            .map(|app| app.app_id.clone())
            .collect::<BTreeSet<_>>();
        if cvm_app_ids.iter().any(|app_id| !app_ids.contains(*app_id)) {
            return Err(InventoryContractError::AppCvmMismatch);
        }
        if apps
            .iter()
            .any(|app| app_ids.contains(&app.app_id) && app.kms_type.as_deref() != Some("phala"))
        {
            return Err(InventoryContractError::AppPolicyMismatch);
        }
        Ok(Self {
            app_ids,
            cvm_count: finite_cvms.len().try_into().unwrap_or(u32::MAX),
        })
    }

    pub fn billable_resource_count(&self) -> u32 {
        self.app_ids
            .len()
            .try_into()
            .unwrap_or(u32::MAX)
            .max(self.cvm_count)
    }

    pub fn reconcile_revisions(
        &self,
        cvms: &[CvmInfo],
        revisions: &[AppRevision],
    ) -> Result<(), InventoryContractError> {
        if revisions
            .iter()
            .any(|revision| !self.app_ids.contains(&revision.app_id))
        {
            return Err(InventoryContractError::RevisionMismatch);
        }
        for cvm in cvms
            .iter()
            .filter(|cvm| cvm.name.starts_with(FINITE_CVM_NAME_PREFIX) && cvm.deleted_at.is_none())
        {
            let app_id = cvm
                .app_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or(InventoryContractError::RevisionCvmMismatch)?;
            let compose_hash = cvm
                .compose_hash
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or(InventoryContractError::RevisionCvmMismatch)?;
            let vm_uuid = cvm
                .vm_uuid
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or(InventoryContractError::RevisionCvmMismatch)?;
            let matches_current_cvm = revisions.iter().any(|revision| {
                revision.app_id == app_id
                    && revision.compose_hash.as_deref() == Some(compose_hash)
                    && revision.vm_uuid.as_deref() == Some(vm_uuid)
            });
            if !matches_current_cvm {
                return Err(InventoryContractError::RevisionCvmMismatch);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cvms() -> Vec<CvmInfo> {
        serde_json::from_str::<serde_json::Value>(include_str!(
            "../tests/fixtures/phala/cvm-list.json"
        ))
        .unwrap()
        .get("items")
        .cloned()
        .map(serde_json::from_value)
        .unwrap()
        .unwrap()
    }

    #[test]
    fn workspace_and_quota_are_exactly_fenced() {
        let current: CurrentUserResponse =
            serde_json::from_str(include_str!("../tests/fixtures/phala/auth-me.json")).unwrap();
        current
            .verify_workspace("workspace_fixture_01", "finite-fixture")
            .unwrap();
        assert_eq!(
            current
                .verify_workspace("workspace_wrong", "finite-fixture")
                .unwrap_err(),
            InventoryContractError::WorkspaceMismatch
        );

        let quotas: WorkspaceQuotas = serde_json::from_str(include_str!(
            "../tests/fixtures/phala/workspace-quotas.json"
        ))
        .unwrap();
        quotas
            .verify_capacity("finite-fixture", 2, 4096, 40)
            .unwrap();
        let mut unlimited = quotas.clone();
        unlimited.quotas.vm_slots.remaining = -1;
        unlimited.quotas.vcpu.remaining = -1;
        unlimited.quotas.memory_mb.remaining = -1;
        unlimited.quotas.disk_gb.remaining = -1;
        unlimited
            .verify_capacity("finite-fixture", 2, 4096, 40)
            .unwrap();
        assert_eq!(
            quotas
                .verify_capacity("finite-fixture", 2, 32_768, 40)
                .unwrap_err(),
            InventoryContractError::InsufficientQuota
        );
    }

    #[test]
    fn provider_inventory_counts_stopped_non_deleted_cvms_and_rejects_partial_apps() {
        let apps: AppsPage =
            serde_json::from_str(include_str!("../tests/fixtures/phala/apps-list.json")).unwrap();
        let inventory = FiniteProviderInventory::reconcile(&apps.dstack_apps, &cvms()).unwrap();
        assert_eq!(
            inventory.app_ids,
            BTreeSet::from(["app_fixture_01".to_string()])
        );
        assert_eq!(inventory.cvm_count, 1);
        assert_eq!(inventory.billable_resource_count(), 1);

        let mut unclassified_provision = apps.dstack_apps.clone();
        unclassified_provision[0].app_provision_type = Some("provider-defined".to_string());
        FiniteProviderInventory::reconcile(&unclassified_provision, &cvms()).unwrap();

        let mut stopped = cvms()[0].clone();
        stopped.status = "terminated".to_string();
        assert_eq!(
            FiniteProviderInventory::reconcile(&apps.dstack_apps, &[stopped])
                .unwrap()
                .cvm_count,
            1
        );

        let mut incomplete = apps.dstack_apps;
        incomplete[0].name = None;
        assert_eq!(
            FiniteProviderInventory::reconcile(&incomplete, &cvms()).unwrap_err(),
            InventoryContractError::IncompleteApp
        );

        let mut wrong_kms = incomplete;
        wrong_kms[0].name = Some("finite-agent-fixture-01".to_string());
        wrong_kms[0].kms_type = Some("ethereum".to_string());
        assert_eq!(
            FiniteProviderInventory::reconcile(&wrong_kms, &cvms()).unwrap_err(),
            InventoryContractError::AppPolicyMismatch
        );
    }

    #[test]
    fn revisions_are_typed_and_reconcile_with_the_current_cvm() {
        let revisions: RevisionsPage =
            serde_json::from_str(include_str!("../tests/fixtures/phala/app-revisions.json"))
                .unwrap();
        revisions.verify_app("app_fixture_01").unwrap();
        let apps: AppsPage =
            serde_json::from_str(include_str!("../tests/fixtures/phala/apps-list.json")).unwrap();
        FiniteProviderInventory::reconcile(&apps.dstack_apps, &cvms())
            .unwrap()
            .reconcile_revisions(&cvms(), &revisions.revisions)
            .unwrap();
        assert_eq!(
            revisions.verify_app("app_wrong").unwrap_err(),
            InventoryContractError::RevisionMismatch
        );
    }
}
