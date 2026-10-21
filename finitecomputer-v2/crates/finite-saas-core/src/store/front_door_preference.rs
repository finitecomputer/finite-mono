//! The signed-in user's front-door preference: "use the new Finite by default".
//!
//! Stored as `users.migrated_at` (migration 0042). It is a routing preference
//! the user sets and clears for their own account, never a security boundary:
//! Core derives no lifecycle, billing, or recovery behavior from it. `/me`
//! reads it through the existing user lookup in `projects.rs`.
use super::*;

/// The caller's preference after a set or clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontDoorPreference {
    pub use_new_finite: bool,
    /// When the preference was first turned on; kept while it stays on.
    pub migrated_at: Option<String>,
}

impl CoreStore {
    /// Set (`true`) or clear (`false`) the preference on exactly this WorkOS
    /// user's row. Idempotent: turning it on keeps the first timestamp, and
    /// `updated_at` moves only when the value changes. Returns `None` for an
    /// account Core has not linked; no row is ever created here.
    #[tracing::instrument(skip(self, now), fields(operation = "set_front_door_preference"))]
    pub async fn set_front_door_preference(
        &self,
        workos_user_id: &str,
        use_new_finite: bool,
        now: Option<String>,
    ) -> CoreResult<Option<FrontDoorPreference>> {
        let workos_user_id = workos_user_id.trim();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let now = now.unwrap_or(current_time_iso()?);
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let row = tx
            .query_opt(
                "UPDATE users
                 SET migrated_at = CASE WHEN $2 THEN COALESCE(migrated_at, $3::text::timestamptz) END,
                     updated_at = CASE
                       WHEN (migrated_at IS NULL) = $2 THEN $3::text::timestamptz
                       ELSE updated_at
                     END
                 WHERE workos_user_id = $1
                 RETURNING core_rfc3339(migrated_at) AS migrated_at",
                &[&workos_user_id, &use_new_finite, &now],
            )
            .await
            .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(row.map(|row| {
            let migrated_at: Option<String> = row.get("migrated_at");
            FrontDoorPreference {
                use_new_finite: migrated_at.is_some(),
                migrated_at,
            }
        }))
    }
}
