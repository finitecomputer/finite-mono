use super::*;
use crate::support::SupportService;
use finite_mail::{MailError, MailTransport, TextEmail};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TestMailer {
    calls: Mutex<Vec<(String, String, String, String)>>,
    fail: std::sync::atomic::AtomicBool,
}
impl MailTransport for TestMailer {
    fn send_text_email(&self, _: &TextEmail<'_>) -> Result<(), MailError> {
        panic!("support requires Reply-To");
    }
    fn send_text_email_with_reply_to(
        &self,
        key: &str,
        email: &TextEmail<'_>,
        reply: &str,
    ) -> Result<(), MailError> {
        self.calls.lock().unwrap().push((
            key.into(),
            email.to.into(),
            reply.into(),
            email.text.into(),
        ));
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            Err(MailError::Send("synthetic failure".into()))
        } else {
            Ok(())
        }
    }
}
fn report(key: &str) -> Value {
    json!({"idempotencyKey": key, "projectId": null, "message": "Agent stopped responding", "supportEmail": "it@example.org", "replyTo": "support-test@example.org"})
}
fn account_headers() -> Vec<(String, String)> {
    vec![(
        "authorization".into(),
        format!(
            "Bearer {}",
            access_token("support-test@example.org", true, None)
        ),
    )]
}

#[tokio::test]
async fn support_round_trip_retries_and_deduplicates_without_chat_or_runtime() {
    with_isolated_postgres(|db| async move {
        let mailer = Arc::new(TestMailer::default());
        mailer.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        let service = SupportService { email: "it@example.org".into(), mailer: mailer.clone() };
        let app = router(db.store.clone(), test_auth()).layer(axum::Extension(service.clone()));
        let headers = account_headers();
        let (status, receipt) = send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("request-1"))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{receipt}");
        assert_eq!(receipt["status"], "pending");
        let id = receipt["id"].as_str().unwrap();
        assert!(service.deliver_next(&db.store).await.unwrap());
        assert_eq!(db.row("support_requests", id).await.unwrap()["status"], "pending");
        assert!(!service.deliver_next(&db.store).await.unwrap(), "backoff must prevent immediate retry");
        db.query_json("UPDATE support_requests SET next_attempt_at = now() RETURNING to_jsonb(support_requests)", &[]).await;
        mailer.fail.store(false, std::sync::atomic::Ordering::Relaxed);
        // Simulate a service restart against the retained database.
        let reopened = CoreStore::connect(&db.url).await.unwrap();
        assert!(service.deliver_next(&reopened).await.unwrap());
        assert_eq!(db.row("support_requests", id).await.unwrap()["status"], "sent");
        let (_, replay) = send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("request-1"))).await;
        assert_eq!(replay["id"], receipt["id"]);
        assert_eq!(replay["status"], "sent");
        assert!(!service.deliver_next(&db.store).await.unwrap());
        let calls = mailer.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1], "retries must use identical key and email payload");
        assert_eq!(calls[0].1, "it@example.org");
        assert_eq!(calls[0].2, "support-test@example.org");
        assert!(calls[0].3.ends_with("Agent stopped responding"));
        assert!(!calls[0].3.contains("chat history"));
    }).await;
}

#[tokio::test]
async fn support_rejects_unauthorized_spoofed_and_unreviewed_submissions() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth()).layer(axum::Extension(SupportService {
            email: "it@example.org".into(),
            mailer: Arc::new(TestMailer::default()),
        }));
        for headers in [
            vec![],
            vec![("authorization".into(), "Bearer core-token".into())],
            vec![(
                "authorization".into(),
                format!(
                    "Bearer {}",
                    access_token("support-test@example.org", false, None)
                ),
            )],
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/me/support",
                &headers,
                Some(report("denied")),
            )
            .await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let headers = account_headers();
        for (field, value, expected) in [
            (
                "supportEmail",
                json!("attacker@example.org"),
                StatusCode::CONFLICT,
            ),
            ("replyTo", json!("other@example.org"), StatusCode::CONFLICT),
            (
                "projectId",
                json!("someone-elses-agent"),
                StatusCode::NOT_FOUND,
            ),
            ("message", json!(" "), StatusCode::BAD_REQUEST),
            ("message", json!("x".repeat(4001)), StatusCode::BAD_REQUEST),
        ] {
            let mut input = report("denied");
            input[field] = value;
            let (status, body) = send_json(
                &app,
                "POST",
                "/api/core/v1/me/support",
                &headers,
                Some(input),
            )
            .await;
            assert_eq!(status, expected, "{body}");
        }
        assert!(db.all("support_requests").await.is_empty());
        let old_core = router(db.store.clone(), test_auth());
        let (status, _) = send_json(
            &old_core,
            "POST",
            "/api/core/v1/me/support",
            &headers,
            Some(report("disabled")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    })
    .await;
}

