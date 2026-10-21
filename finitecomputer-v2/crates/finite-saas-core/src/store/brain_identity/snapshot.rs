//! One bounded identity snapshot shared by legacy and scoped readers.
use super::*;

pub(super) struct IdentitySnapshot {
    humans: BTreeMap<String, HumanAssociationRow>,
    agent_rows: Vec<AgentPinRow>,
    accounts: BTreeMap<String, AccountRow>,
    releasable: BTreeSet<String>,
    human_keys_by_account: BTreeMap<String, Vec<String>>,
}

impl IdentitySnapshot {
    pub(super) async fn load<C: GenericClient + Sync>(
        client: &C,
        request: &BrainIdentityDescriptionsRequest,
        keys: &[String],
        extra_accounts: &[String],
        hint_scope: Option<&[String]>,
    ) -> CoreResult<Self> {
        let npub_by_key = keys
            .iter()
            .filter_map(|key| npub_for_hex(key).map(|npub| (key.clone(), npub)))
            .collect::<BTreeMap<_, _>>();
        let npubs = npub_by_key.values().cloned().collect::<Vec<_>>();

        let humans = client
            .query(
                "SELECT id, user_id, public_key_hex, core_rfc3339(last_observed_at) AS last_observed_at,
                        revision
                 FROM account_brain_principals
                 WHERE status = 'active' AND public_key_hex = ANY($1)",
                &[&keys],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>("public_key_hex"),
                    HumanAssociationRow {
                        id: row.get("id"),
                        user_id: row.get("user_id"),
                        last_observed_at: row.get("last_observed_at"),
                        revision: row.get("revision"),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let agent_rows = agent_pin_rows(client, &npubs).await?;
        let mut implicated = humans
            .values()
            .map(|row| row.user_id.clone())
            .collect::<BTreeSet<_>>();
        implicated.extend(
            agent_rows
                .iter()
                .filter_map(|row| row.consistent.as_ref())
                .map(|pin| pin.owner_user_id.clone()),
        );
        implicated.extend(extra_accounts.iter().cloned());
        let implicated = implicated.into_iter().collect::<Vec<_>>();

        // v1: only accounts that shared with this exact Brain. v2: every
        // implicated account, because Brain asks only about keys that acted
        // in this Brain themselves.
        let releasable = if request.requires_sharing_scope() {
            client
                .query(
                    "SELECT user_id FROM account_brain_sharing_scopes
                 WHERE revoked_at IS NULL AND brain_server = $1 AND brain_id = $2
                   AND user_id = ANY($3)",
                    &[&request.brain_server, &request.brain_id, &implicated],
                )
                .await
                .map_err(store_error)?
                .iter()
                .map(|row| row.get::<_, String>("user_id"))
                .collect::<BTreeSet<_>>()
        } else {
            implicated.iter().cloned().collect::<BTreeSet<_>>()
        };
        let releasable_ids = releasable.iter().cloned().collect::<Vec<_>>();
        // v2 names only an owner's human keys that are in this same request,
        // never keys outside it; without a sharing scope those would link the
        // owner across Brains.
        let accounts = client
            .query(
                "SELECT id,
                        CASE WHEN octet_length(normalized_email) <= $2 THEN normalized_email END
                          AS normalized_email,
                        link_status, workos_user_id, core_rfc3339(updated_at) AS updated_at
                 FROM users WHERE id = ANY($1)",
                &[&releasable_ids, &(MAX_CONTACT_BYTES as i32)],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>("id"),
                    AccountRow {
                        email: row.get("normalized_email"),
                        linked: row.get::<_, String>("link_status") == "linked",
                        workos_subject: row.get("workos_user_id"),
                        updated_at: row.get("updated_at"),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        // Bounded in SQL: at most MAX_RESPONSIBLE_HUMAN_KEYS rows per releasable
        // account, read through the (user_id, public_key_hex) partial index.
        let mut human_keys_by_account: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in client
            .query(
                "SELECT account.id AS user_id, account_key.public_key_hex
                 FROM unnest($1::text[]) AS account(id)
                 CROSS JOIN LATERAL (
                   SELECT public_key_hex FROM account_brain_principals
                   WHERE user_id = account.id AND status = 'active'
                     AND ($3::text[] IS NULL OR public_key_hex = ANY($3))
                   ORDER BY public_key_hex
                   LIMIT $2
                 ) AS account_key
                 ORDER BY account.id, account_key.public_key_hex",
                &[
                    &releasable_ids,
                    &(MAX_RESPONSIBLE_HUMAN_KEYS as i64),
                    &hint_scope,
                ],
            )
            .await
            .map_err(store_error)?
        {
            human_keys_by_account
                .entry(row.get::<_, String>("user_id"))
                .or_default()
                .push(row.get("public_key_hex"));
        }
        // A human key that is also pinned as an agent is a conflicting
        // record, not a hint. Exclude it with one bounded, indexed equality
        // read over the (at most 8 per account) candidate keys.
        let hint_npubs = human_keys_by_account
            .values()
            .flatten()
            .filter_map(|key| npub_for_hex(key))
            .collect::<Vec<_>>();
        let pinned_hints = client
            .query(
                "SELECT DISTINCT health_reporting_npub FROM agent_runtimes
                 WHERE health_reporting_npub = ANY($1)",
                &[&hint_npubs],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| row.get::<_, String>("health_reporting_npub"))
            .collect::<BTreeSet<_>>();
        for keys in human_keys_by_account.values_mut() {
            keys.retain(|key| npub_for_hex(key).is_some_and(|npub| !pinned_hints.contains(&npub)));
        }
        Ok(Self {
            humans,
            agent_rows,
            accounts,
            releasable,
            human_keys_by_account,
        })
    }

    pub(super) fn pins(&self, key: &str) -> Vec<&AgentPinRow> {
        let npub = npub_for_hex(key);
        self.agent_rows
            .iter()
            .filter(|row| Some(&row.npub) == npub.as_ref())
            .collect()
    }

    pub(super) fn describe(&self, key: &str) -> IdentityDescription {
        let pins = self.pins(key);
        let disclosable = |user: &str| {
            self.releasable.contains(user)
                && self
                    .accounts
                    .get(user)
                    .is_some_and(|account| account.linked)
        };
        describe_key(
            key,
            self.humans.get(key),
            agent_evidence(&pins),
            &disclosable,
            &self.accounts,
            &self.human_keys_by_account,
        )
    }

    pub(super) fn human(&self, key: &str) -> Option<&HumanAssociationRow> {
        self.humans.get(key)
    }
    pub(super) fn account(&self, id: &str) -> Option<&AccountRow> {
        self.accounts.get(id)
    }
}
