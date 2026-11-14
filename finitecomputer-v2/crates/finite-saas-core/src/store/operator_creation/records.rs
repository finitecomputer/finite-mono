//! Reading and claiming `operator_agent_creations` rows.

use super::*;
use crate::{parse_operator_creation_admission, parse_operator_creation_surface};

pub(super) const RECORD_COLUMNS: &str = "o.id, o.idempotency_key, o.owner_email,
    o.owner_user_id, o.owner_account_created, o.customer_org_id, o.customer_org_created,
    o.owner_workos_user_id, o.owner_chat_account_id, o.existing_account_confirmed,
    o.project_id, o.agent_creation_request_id, o.display_name, o.hosting_tier,
    o.placement_runner_class, o.runtime_resource_class, o.target_source_host_id,
    o.admission, o.reason, o.admin_user_id, o.admin_email, o.admin_workos_user_id,
    o.surface, o.cli_os_user, core_rfc3339(o.created_at) AS created_at";

pub(super) async fn select_record_by_key<C>(
    client: &C,
    key: &str,
) -> CoreResult<Option<OperatorAgentCreation>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            &format!(
                "SELECT {RECORD_COLUMNS} FROM operator_agent_creations o WHERE o.idempotency_key = $1"
            ),
            &[&key],
        )
        .await
        .map_err(store_error)?
        .map(|row| record_from_row(&row))
        .transpose()
}

/// Insert the record unless its idempotency key exists; true when inserted.
pub(super) async fn claim_record<C>(client: &C, record: &OperatorAgentCreation) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    let owner = &record.owner;
    let actor = &record.actor;
    client
        .execute(
            "INSERT INTO operator_agent_creations
               (id, idempotency_key, owner_email, owner_user_id, owner_account_created,
                customer_org_id, customer_org_created, owner_workos_user_id, owner_chat_account_id,
                existing_account_confirmed, project_id, agent_creation_request_id, display_name,
                hosting_tier, placement_runner_class, runtime_resource_class, target_source_host_id,
                admission, reason, admin_user_id, admin_email, admin_workos_user_id, surface,
                cli_os_user, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18, $19, $20, $21, $22, $23, $24, $25::text::timestamptz)
             ON CONFLICT (idempotency_key) DO NOTHING",
            &[
                &record.id,
                &record.idempotency_key,
                &owner.email,
                &owner.user_id,
                &owner.account_created,
                &owner.customer_org_id,
                &owner.customer_org_created,
                &owner.workos_user_id,
                &owner.chat_account_id,
                &owner.existing_account_confirmed,
                &record.project_id,
                &record.agent_creation_request_id,
                &record.display_name,
                &record.hosting_tier.as_str(),
                &record.placement.runner_class.as_str(),
                &record.placement.runtime_resource_class.as_str(),
                &record.target_source_host_id,
                &record.admission.as_str(),
                &record.reason,
                &actor.admin_user_id,
                &actor.admin.email,
                &actor.admin.workos_user_id,
                &actor.surface.as_str(),
                &actor.cli_os_user,
                &record.created_at,
            ],
        )
        .await
        .map_err(store_error)
        .map(|inserted| inserted == 1)
}

pub(super) fn record_from_row(row: &Row) -> CoreResult<OperatorAgentCreation> {
    let invalid = |column: &str, value: &str| {
        CoreError::Store(format!("invalid operator creation {column} {value}"))
    };
    let text = |column: &str| -> String { row.get(column) };
    let tier = text("hosting_tier");
    let runner_class = text("placement_runner_class");
    let resource_class = text("runtime_resource_class");
    let admission = text("admission");
    let surface = text("surface");
    Ok(OperatorAgentCreation {
        id: text("id"),
        idempotency_key: text("idempotency_key"),
        owner: OperatorCreationOwner {
            email: text("owner_email"),
            user_id: text("owner_user_id"),
            account_created: row.get("owner_account_created"),
            customer_org_id: text("customer_org_id"),
            customer_org_created: row.get("customer_org_created"),
            workos_user_id: text("owner_workos_user_id"),
            chat_account_id: text("owner_chat_account_id"),
            existing_account_confirmed: row.get("existing_account_confirmed"),
        },
        project_id: text("project_id"),
        agent_creation_request_id: text("agent_creation_request_id"),
        display_name: text("display_name"),
        hosting_tier: parse_hosting_tier(&tier).ok_or_else(|| invalid("tier", &tier))?,
        placement: RuntimePlacement {
            runner_class: parse_runner_class(&runner_class)
                .ok_or_else(|| invalid("runner class", &runner_class))?,
            runtime_resource_class: parse_runtime_resource_class(&resource_class)
                .ok_or_else(|| invalid("resource class", &resource_class))?,
        },
        target_source_host_id: row.get("target_source_host_id"),
        admission: parse_operator_creation_admission(&admission)
            .ok_or_else(|| invalid("admission", &admission))?,
        reason: text("reason"),
        actor: OperatorCreationActor {
            admin_user_id: text("admin_user_id"),
            admin: OperatorIdentity {
                email: text("admin_email"),
                workos_user_id: text("admin_workos_user_id"),
            },
            surface: parse_operator_creation_surface(&surface)
                .ok_or_else(|| invalid("surface", &surface))?,
            cli_os_user: row.get("cli_os_user"),
        },
        created_at: text("created_at"),
    })
}

pub(super) fn listing_from_row(row: &Row) -> CoreResult<OperatorAgentCreationListing> {
    let record = record_from_row(row)?;
    let current_workos: Option<String> = row.get("owner_current_workos_user_id");
    let linked = row.get::<_, String>("owner_link_status") == "linked";
    Ok(OperatorAgentCreationListing {
        signed_in_as_provisioned_user: linked
            && current_workos.as_deref() == Some(record.owner.workos_user_id.as_str()),
        owner_current_email: row.get("owner_current_email"),
        owner_linked: linked,
        owner_current_workos_user_id: current_workos,
        record,
    })
}
