use super::*;
use serde_json::{Value, json};
use std::sync::Mutex;

const FIXTURE: &str = include_str!(
    "../../../../../../finitecomputer-v2/docs/contracts/brain-identity-descriptions-v3.example.json"
);
fn fixture() -> (ScopedLookupRequest, ScopedCoreResponse) {
    let response: ScopedCoreResponse = serde_json::from_str(FIXTURE).unwrap();
    let keys: Vec<_> = response
        .results
        .iter()
        .map(|r| r.public_key_hex.clone())
        .collect();
    let request = ScopedLookupRequest {
        brain_server: "https://brain.test".to_owned(),
        brain_id: response.brain_id.clone(),
        requested_by_hex: "ee".repeat(32),
        participating_keys: keys.clone(),
        keys,
    };
    (request, response)
}
#[test]
fn shared_v3_fixture_and_provenance_adversaries_are_checked_as_whole_batches() {
    let (request, response) = fixture();
    validate_scoped_response(&request, response.clone()).unwrap();
    let mut cases = vec![];
    let mut r = response.clone();
    r.results.pop();
    cases.push(r);
    let mut r = response.clone();
    r.results[3] = r.results[2].clone();
    cases.push(r);
    let mut r = response.clone();
    r.results[2].operator_annotation.as_mut().unwrap().label = "Name — guess".to_owned();
    cases.push(r);
    let mut r = response.clone();
    r.results[2]
        .operator_annotation
        .as_mut()
        .unwrap()
        .observed_at = "2026-02-30T00:00:00Z".to_owned();
    cases.push(r);
    let mut r = response.clone();
    r.results[1].operator_record = None;
    cases.push(r);
    let mut r = response.clone();
    r.results[1].operator_record.as_mut().unwrap().state = "conflict".to_owned();
    cases.push(r);
    let mut r = response.clone();
    r.results[4].operator_record = r.results[1].operator_record.clone();
    cases.push(r);
    let mut r = response.clone();
    r.results[9]
        .responsible_account
        .as_mut()
        .unwrap()
        .human_public_keys_hex = vec![request.keys[4].clone()];
    cases.push(r);
    let mut r = response.clone();
    r.results[2].account_email = Some("leak@example.test".to_owned());
    cases.push(r);
    let mut r = response.clone();
    r.results[2].operator_record.as_mut().unwrap().observed_at = "2026-01-02T00:00:00Z".to_owned();
    cases.push(r);
    for response in cases {
        assert!(validate_scoped_response(&request, response).is_err());
    }
    let mut nonparticipant = request.clone();
    nonparticipant.participating_keys.clear();
    assert!(validate_scoped_response(&nonparticipant, response).is_err());
}

