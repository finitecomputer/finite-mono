use super::*;
use crate::core_descriptions::scoped::{
    DESCRIPTIONS_VERSION_V3, ScopedCoreDescription, ScopedCoreResponse, ScopedLookupReport,
    ScopedLookupRequest,
};

fn record() -> serde_json::Value {
    serde_json::json!({"state":"used","revision":"fixture-record","observedAt":"2026-05-01T00:00:00Z"})
}

fn answer(request: &ScopedLookupRequest, account: &str, annotation: &str) -> ScopedCoreResponse {
    let results = request.keys.iter().map(|key| {
        let row = if key == account {
            serde_json::json!({"publicKeyHex":key,"state":"resolved","kind":"human","accountEmail":"synthetic-account@example.test",
                "source":{"kind":"operatorScopedRecord","revision":"fixture-record","observedAt":"2026-05-01T00:00:00Z"},"operatorRecord":record()})
        } else if key == annotation {
            serde_json::json!({"publicKeyHex":key,"state":"notShared","operatorAnnotation":{"kind":"historical_identity","label":"Previous synthetic Agent","observedAt":"2026-05-01T00:00:00Z"},"operatorRecord":record()})
        } else {
            serde_json::json!({"publicKeyHex":key,"state":"notShared"})
        };
        serde_json::from_value::<ScopedCoreDescription>(row).unwrap()
    }).collect();
    ScopedCoreResponse {
        version: DESCRIPTIONS_VERSION_V3.to_owned(),
        brain_id: request.brain_id.clone(),
        checked_at: "2026-05-02T00:00:00Z".into(),
        results,
    }
}

#[tokio::test]
async fn v3_signed_roster_describes_scoped_nonparticipants_without_durable_mutation() {
    let cast = Cast::new();
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
    let account = hex_of(&cast.replacement);
    let annotation = hex_of(&cast.arbitrary);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let calls = observed.clone();
    state.core_descriptions = Some(CoreDescriptionClient {
        brain_server: TEST_BASE_URL.trim_end_matches('/').into(),
        lookup: Arc::new(move |request| {
            calls.lock().unwrap().push(request.clone());
            ScopedLookupReport {
                result: Ok(Some(answer(request, &account, &annotation))),
                legacy: false,
                negotiation: "direct",
            }
        }),
    });
    let before = durable_state(&state, &cast);
    let router = router_with_state(state.clone());
    let (status, report) = get_report(&router, &cast.admin, "", TEST_NOW + 60).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["descriptions"]["state"], "checked");
    assert_eq!(
        report["descriptions"]["coreProtocolVersion"],
        DESCRIPTIONS_VERSION_V3
    );
    assert_eq!(report["descriptions"]["coreNegotiation"], "direct");
    assert_eq!(
        report["descriptions"]["checkedKeys"],
        report["identities"].as_array().unwrap().len()
    );
    assert_eq!(
        row(&report, &cast.replacement)["description"]["accountEmail"],
        "synthetic-account@example.test"
    );
    assert_eq!(
        row(&report, &cast.arbitrary)["description"]["state"],
        "notShared"
    );
    assert_eq!(
        row(&report, &cast.arbitrary)["description"]["reason"],
        "noParticipation"
    );
    assert_eq!(
        row(&report, &cast.arbitrary)["operatorAnnotation"]["label"],
        "Previous synthetic Agent"
    );
    let calls = observed.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].keys.contains(&hex_of(&cast.replacement)));
    assert!(
        !calls[0]
            .participating_keys
            .contains(&hex_of(&cast.replacement))
    );
    assert_eq!(
        calls[0].keys.len(),
        report["identities"].as_array().unwrap().len()
    );
    assert!(calls[0].keys.contains(&hex_of(&cast.outsider)));
    assert_eq!(before, durable_state(&state, &cast));
}

#[tokio::test]
async fn v3_failure_and_fallback_failure_have_distinct_nonparticipant_policy() {
    let cast = Cast::new();
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
    for legacy in [false, true] {
        state.core_descriptions = Some(CoreDescriptionClient {
            brain_server: TEST_BASE_URL.trim_end_matches('/').into(),
            lookup: Arc::new(move |_| ScopedLookupReport {
                result: Err(CoreLookupFailure::Unavailable(
                    "synthetic transport failure".into(),
                )),
                legacy,
                negotiation: if legacy {
                    "afterSchemaRejection"
                } else {
                    "direct"
                },
            }),
        });
        let router = router_with_state(state.clone());
        let (status, report) =
            get_report(&router, &cast.admin, "", TEST_NOW + 55 + u64::from(legacy)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(report["descriptions"]["state"], "unavailable");
        assert!(report["descriptions"]["coreProtocolVersion"].is_null());
        assert_eq!(
            row(&report, &cast.replacement)["description"]["state"],
            if legacy { "notShared" } else { "unavailable" }
        );
        assert_eq!(
            row(&report, &cast.mailbox)["description"]["state"],
            "unavailable"
        );
        for row in report["identities"].as_array().unwrap() {
            assert!(row["operatorRecord"].is_null());
            assert!(row["operatorAnnotation"].is_null());
        }
    }
}

#[tokio::test]
async fn v3_bad_operator_provenance_invalidates_entire_signed_page() {
    let cast = Cast::new();
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
    let account = hex_of(&cast.replacement);
    let annotation = hex_of(&cast.arbitrary);
    state.core_descriptions = Some(CoreDescriptionClient {
        brain_server: TEST_BASE_URL.trim_end_matches('/').into(),
        lookup: Arc::new(move |request| {
            let mut response = answer(request, &account, &annotation);
            response
                .results
                .iter_mut()
                .find(|r| r.public_key_hex == account)
                .unwrap()
                .operator_record = None;
            ScopedLookupReport {
                result: Ok(Some(response)),
                legacy: false,
                negotiation: "direct",
            }
        }),
    });
    let router = router_with_state(state);
    let (status, report) = get_report(&router, &cast.admin, "", TEST_NOW + 60).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["descriptions"]["state"], "unavailable");
    assert!(
        !report
            .to_string()
            .contains("synthetic-account@example.test")
    );
    assert!(!report.to_string().contains("Previous synthetic Agent"));
}
