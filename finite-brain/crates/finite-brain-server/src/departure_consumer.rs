//! Permanent Departure Fact consumer (ADR-0046).
//!
//! SaaS Core emits durable, monotonic Permanent Departure Facts when account
//! principals (humans or Managed Agents) permanently depart. This module polls
//! that log from a last-applied-revision cursor and applies each fact through
//! the store's revocation machinery. Routine authorization never consults
//! Core: this loop is the only caller, it runs outside every request path,
//! and Core outages only delay revocation — they degrade nothing else.

use crate::*;

/// How often the consumer polls Core's departure log.
pub const DEPARTURE_FACT_POLL_INTERVAL: Duration = Duration::from_secs(30);
const DEPARTURE_FACT_PAGE_LIMIT: u32 = 500;
const DEPARTURE_FACT_BACKOFF_INITIAL: Duration = Duration::from_secs(5);
const DEPARTURE_FACT_BACKOFF_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreDepartureFactsPage {
    facts: Vec<CoreDepartureFact>,
    #[allow(dead_code)]
    max_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreDepartureFact {
    revision: i64,
    account_id: String,
    principal_kind: String,
    principal_ref: String,
    #[allow(dead_code)]
    departed_at: String,
    #[allow(dead_code)]
    reason: String,
}

/// Start the background departure-fact consumer. Returns `None` when the Core
/// and Identity authorities are not configured; brains then work fully
/// without the enrichment layer, exactly as before.
pub fn spawn_departure_fact_consumer(state: &ServerState) -> Option<tokio::task::JoinHandle<()>> {
    state.agent_bootstrap_authorities.as_ref()?;
    Some(tokio::spawn(departure_fact_consumer_loop(
        state.clone(),
        DEPARTURE_FACT_POLL_INTERVAL,
    )))
}

async fn departure_fact_consumer_loop(state: ServerState, interval: Duration) {
    let mut backoff = DEPARTURE_FACT_BACKOFF_INITIAL;
    loop {
        tokio::time::sleep(interval).await;
        match poll_departure_facts_once(&state).await {
            Ok(applied) => {
                if applied > 0 {
                    eprintln!(
                        "finite-brain applied {applied} Permanent Departure Fact(s) from Core"
                    );
                }
                backoff = DEPARTURE_FACT_BACKOFF_INITIAL;
            }
            Err(error) => {
                // Core or Identity is unreachable or misbehaving; revocation
                // is delayed, never skipped, and routine authorization is
                // unaffected. Retry with capped exponential backoff.
                eprintln!(
                    "finite-brain Permanent Departure Fact poll failed ({}): {}",
                    error.status, error.message
                );
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2).min(DEPARTURE_FACT_BACKOFF_MAX);
            }
        }
    }
}

/// Poll one page of departure facts and apply every fact in revision order.
/// Returns the number of newly applied facts. The cursor only advances inside
/// the store's per-fact revocation transaction, so any failure here simply
/// replays later.
pub(crate) async fn poll_departure_facts_once(state: &ServerState) -> Result<usize, ApiError> {
    let Some(authorities) = state.agent_bootstrap_authorities.clone() else {
        return Ok(0);
    };
    let cursor = {
        let store = state.store.lock().map_err(lock_error)?;
        store.departure_fact_cursor()?
    };
    let page: CoreDepartureFactsPage = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/departure-facts",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "afterRevision": cursor, "limit": DEPARTURE_FACT_PAGE_LIMIT }),
        "Finite Core Permanent Departure Facts",
    )
    .await?;

    let mut expected_revision = cursor;
    let mut applied = 0;
    for fact in &page.facts {
        if fact.revision <= expected_revision {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Finite Core Permanent Departure Facts authority failure: out-of-order revisions",
            ));
        }
        expected_revision = fact.revision;
        let departed_npub = resolve_departed_principal(state, fact).await?;
        let application = DepartureFactApplication {
            fact_revision: fact.revision,
            account_id: fact.account_id.clone(),
            principal_kind: DeparturePrincipalKind::try_from(fact.principal_kind.as_str())?,
            principal_ref: fact.principal_ref.clone(),
            departed_npub,
            applied_at: server_timestamp(state),
        };
        let outcome = {
            let mut store = state.store.lock().map_err(lock_error)?;
            store.apply_departure_fact(&application)?
        };
        if outcome.applied {
            applied += 1;
            for brain_id in &outcome.affected_brain_ids {
                state.publish_access_update(brain_id);
            }
        }
    }
    Ok(applied)
}

