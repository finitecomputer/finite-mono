//! FIN-463's hold: a Runtime the finite.chat mover stopped for a move is
//! started by nothing but the mover's own `runtime-restart-exact` (staff gave
//! it back), and a Runtime marked moved by nothing at all
//! (`agent_runtime_move_holds`, migration 0046). Stop and destroy stay
//! allowed.
//!
//! A Runner starts compute only for a start Core queued: a restart,
//! recovery or upgrade, or a relocation. So the rule has two halves, each
//! under the Runtime's row lock:
//!
//! - no start is queued while it is held ([`refuse_a_held_start`], where
//!   every start is queued: the control enqueue and the relocation insert);
//! - no hold or moved mark is taken while a start is queued or running
//!   ([`refuse_while_a_start_is_active`]).
//!
//! So a held Runtime never has a start a Runner could lease.
//!
//! Delete with the table, when every old agent is moved or declined.

use super::*;

/// Refuse to start `runtime_id` while it is held for a move.
pub(super) async fn refuse_a_held_start<C>(client: &C, runtime_id: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    lock_runtime(client, runtime_id).await?;
    let held: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM agent_runtime_move_holds WHERE agent_runtime_id = $1)
                 OR EXISTS (SELECT 1 FROM agent_runtime_moves WHERE agent_runtime_id = $1)",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if held {
        return Err(CoreError::RuntimeMoving);
    }
    Ok(())
}

/// Refuse to hold `runtime_id`, or mark it moved, while a start of it is
/// queued or running: a restart, recovery or upgrade, or a relocation
/// (a trial restore is one too).
pub(super) async fn refuse_while_a_start_is_active<C>(
    client: &C,
    runtime_id: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    lock_runtime(client, runtime_id).await?;
    let active: bool = client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM runtime_control_requests
                 WHERE agent_runtime_id = $1
                   AND kind IN ('restart', 'recover_known_good_chat_runtime', 'upgrade')
                   AND status IN ('requested', 'launching', 'compute_up', 'ready'))
             OR EXISTS (
                 SELECT 1 FROM agent_creation_requests
                 WHERE agent_runtime_id = $1 AND relocation_spec IS NOT NULL
                   AND status IN ('requested', 'launching'))",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if active {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    Ok(())
}

/// The Runtime's row lock, which serializes a hold with a start.
async fn lock_runtime<C>(client: &C, runtime_id: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT 1 FROM agent_runtimes WHERE id = $1 FOR UPDATE",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

/// The mover's stop holds the Runtime, from the same transaction, as of
/// the stop's request.
pub(super) async fn hold<C>(client: &C, stop: &RuntimeControlRequest) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO agent_runtime_move_holds (agent_runtime_id, stop_request_id, held_at)
             VALUES ($1, $2, $3::text::timestamptz)
             ON CONFLICT (agent_runtime_id) DO NOTHING",
            &[&stop.agent_runtime_id, &stop.id, &stop.created_at],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

/// The mover's restart gives it back: the hold goes, in the transaction
/// that enqueues the restart. A Runtime marked moved stays held.
pub(super) async fn release<C>(client: &C, runtime_id: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "DELETE FROM agent_runtime_move_holds WHERE agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}
