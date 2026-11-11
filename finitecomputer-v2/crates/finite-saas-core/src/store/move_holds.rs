//! FIN-463's hold: a Runtime the finite.chat mover stopped for a move is
//! started by nothing but the mover's own `runtime-restart-exact` (staff gave
//! it back), and a Runtime marked moved by nothing at all
//! (`agent_runtime_move_holds`, migration 0046). One rule, asked where every
//! start reaches Core: the control enqueue (restart, recovery, upgrade) and
//! the relocation insert. Stop and destroy stay allowed.
//!
//! Delete with the table, when every old agent is moved or declined.

use super::*;

/// Refuse to start `runtime_id` while it is held for a move.
pub(super) async fn refuse_a_held_start<C>(client: &C, runtime_id: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
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
