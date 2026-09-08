//! Read-only mailbox authority inventory. This intentionally has no mutation:
//! a Core login rename is not authority to rewrite Sites mailbox grants.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEmailPreflightRequest {
    pub old_email: String,
    pub new_email: String,
}

#[derive(Debug, Serialize)]
pub struct AccountEmailPreflight {
    pub source_email_principal_id: Option<String>,
    pub destination_email_principal_id: Option<String>,
    pub source_rows: BTreeMap<String, i64>,
    pub destination_rows: BTreeMap<String, i64>,
    pub owned_project_ids: Vec<String>,
    pub authorized_native_principal_ids: Vec<String>,
    pub requires_product_authority_review: bool,
}

impl Store {
    /// Opens an existing registry strictly read-only, without running migrations,
    /// reconciliation or token cleanup. Unknown schemas fail rather than guessing.
    pub fn account_email_preflight(
        path: &Path,
        request: &AccountEmailPreflightRequest,
    ) -> Result<AccountEmailPreflight, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        inspect(&conn, request)
    }
}

fn inspect(
    conn: &Connection,
    request: &AccountEmailPreflightRequest,
) -> Result<AccountEmailPreflight, StoreError> {
    for email in [&request.old_email, &request.new_email] {
        if email.is_empty()
            || email.len() > 320
            || !email.contains('@')
            || email.chars().any(char::is_whitespace)
            || email.to_lowercase() != *email
        {
            return Err(StoreError::Conflict(
                "email preflight requires normalized mailbox addresses",
            ));
        }
    }
    if request.old_email == request.new_email {
        return Err(StoreError::Conflict(
            "email preflight requires distinct addresses",
        ));
    }
    let tx = conn.unchecked_transaction()?;
    let principal = |email: &str| -> Result<Option<String>, StoreError> {
        Ok(tx
            .query_row(
                "SELECT id FROM sites_email_principals WHERE email=?1",
                [email],
                |r| r.get(0),
            )
            .optional()?)
    };
    let source_email_principal_id = principal(&request.old_email)?;
    let destination_email_principal_id = principal(&request.new_email)?;
    let counts = |email: &str| -> Result<BTreeMap<String, i64>, StoreError> {
        let mut result = BTreeMap::new();
        // Fixed schema identifiers, never caller-provided SQL. Token hashes,
        // addresses, keys and message data are deliberately absent from output.
        for table in [
            "sites_email_principals",
            "principals",
            "shares",
            "email_keys",
            "principal_email_links",
            "email_login_tokens",
            "login_tokens",
            "site_access_requests",
            "site_notification_outbox",
            "hosted_requester_assertions",
            "sites_legacy_email_resolutions",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE email=?1");
            result.insert(table.into(), tx.query_row(&sql, [email], |r| r.get(0))?);
        }
        Ok(result)
    };
    let source_rows = counts(&request.old_email)?;
    let destination_rows = counts(&request.new_email)?;
    let query_ids = |sql: &str| -> Result<Vec<String>, StoreError> {
        let mut statement = tx.prepare(sql)?;
        let rows = statement.query_map([&source_email_principal_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    };
    let owned_project_ids =
        query_ids("SELECT id FROM projects WHERE publisher_email_principal_id=?1 ORDER BY id")?;
    let authorized_native_principal_ids = query_ids(
        "SELECT native_principal_id FROM sites_authorized_keys WHERE email_principal_id=?1 AND revoked_at IS NULL ORDER BY native_principal_id",
    )?;
    let requires_product_authority_review = source_rows
        .values()
        .chain(destination_rows.values())
        .any(|count| *count > 0);
    tx.commit()?;
    Ok(AccountEmailPreflight {
        source_email_principal_id,
        destination_email_principal_id,
        source_rows,
        destination_rows,
        owned_project_ids,
        authorized_native_principal_ids,
        requires_product_authority_review,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_email_preflight_reads_existing_authority_without_migrating_or_consuming_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let store = Store::open(&path).unwrap();
        store.conn.execute("INSERT INTO sites_email_principals(id,email,verified_at,created_at,updated_at) VALUES ('sep_fixture','before@example.test',1,1,1)", []).unwrap();
        store.conn.execute("INSERT INTO email_login_tokens(token_hash,email,expires_at,created_at) VALUES (?1,'before@example.test',9999999999,1)", ["a".repeat(64)]).unwrap();
        drop(store);
        let before = std::fs::read(&path).unwrap();
        let request = AccountEmailPreflightRequest {
            old_email: "before@example.test".into(),
            new_email: "after@example.test".into(),
        };
        let result = Store::account_email_preflight(&path, &request).unwrap();
        assert_eq!(
            result.source_email_principal_id.as_deref(),
            Some("sep_fixture")
        );
        assert!(result.destination_email_principal_id.is_none());
        assert_eq!(result.source_rows["email_login_tokens"], 1);
        assert!(result.requires_product_authority_review);
        assert_eq!(before, std::fs::read(&path).unwrap());
        let missing = dir.path().join("absent.db");
        assert!(Store::account_email_preflight(&missing, &request).is_err());
        assert!(!missing.exists());
        let unknown = dir.path().join("unknown.db");
        Connection::open(&unknown).unwrap();
        let before = std::fs::read(&unknown).unwrap();
        assert!(Store::account_email_preflight(&unknown, &request).is_err());
        assert_eq!(before, std::fs::read(&unknown).unwrap());
    }
}
