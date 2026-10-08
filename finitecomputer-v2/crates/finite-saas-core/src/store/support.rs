use super::*;
use crate::identity::generate_surrogate_id;
use crate::support::{SubmitSupportResult, SupportDelivery, SupportInput, SupportReceipt};

impl CoreStore {
    #[tracing::instrument(skip(self, input), fields(operation = "submit_support"))]
    pub(crate) async fn submit_support(
        &self,
        workos_user_id: &str,
        input: &SupportInput,
    ) -> CoreResult<SubmitSupportResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Serialize only this account's submissions, including its hourly limit.
        let user = tx
            .query_one(
                "SELECT id FROM users WHERE workos_user_id = $1 FOR UPDATE",
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?;
        let user_id: String = user.get(0);
        if let Some(row) = tx.query_opt("SELECT id, status, project_id, recipient, reply_to, message FROM support_requests WHERE user_id = $1 AND idempotency_key = $2", &[&user_id, &input.idempotency_key]).await.map_err(store_error)? {
            let matches = row.get::<_, Option<String>>("project_id") == input.project_id
                && row.get::<_, String>("recipient") == input.support_email
                && row.get::<_, String>("reply_to") == input.reply_to
                && row.get::<_, String>("message") == input.message;
            return Ok(if matches { SubmitSupportResult::Receipt(SupportReceipt { id: row.get("id"), status: row.get("status") }) } else { SubmitSupportResult::Conflict });
        }
        let count: i64 = tx.query_one("SELECT count(*) FROM support_requests WHERE user_id = $1 AND created_at > now() - interval '1 hour'", &[&user_id]).await.map_err(store_error)?.get(0);
        if count >= 5 {
            return Ok(SubmitSupportResult::RateLimited);
        }
        let id = generate_surrogate_id("support")?;
        tx.execute("INSERT INTO support_requests (id, user_id, idempotency_key, project_id, recipient, reply_to, message) VALUES ($1,$2,$3,$4,$5,$6,$7)", &[&id, &user_id, &input.idempotency_key, &input.project_id, &input.support_email, &input.reply_to, &input.message]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(SubmitSupportResult::Receipt(SupportReceipt {
            id,
            status: "pending".into(),
        }))
    }

    #[tracing::instrument(skip(self), fields(operation = "claim_support_delivery"))]
    pub(crate) async fn claim_support_delivery(&self) -> CoreResult<Option<SupportDelivery>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Stop before the provider's 24h deduplication window can expire. Keep
        // ambiguous reports for operator investigation rather than sending anew.
        let expired = tx.query("UPDATE support_requests SET status = 'failed', lease_token = NULL, lease_until = NULL WHERE status = 'pending' AND created_at < now() - interval '23 hours' AND (lease_until IS NULL OR lease_until < now()) RETURNING id", &[]).await.map_err(store_error)?;
        for row in expired {
            tracing::error!(request_id = %row.get::<_, String>(0), "support email needs operator attention");
        }
        let token = generate_surrogate_id("delivery")?;
        let row = tx.query_opt("UPDATE support_requests SET lease_token = $1, lease_until = now() + interval '2 minutes', attempts = attempts + 1 WHERE id = (SELECT id FROM support_requests WHERE status = 'pending' AND next_attempt_at <= now() AND (lease_until IS NULL OR lease_until < now()) ORDER BY next_attempt_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id, recipient, reply_to, project_id, message", &[&token]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(row.map(|row| SupportDelivery {
            id: row.get("id"),
            recipient: row.get("recipient"),
            reply_to: row.get("reply_to"),
            project_id: row.get("project_id"),
            message: row.get("message"),
            lease_token: token,
        }))
    }

    #[tracing::instrument(skip(self), fields(operation = "finish_support_delivery"))]
    pub(crate) async fn finish_support_delivery(
        &self,
        id: &str,
        lease: &str,
        sent: bool,
    ) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("UPDATE support_requests SET status = CASE WHEN $3 THEN 'sent' ELSE 'pending' END, next_attempt_at = now() + interval '1 minute', lease_token = NULL, lease_until = NULL WHERE id = $1 AND lease_token = $2 AND status = 'pending'", &[&id, &lease, &sent]).await.map_err(store_error)?;
        self.finish(tx).await
    }
}
