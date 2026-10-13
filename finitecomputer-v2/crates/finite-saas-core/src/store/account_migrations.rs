//! Account migration marker for the new Finite product (finite.chat).
//!
//! The import flow records, once, that an account moved. Core never derives
//! lifecycle, billing, or recovery behavior from the marker; the legacy
//! dashboard's front door reads it through `/me` (folded into the user lookup
//! in `projects.rs`) to stop offering the legacy surface to that account.
//! There is deliberately no unset path here.
use super::*;

/// Outcome of an idempotent migration mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMigrationMark {
    pub workos_user_id: String,
    /// The first recorded mark; replays return it unchanged.
    pub migrated_at: String,
    /// False when an earlier call had already recorded the migration.
    pub newly_marked: bool,
}

impl CoreStore {
    /// Record that a linked WorkOS account moved to the new product.
    ///
    /// Idempotent: the first mark wins and later calls return it unchanged.
    /// Returns `None` for an account Core has never linked; the caller must
    /// not invent a user row for it.
    #[tracing::instrument(skip(self, now), fields(operation = "mark_account_migrated"))]
    pub async fn mark_account_migrated(
        &self,
        workos_user_id: &str,
        now: Option<String>,
    ) -> CoreResult<Option<AccountMigrationMark>> {
        let workos_user_id = workos_user_id.trim();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let now = now.unwrap_or(current_time_iso()?);
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let Some(row) = tx
            .query_opt(
                "SELECT id, core_rfc3339(migrated_at) AS migrated_at
                 FROM users WHERE workos_user_id = $1 FOR UPDATE",
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?
        else {
            self.finish(tx).await?;
            return Ok(None);
        };
        let user_id: String = row.get("id");
        let (migrated_at, newly_marked) = match row.get::<_, Option<String>>("migrated_at") {
            Some(existing) => (existing, false),
            None => {
                let marked = tx
                    .query_one(
                        "UPDATE users
                         SET migrated_at = $2::text::timestamptz,
                             updated_at = $2::text::timestamptz
                         WHERE id = $1 AND migrated_at IS NULL
                         RETURNING core_rfc3339(migrated_at) AS migrated_at",
                        &[&user_id, &now],
                    )
                    .await
                    .map_err(store_error)?;
                (marked.get("migrated_at"), true)
            }
        };
        self.finish(tx).await?;
        Ok(Some(AccountMigrationMark {
            workos_user_id: workos_user_id.to_string(),
            migrated_at,
            newly_marked,
        }))
    }
}