/// Bind a departed principal to its npub. Humans resolve through Finite
/// Identity's user binding (Core's `account_id` is the WorkOS user id); agents
/// resolve their Managed Agent Email through NIP-05 and verify the binding
/// through Identity's agent resolution. When the authority-side binding is
/// already gone (retired or deleted principals), the identity alias the Brain
/// recorded at grant time is the fallback evidence. Returns `None` when
/// nothing can bind the principal; the fact is then consumed with no local
/// effect because no local access can be attributed to it.
async fn resolve_departed_principal(
    state: &ServerState,
    fact: &CoreDepartureFact,
) -> Result<Option<UserId>, ApiError> {
    match fact.principal_kind.as_str() {
        "human" => resolve_departed_human(state, fact).await,
        "agent" => resolve_departed_agent(state, fact).await,
        other => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "Finite Core Permanent Departure Facts authority failure: unknown principal kind {other}"
            ),
        )),
    }
}

async fn resolve_departed_human(
    state: &ServerState,
    fact: &CoreDepartureFact,
) -> Result<Option<UserId>, ApiError> {
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let owner: Option<IdentityUserResolutionResponse> = post_authority_json_optional(
        &format!(
            "{}/api/v1/operator/brain/user-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "workosUserId": fact.account_id }),
        "Finite Identity departed User resolution",
    )
    .await?;
    match owner {
        Some(owner) if owner.workos_user_id == fact.account_id => {
            Ok(Some(UserId::new(owner.user_npub)?))
        }
        Some(_) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Identity returned a mismatched departed WorkOS account",
        )),
        // The account binding is already gone; the locally recorded identity
        // alias for the departed mailbox is the remaining evidence.
        None => local_alias_npub_for_ref(state, &fact.principal_ref),
    }
}

async fn resolve_departed_agent(
    state: &ServerState,
    fact: &CoreDepartureFact,
) -> Result<Option<UserId>, ApiError> {
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let managed_agent_email = canonical_email(&fact.principal_ref)?;
    let resolved = match resolve_identity_input(state, &managed_agent_email).await {
        Ok(resolved) => Some(resolved),
        Err(error) if error.status == StatusCode::NOT_FOUND => None,
        Err(error) => return Err(error),
    };
    if let Some(resolved) = resolved {
        let agent: Option<IdentityAgentResolutionResponse> = post_authority_json_optional(
            &format!(
                "{}/api/v1/operator/brain/agent-resolution",
                authorities.identity_base_url
            ),
            "X-Finite-Operator-Token",
            &authorities.identity_token,
            &serde_json::json!({ "agentNpub": resolved.npub }),
            "Finite Identity departed Agent resolution",
        )
        .await?;
        match agent {
            Some(agent)
                if agent.agent_npub == resolved.npub
                    && canonical_email(&agent.managed_agent_email)? == managed_agent_email =>
            {
                return Ok(Some(UserId::new(resolved.npub)?));
            }
            Some(_) => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "Finite Identity returned a mismatched departed Managed Agent",
                ));
            }
            None => {
                // The binding is gone with the agent. Trust the NIP-05 answer
                // only when the locally recorded alias confirms it, so a
                // rebound name can never target a different npub.
                if let Some(alias) = local_alias_npub_for_ref(state, &managed_agent_email)?
                    && alias.as_str() == resolved.npub
                {
                    return Ok(Some(alias));
                }
                return Ok(None);
            }
        }
    }
    // The NIP-05 name is gone with the agent; the locally recorded identity
    // alias is the remaining evidence.
    local_alias_npub_for_ref(state, &managed_agent_email)
}

fn local_alias_npub_for_ref(
    state: &ServerState,
    principal_ref: &str,
) -> Result<Option<UserId>, ApiError> {
    let Ok(canonical) = canonical_email(principal_ref) else {
        return Ok(None);
    };
    let store = state.store.lock().map_err(lock_error)?;
    Ok(store
        .identity_alias_for_preferred_nip05(&canonical)?
        .map(|alias| alias.npub))
}