/// Actual HTTP capture; the legacy decoder fixture has the deployed strict
/// five-field shape. Actual pinned old binaries are a separate release gate.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyWire {
    version: String,
    brain_server: String,
    brain_id: String,
    requested_by_public_key_hex: String,
    public_keys_hex: Vec<String>,
}
struct Stub {
    url: String,
    seen: Arc<Mutex<Vec<Value>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Stub {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn server(mode: &'static str) -> Stub {
    let seen = Arc::new(Mutex::new(vec![]));
    let recorded = seen.clone();
    let (address_tx, address_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            address_tx.send(format!("http://{}",listener.local_addr().unwrap())).unwrap();
            let handler=move|axum::Json(body):axum::Json<Value>|{
                let recorded=recorded.clone();async move{
                    recorded.lock().unwrap().push(body.clone());
                    let error=|status,message:&str|(status,axum::Json(json!({"error":message,"correlation_id":"test"})));
                    if mode=="unauthorized" {return error(axum::http::StatusCode::UNAUTHORIZED,"no");}
                    if mode=="other400" {return error(axum::http::StatusCode::BAD_REQUEST,"v3 participating subset is required");}
                    if mode=="version" && body["version"]==DESCRIPTIONS_VERSION_V3 {return error(axum::http::StatusCode::BAD_REQUEST,UNSUPPORTED_VERSION_ERROR);}
                    if mode=="old" || mode=="version" || mode=="slow" {
                        if mode=="slow" {tokio::time::sleep(Duration::from_millis(1800)).await;}
                        let Ok(legacy)=serde_json::from_value::<LegacyWire>(body.clone()) else{return error(axum::http::StatusCode::BAD_REQUEST,"invalid descriptions request body");};
                        assert_eq!(legacy.brain_server,"https://brain.test");assert!(canonical_hex(&legacy.requested_by_public_key_hex));
                        if legacy.version==DESCRIPTIONS_VERSION_V2 {return error(axum::http::StatusCode::BAD_REQUEST,UNSUPPORTED_VERSION_ERROR);}
                        return (axum::http::StatusCode::OK,axum::Json(json!({"version":legacy.version,"brainId":legacy.brain_id,
                            "checkedAt":"2026-01-01T00:00:00Z","results":legacy.public_keys_hex.iter().map(|key|json!({"publicKeyHex":key,"state":"notShared"})).collect::<Vec<_>>()})));
                    }
                    let response=json!({"version":DESCRIPTIONS_VERSION_V3,"brainId":body["brainId"],"checkedAt":"2026-01-01T00:00:00Z",
                        "results":body["publicKeysHex"].as_array().unwrap().iter().map(|key|json!({"publicKeyHex":key,"state":"notShared"})).collect::<Vec<_>>()});
                    (axum::http::StatusCode::OK,axum::Json(response))
                }
            };
            let app=axum::Router::new().route(DESCRIPTIONS_PATH,axum::routing::post(handler));
            axum::serve(listener,app).with_graceful_shutdown(async{let _=stop_rx.await;}).await.unwrap();
        });
    });
    Stub {
        url: address_rx.recv().unwrap(),
        seen,
        stop: Some(stop_tx),
        thread: Some(thread),
    }
}
#[test]
fn http_negotiation_reconstructs_only_the_subset_and_preserves_first_rejection() {
    for (mode, negotiation, count) in [
        ("direct", "direct", 1),
        ("old", "afterSchemaRejection", 3),
        ("version", "afterVersionRejection", 3),
    ] {
        let stub = server(mode);
        let lookup = http_scoped_description_lookup(&stub.url, "credential".to_owned()).unwrap();
        let (mut request, _) = fixture();
        request.participating_keys = vec![request.keys[0].clone(), request.keys[5].clone()];
        let report = lookup(&request);
        assert!(report.result.is_ok());
        assert_eq!(report.negotiation, negotiation);
        let seen = stub.seen.lock().unwrap();
        assert_eq!(seen.len(), count);
        assert_eq!(seen[0]["publicKeysHex"], json!(request.keys));
        for body in seen.iter().skip(1) {
            assert!(body.get("participatingPublicKeysHex").is_none());
            assert_eq!(body["publicKeysHex"], json!(request.participating_keys));
        }
    }
}
#[test]
fn http_empty_subset_and_non_negotiation_failures_do_not_issue_legacy_requests() {
    for mode in ["old", "other400", "unauthorized"] {
        let stub = server(mode);
        let lookup = http_scoped_description_lookup(&stub.url, "credential".to_owned()).unwrap();
        let (mut request, _) = fixture();
        request.participating_keys.clear();
        let report = lookup(&request);
        assert_eq!(stub.seen.lock().unwrap().len(), 1);
        if mode == "old" {
            assert!(report.legacy);
            assert!(report.result.unwrap().is_none());
        } else {
            assert!(!report.legacy);
            assert!(report.result.is_err());
        }
    }
}
#[test]
fn http_fallback_uses_one_total_deadline() {
    let stub = server("slow");
    let lookup = http_scoped_description_lookup(&stub.url, "credential".to_owned()).unwrap();
    let (request, _) = fixture();
    let start = Instant::now();
    let report = lookup(&request);
    assert!(report.result.is_err());
    assert!(start.elapsed() < Duration::from_millis(3500));
    assert!(stub.seen.lock().unwrap().len() <= 2);
}
