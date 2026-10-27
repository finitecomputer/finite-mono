//! Private per-account key aliases. Every read and write is scoped to the one
//! verified viewer account; there is no cross-account or global read.

use super::*;
use crate::account_labels::{KeyAlias, MAX_ALIASES_PER_ACCOUNT};

#[derive(Debug)]
pub enum KeyAliasError {
    /// The verified WorkOS subject has no linked Core account. These routes
    /// never enroll an account.
    AccountNotLinked,
    /// Saving a new alias would exceed `MAX_ALIASES_PER_ACCOUNT`.
    LimitReached,
    Store(CoreError),
}

impl From<CoreError> for KeyAliasError {
    fn from(error: CoreError) -> Self {
        Self::Store(error)
    }
}

const LINKED_VIEWER_SQL: &str =
    "SELECT id FROM users WHERE workos_user_id = $1 AND link_status = 'linked'";

impl CoreStore {
    /// The viewer's aliases for exactly these canonical keys. Read-only; an
    /// unlinked viewer simply has none.
    pub async fn key_aliases_for_viewer(
        &self,
        workos_user_id: &str,
        public_keys_hex: &[String],
    ) -> CoreResult<Vec<KeyAlias>> {
        let mut client = self.connection().await?;
        let tx = client
            .build_transaction()
            .read_only(true)
            .start()
            .await
            .map_err(store_error)?;
        let rows = tx
            .query(
                "SELECT alias.public_key_hex, alias.alias,
                        core_rfc3339(alias.updated_at) AS updated_at
                 FROM account_key_aliases AS alias
                 JOIN users AS viewer ON viewer.id = alias.viewer_user_id
                 WHERE viewer.workos_user_id = $1 AND viewer.link_status = 'linked'
                   AND alias.public_key_hex = ANY($2)
                 ORDER BY alias.public_key_hex",
                &[&workos_user_id, &public_keys_hex],
            )
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(rows.iter().map(key_alias_from_row).collect())
    }

    /// Create or replace the viewer's alias for one canonical key. Inputs are
    /// already normalized by `account_labels`.
    #[tracing::instrument(skip(self, alias, now), fields(operation = "set_key_alias"))]
    pub async fn set_key_alias(
        &self,
        workos_user_id: &str,
        public_key_hex: &str,
        alias: &str,
        now: &str,
    ) -> Result<KeyAlias, KeyAliasError> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Row lock on the viewer only: serializes this account's alias
        // writes so the count limit holds under concurrency.
        let viewer_id: String = tx
            .query_opt(
                format!("{LINKED_VIEWER_SQL} FOR NO KEY UPDATE").as_str(),
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?
            .ok_or(KeyAliasError::AccountNotLinked)?
            .get("id");
        let existing = tx
            .query_opt(
                "SELECT 1 FROM account_key_aliases
                 WHERE viewer_user_id = $1 AND public_key_hex = $2",
                &[&viewer_id, &public_key_hex],
            )
            .await
            .map_err(store_error)?
            .is_some();
        if !existing {
            let count: i64 = tx
                .query_one(
                    "SELECT count(*) AS count FROM account_key_aliases WHERE viewer_user_id = $1",
                    &[&viewer_id],
                )
                .await
                .map_err(store_error)?
                .get("count");
            if count >= MAX_ALIASES_PER_ACCOUNT {
                return Err(KeyAliasError::LimitReached);
            }
        }
        let row = tx
            .query_one(
                "INSERT INTO account_key_aliases
                   (viewer_user_id, public_key_hex, alias, created_at, updated_at)
                 VALUES ($1, $2, $3, $4::text::timestamptz, $4::text::timestamptz)
                 ON CONFLICT (viewer_user_id, public_key_hex) DO UPDATE SET
                   alias = EXCLUDED.alias,
                   updated_at = EXCLUDED.updated_at
                 RETURNING public_key_hex, alias, core_rfc3339(updated_at) AS updated_at",
                &[&viewer_id, &public_key_hex, &alias, &now],
            )
            .await
            .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(key_alias_from_row(&row))
    }

    /// Remove the viewer's alias for one key. Returns whether one existed.
    #[tracing::instrument(skip(self), fields(operation = "clear_key_alias"))]
    pub async fn clear_key_alias(
        &self,
        workos_user_id: &str,
        public_key_hex: &str,
    ) -> Result<bool, KeyAliasError> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let viewer_id: String = tx
            .query_opt(
                format!("{LINKED_VIEWER_SQL} FOR NO KEY UPDATE").as_str(),
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?
            .ok_or(KeyAliasError::AccountNotLinked)?
            .get("id");
        let removed = tx
            .execute(
                "DELETE FROM account_key_aliases
                 WHERE viewer_user_id = $1 AND public_key_hex = $2",
                &[&viewer_id, &public_key_hex],
            )
            .await
            .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(removed == 1)
    }
}

fn key_alias_from_row(row: &Row) -> KeyAlias {
    KeyAlias::new(
        row.get("public_key_hex"),
        row.get("alias"),
        row.get("updated_at"),
    )
}
