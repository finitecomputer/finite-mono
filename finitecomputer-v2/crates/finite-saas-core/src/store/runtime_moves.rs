//! FIN-463: marking an exact Runtime as moved to finite.chat, so the mover's
//! roster never offers it again (`agent_runtime_moves`, migration 0044).

use super::*;

impl CoreStore {
    /// Mark one exact Runtime `migrated` or `archived`. Repeating the same
    /// mark returns the stored receipt unchanged; `archived` may become
    /// `migrated`; any other change is refused.
    pub async fn admin_mark_runtime_moved_exact(
        &self,
        input: AdminMarkRuntimeMovedExactInput,
    ) -> CoreResult<RuntimeMoveReceipt> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_mark_runtime_moved_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    /// Whether this person's agents have all moved to finite.chat, so the
    /// dashboard sends them there: at least one agent they belong to was
    /// marked moved (even if since offboarded, which archives their
    /// membership), and no agent they can still see runs unmoved.
    pub async fn moved_to_finite_chat_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<bool> {
        let client = self.connection().await?;
        let row = client
            .query_one(
                "WITH belongs AS (
                   SELECT membership.project_id, membership.archived_at
                   FROM users AS person
                   JOIN chat_identities AS identity ON identity.user_id = person.id
                   JOIN project_room_memberships AS membership
                     ON membership.chat_identity_id = identity.id
                   WHERE person.workos_user_id = $1
                 )
                 SELECT EXISTS (
                          SELECT 1 FROM belongs
                          JOIN agent_runtime_moves AS moved ON moved.project_id = belongs.project_id
                        )
                    AND NOT EXISTS (
                          SELECT 1 FROM belongs
                          JOIN projects AS project ON project.id = belongs.project_id
                          JOIN project_runtime_links AS link
                            ON link.project_id = belongs.project_id AND link.active
                          WHERE belongs.archived_at IS NULL
                            AND project.import_candidate_id IS NULL
                            AND NOT EXISTS (
                              SELECT 1 FROM agent_runtime_moves AS moved
                              WHERE moved.agent_runtime_id = link.agent_runtime_id
                            )
                        ) AS moved",
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?;
        Ok(row.get("moved"))
    }
}

async fn postgres_admin_mark_runtime_moved_exact<C>(
    client: &C,
    input: AdminMarkRuntimeMovedExactInput,
) -> CoreResult<RuntimeMoveReceipt>
where
    C: GenericClient + Sync,
{
    let agent_npub = match (input.moved, input.agent_npub.as_deref().map(str::trim)) {
        (RuntimeMoved::Migrated, Some(npub)) if canonical_npub(npub) => Some(npub.to_string()),
        (RuntimeMoved::Archived, None) => None,
        _ => return Err(CoreError::InvalidRuntimeMoveNpub),
    };
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    // The Runtime itself, linked or not: the mover marks a Runtime after
    // it is stopped, and possibly after it is retired and offboarded.
    let runtime = client
        .query_opt(
            "SELECT id, source_host_id, source_machine_id
             FROM agent_runtimes
             WHERE id = $1 AND project_id = $2
             FOR UPDATE",
            &[&input.expected_agent_runtime_id, &input.project_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let source_host_id: String = runtime.get("source_host_id");
    let source_machine_id: String = runtime.get("source_machine_id");
    if source_host_id != input.expected_source_host_id
        || source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    let existing = client
        .query_opt(
            "SELECT moved, core_rfc3339(moved_at) AS moved_at, agent_npub
             FROM agent_runtime_moves WHERE agent_runtime_id = $1",
            &[&input.expected_agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    if let Some(row) = existing {
        let moved: String = row.get("moved");
        let stored = RuntimeMoveReceipt {
            project_id: input.project_id.clone(),
            agent_runtime_id: input.expected_agent_runtime_id.clone(),
            moved: RuntimeMoved::parse(&moved).ok_or(CoreError::RuntimeMoveConflict)?,
            moved_at: row.get("moved_at"),
            agent_npub: row.get("agent_npub"),
        };
        if stored.moved == input.moved && stored.agent_npub == agent_npub {
            return Ok(stored);
        }
        if !(stored.moved == RuntimeMoved::Archived && input.moved == RuntimeMoved::Migrated) {
            return Err(CoreError::RuntimeMoveConflict);
        }
    }
    let row = client
        .query_one(
            "INSERT INTO agent_runtime_moves (agent_runtime_id, project_id, moved, moved_at, agent_npub)
             VALUES ($1, $2, $3, $4::text::timestamptz, $5)
             ON CONFLICT (agent_runtime_id) DO UPDATE
               SET moved = EXCLUDED.moved, moved_at = EXCLUDED.moved_at,
                   agent_npub = EXCLUDED.agent_npub
             RETURNING core_rfc3339(moved_at) AS moved_at",
            &[
                &input.expected_agent_runtime_id,
                &input.project_id,
                &input.moved.as_str(),
                &now,
                &agent_npub,
            ],
        )
        .await
        .map_err(store_error)?;
    let receipt = RuntimeMoveReceipt {
        project_id: input.project_id,
        agent_runtime_id: input.expected_agent_runtime_id,
        moved: input.moved,
        moved_at: row.get("moved_at"),
        agent_npub,
    };
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_mark_moved",
            target_type: "agent_runtime",
            target_id: &receipt.agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": receipt.project_id.clone(),
                "moved": receipt.moved.as_str(),
                "agentNpub": receipt.agent_npub.clone(),
            }),
            now: &now,
        },
    )
    .await?;
    Ok(receipt)
}

/// A bech32 `npub` of a 32-byte key, in the canonical lowercase form that
/// re-encodes to itself.
fn canonical_npub(value: &str) -> bool {
    let Ok((hrp, bytes)) = bech32::decode(value) else {
        return false;
    };
    hrp.as_str() == "npub"
        && bytes.len() == 32
        && bech32::encode::<bech32::Bech32>(hrp, &bytes).is_ok_and(|encoded| encoded == value)
}
