//! The fleet from Core's database (FIN-463 M1), through a read-only role
//! that may read only the columns below (the ops notes give its grants).
//!
//! - **The roster:** each live old agent (an active runtime link, not an
//!   imported candidate, not marked moved) and its owner, by their WorkOS
//!   user. Its sizes come from the daily `sizes` run's cache.
//! - **A binding:** the old agent's current runtime (its active link's, or
//!   once offboarded its latest), the runner host and machine, and its
//!   durable tree, all as Core reads them for a control (`control_leases`):
//!   only a Kata-placed runtime, the only kind whose tree the host reader
//!   can read; its spec the latest `running` creation request for that
//!   runtime, used only when it names this very runtime; its tree the
//!   spec's, repaired as Core repairs it when it names the machine, or the
//!   runtime id when there is no spec (as Core synthesizes one). Anything
//!   else isn't this mover's to guess at: it's left off the roster and
//!   never acted on. A Recovery Snapshot already taken is the retirement's
//!   receipt.
//!
//! Brains, sites and which secrets an agent holds are not Core's: the
//! roster lists none of them yet (the ops notes say where they come from).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio_postgres::NoTls;

use crate::contract::{Has, ROSTER_AGENTS_MAX, Roster, RosterAgent, RosterPerson, valid_old};
use crate::fleet::{Binding, Fleet, FleetError};
use crate::sizes::SizesCache;

pub struct PgFleet {
    conninfo: String,
    sizes_file: PathBuf,
}

/// Each runtime's spec, as Core picks it for a control.
const SPEC: &str = "
LEFT JOIN LATERAL (
  SELECT request.id, request.runtime_spec FROM agent_creation_requests AS request
  WHERE request.agent_runtime_id = runtime.id AND request.status = 'running'
  ORDER BY request.created_at DESC, request.id DESC LIMIT 1
) AS spec ON true";

/// The runtime and its spec, as `binding_of` reads them.
const RUNTIME_COLUMNS: &str = "
       runtime.id AS runtime_id, runtime.source_host_id, runtime.source_machine_id,
       runtime.placement_runner_class,
       spec.id IS NOT NULL AS has_creation_row, spec.runtime_spec IS NOT NULL AS has_spec,
       spec.runtime_spec->>'schema' AS spec_schema,
       spec.runtime_spec->'spec'->>'agentRuntimeId' AS spec_runtime_id,
       spec.runtime_spec->'spec'->>'durableStateId' AS spec_durable_state_id";

/// The live old agents, with their owner and binding.
fn live_query() -> String {
    format!(
        "SELECT person.workos_user_id, person.normalized_email,
       project.id AS old, project.display_name,
       (SELECT primary_request.profile_picture_url FROM agent_creation_requests AS primary_request
        WHERE primary_request.project_id = project.id AND primary_request.relocation_spec IS NULL)
         AS profile_picture_url,
       {RUNTIME_COLUMNS}
FROM projects AS project
JOIN users AS person ON person.id = project.owner_user_id
JOIN project_runtime_links AS link ON link.project_id = project.id AND link.active
JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
{SPEC}
WHERE project.import_candidate_id IS NULL
  AND person.workos_user_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM agent_runtime_moves AS moved WHERE moved.agent_runtime_id = runtime.id)
ORDER BY person.workos_user_id, project.id"
    )
}

/// One old agent's current runtime: its active link's, or its latest.
fn binding_query() -> String {
    format!(
        "SELECT {RUNTIME_COLUMNS},
       (SELECT snapshot.request_id FROM runtime_retirement_snapshots AS snapshot
        WHERE snapshot.agent_runtime_id = runtime.id) AS retired_receipt
FROM agent_runtimes AS runtime
LEFT JOIN project_runtime_links AS link ON link.agent_runtime_id = runtime.id AND link.active
{SPEC}
WHERE runtime.project_id = $1
ORDER BY link.active IS NOT NULL DESC, runtime.created_at DESC, runtime.id DESC
LIMIT 1"
    )
}

impl PgFleet {
    pub fn new(conninfo: &str, sizes_file: &Path) -> Self {
        Self {
            conninfo: conninfo.to_string(),
            sizes_file: sizes_file.to_path_buf(),
        }
    }

