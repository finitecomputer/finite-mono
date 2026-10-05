use super::*;
use crate::trials::{UpdateTrialCode, hash_trial_code, normalize_trial_code};

impl CoreStore {
    #[tracing::instrument(skip_all, fields(operator = actor, campaign = id))]
    pub async fn update_trial_code(
        &self,
        id: &str,
        input: UpdateTrialCode,
        actor: &str,
    ) -> CoreResult<()> {
        if actor.trim().is_empty() {
            return Err(CoreError::TrialUnavailable("Admin identity is required."));
        }
        let code = normalize_trial_code(&input.code)?;
        let hash = hash_trial_code(&code)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Same lock as reservations and capacity edits. Existing reservations
        // and redemptions belong to the campaign ID, independent of its code.
        let row = tx
            .query_opt(
                "SELECT code_revision FROM trial_campaigns WHERE id=$1 FOR UPDATE",
                &[&id],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::TrialUnavailable("Trial campaign not found."))?;
        let revision: i32 = row.get(0);
        if revision != input.expected_code_revision {
            return Err(CoreError::TrialUnavailable(
                "The code changed. Refresh before editing it.",
            ));
        }
        let next_revision = revision.checked_add(1).ok_or(CoreError::TrialUnavailable(
            "This code cannot be edited again.",
        ))?;
        tx.execute(
            "UPDATE trial_campaigns SET code=$2,code_hash=$3,code_revision=$4 WHERE id=$1",
            &[&id, &code, &hash, &next_revision],
        )
        .await
        .map_err(|error| {
            if error
                .as_db_error()
                .is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
            {
                CoreError::TrialUnavailable("Another campaign already uses this code.")
            } else {
                store_error(error)
            }
        })?;
        self.finish(tx).await
    }
}
