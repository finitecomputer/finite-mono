//! A test owner-identity backend on Core's WorkOS test registry, so a user
//! the operator path creates is the same user a later test sign-in uses.

use super::*;
use crate::auth::test_support::{registry_put_user, registry_user, registry_users_by_email};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT_USER: AtomicU64 = AtomicU64::new(1);

/// The hosted device keys each chat identity by WorkOS user id and keeps it
/// on disk, so one WorkOS user always has one chat account. Model that as a
/// pure function of the id.
pub(crate) fn test_chat_account_id(workos_user_id: &str) -> String {
    Sha256::digest(format!("hosted-chat:{workos_user_id}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Default)]
pub(crate) struct TestOwnerIdentityBackend {
    pub(crate) created_users: Mutex<Vec<String>>,
    pub(crate) chat_reads: Mutex<Vec<String>>,
    pub(crate) operators: Mutex<BTreeSet<String>>,
    pub(crate) hosted_unavailable: AtomicBool,
}

impl TestOwnerIdentityBackend {
    pub(crate) fn created(&self) -> Vec<String> {
        self.created_users.lock().unwrap().clone()
    }
}

pub(crate) fn test_provisioner() -> (OwnerIdentityProvisioner, Arc<TestOwnerIdentityBackend>) {
    let backend = Arc::new(TestOwnerIdentityBackend::default());
    (
        OwnerIdentityProvisioner::new(backend.clone() as Arc<dyn OwnerIdentityBackend>),
        backend,
    )
}

impl OwnerIdentityBackend for TestOwnerIdentityBackend {
    fn workos_users_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<Vec<WorkosUserRecord>>> {
        Box::pin(async move { Ok(registry_users_by_email(email)) })
    }

    fn create_workos_user<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<WorkosUserRecord>> {
        Box::pin(async move {
            // WorkOS keeps one user per email in an environment.
            if !registry_users_by_email(email).is_empty() {
                return Err(CoreError::OwnerIdentityUnavailable(
                    "WorkOS user creation returned HTTP 422".into(),
                ));
            }
            let id = format!(
                "user_op_{}_{}",
                std::process::id(),
                NEXT_USER.fetch_add(1, Ordering::SeqCst)
            );
            registry_put_user(&id, email, false);
            self.created_users.lock().unwrap().push(id.clone());
            Ok(registry_user(&id).unwrap())
        })
    }

    fn workos_user<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, CoreResult<Option<WorkosUserRecord>>> {
        Box::pin(async move { Ok(registry_user(id)) })
    }

    fn operator_membership_active<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<bool>> {
        Box::pin(async move { Ok(self.operators.lock().unwrap().contains(workos_user_id)) })
    }

    fn hosted_chat_account_id<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<String>> {
        Box::pin(async move {
            if self.hosted_unavailable.load(Ordering::SeqCst) {
                return Err(CoreError::OwnerIdentityUnavailable(
                    "hosted chat is unreachable".into(),
                ));
            }
            self.chat_reads
                .lock()
                .unwrap()
                .push(workos_user_id.to_string());
            Ok(test_chat_account_id(workos_user_id))
        })
    }
}