    fn query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, FleetError> {
        let failed = |error: tokio_postgres::Error| {
            FleetError::Unreadable(format!("Core's database: {error}"))
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| FleetError::Unreadable(error.to_string()))?;
        runtime.block_on(async {
            let (client, connection) = tokio_postgres::connect(&self.conninfo, NoTls)
                .await
                .map_err(failed)?;
            let driver = tokio::spawn(async move {
                let _ = connection.await;
            });
            let rows = client.query(sql, params).await.map_err(failed);
            drop(client);
            let _ = driver.await;
            rows
        })
    }

    /// Every live old agent's binding, for the daily `sizes` run.
    pub fn live(&self) -> Result<Vec<Binding>, FleetError> {
        Ok(self
            .query(&live_query(), &[])?
            .iter()
            .filter_map(binding_of)
            .collect())
    }
}

/// A row's binding, or `None` when Core's rule gives this mover nothing
/// to act on (then it's never listed, read or changed).
fn binding_of(row: &tokio_postgres::Row) -> Option<Binding> {
    let runtime_id: String = row.get("runtime_id");
    let source_machine_id: String = row.get("source_machine_id");
    let placement: Option<String> = row.get("placement_runner_class");
    let has_creation_row: bool = row.get("has_creation_row");
    let has_spec: bool = row.get("has_spec");
    if placement.as_deref() != Some("kata") || !has_creation_row {
        return None;
    }
    let durable_state_id = if has_spec {
        let schema: Option<String> = row.get("spec_schema");
        let spec_runtime: Option<String> = row.get("spec_runtime_id");
        let durable: String = row.get::<_, Option<String>>("spec_durable_state_id")?;
        if schema.as_deref() != Some("runtime_spec.v1")
            || spec_runtime.as_deref() != Some(runtime_id.as_str())
        {
            return None;
        }
        // Core's repair (`repair_persisted_runtime_spec`): a spec naming the
        // machine means the runtime's own id.
        if durable == source_machine_id {
            runtime_id.clone()
        } else {
            durable
        }
    } else {
        // Core synthesizes a Kata runtime's missing spec with its own id.
        runtime_id.clone()
    };
    Some(Binding {
        old: row.try_get("old").unwrap_or_default(),
        agent_runtime_id: runtime_id,
        source_host_id: row.get("source_host_id"),
        source_machine_id,
        durable_state_id,
        owned_brains: Vec::new(),
        retired_receipt: row.try_get("retired_receipt").ok().flatten(),
    })
}

impl Fleet for PgFleet {
    fn roster(&self) -> Result<Roster, FleetError> {
        let sizes = SizesCache::read(&self.sizes_file);
        let mut people: BTreeMap<String, RosterPerson> = BTreeMap::new();
        for row in self.query(&live_query(), &[])? {
            let old: String = row.get("old");
            // An id the wire can't carry, or a runtime whose spec isn't its
            // own, is left off the roster: never offered, never moved.
            if !valid_old(&old) || binding_of(&row).is_none() {
                continue;
            }
            let workos_id: String = row.get("workos_user_id");
            let person = people
                .entry(workos_id.clone())
                .or_insert_with(|| RosterPerson {
                    workos_id,
                    email: row.get("normalized_email"),
                    agents: Vec::new(),
                });
            if person.agents.len() >= ROSTER_AGENTS_MAX {
                continue;
            }
            person.agents.push(RosterAgent {
                sizes: sizes.agents.get(&old).copied().unwrap_or_default(),
                old,
                name: row.get("display_name"),
                picture_url: row.get("profile_picture_url"),
                last_active_ms: None,
                brains: Vec::new(),
                sites: Vec::new(),
                has: Has::default(),
            });
        }
        let observed_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        Ok(Roster {
            snapshot: format!("core-{observed_ms}"),
            observed_ms,
            people: people.into_values().collect(),
        })
    }

    fn binding(&self, old: &str) -> Result<Option<Binding>, FleetError> {
        let rows = self.query(&binding_query(), &[&old])?;
        Ok(rows.first().and_then(|row| {
            let mut binding = binding_of(row)?;
            binding.old = old.to_string();
            Some(binding)
        }))
    }
}