#[tokio::test]
async fn support_concurrency_rate_limit_and_ambiguous_delivery_expiry() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth()).layer(axum::Extension(SupportService { email: "it@example.org".into(), mailer: Arc::new(TestMailer::default()) }));
        let headers = account_headers();
        // Establish the account before racing independent report submissions.
        send_json(&app, "GET", "/api/core/v1/me", &headers, None).await;
        let (first, second) = tokio::join!(
            send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("same"))),
            send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("same")))
        );
        assert_eq!(first.0, StatusCode::ACCEPTED);
        assert_eq!(first.1["id"], second.1["id"]);
        assert_eq!(db.all("support_requests").await.len(), 1);
        let mut changed = report("same"); changed["message"] = json!("Different report");
        assert_eq!(send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(changed)).await.0, StatusCode::CONFLICT);
        for index in 0..4 { assert_eq!(send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report(&format!("more-{index}")))).await.0, StatusCode::ACCEPTED); }
        assert_eq!(send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("limited"))).await.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(send_json(&app, "POST", "/api/core/v1/me/support", &headers, Some(report("same"))).await.0, StatusCode::ACCEPTED);
        let delivery = db.claim_support_delivery().await.unwrap().unwrap();
        db.finish_support_delivery(&delivery.id, "wrong-lease", true).await.unwrap();
        assert_eq!(db.row("support_requests", &delivery.id).await.unwrap()["status"], "pending");
        db.query_json("UPDATE support_requests SET created_at = now() - interval '25 hours', lease_until = now() - interval '1 minute' RETURNING to_jsonb(support_requests)", &[]).await;
        assert!(db.claim_support_delivery().await.unwrap().is_none());
        assert!(db.all("support_requests").await.iter().all(|row| row["status"] == "failed"));
        // Additive migration reapplication preserves retained reports and existing users.
        db.migrate().await.unwrap();
        assert_eq!(db.all("support_requests").await.len(), 5);
        assert_eq!(send_json(&app, "GET", "/api/core/v1/me", &headers, None).await.0, StatusCode::OK);
    }).await;
}

#[tokio::test]
async fn support_accepts_an_owned_project_before_agent_launch_and_preserves_it() {
    with_isolated_postgres(|db| async move {
        let creation = db
            .request_agent_creation(crate::RequestAgentCreationInput {
                verified_email: "support-owner@example.org".into(),
                workos_user_id: "support-owner".into(),
                display_name: "Not launched yet".into(),
                launch_code: issue_test_launch_code(&db).await,
                idempotency_key: "support-onboarding".into(),
                now: None,
            })
            .await
            .unwrap();
        let app = router(db.store.clone(), test_auth()).layer(axum::Extension(SupportService {
            email: "it@example.org".into(),
            mailer: Arc::new(TestMailer::default()),
        }));
        let headers = [(
            "authorization".into(),
            format!(
                "Bearer {}",
                access_token_with_subject("support-owner", "support-owner@example.org", true, None)
            ),
        )];
        let before = db.row("projects", &creation.project.id).await.unwrap();
        let mut input = report("owned-project");
        input["replyTo"] = json!("support-owner@example.org");
        input["projectId"] = json!(creation.project.id);
        assert_eq!(
            send_json(
                &app,
                "POST",
                "/api/core/v1/me/support",
                &headers,
                Some(input.clone())
            )
            .await
            .0,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            db.row("projects", &creation.project.id).await.unwrap(),
            before
        );
        // A different verified account cannot attach that Project to a report.
        input["replyTo"] = json!("support-test@example.org");
        assert_eq!(
            send_json(
                &app,
                "POST",
                "/api/core/v1/me/support",
                &account_headers(),
                Some(input)
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    })
    .await;
}
