use super::*;
use crate::brain_identity::scoped::DESCRIPTIONS_VERSION_V3;

#[tokio::test]
async fn v3_requires_a_canonical_subset_and_semantic_errors_never_trigger_negotiation() {
    with_isolated_postgres(|db| async move {
        let app = identity_app(db.store);
        let headers = [(DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string())];
        let mut valid = description_body(&[key(1), key(2)]);
        valid["version"] = DESCRIPTIONS_VERSION_V3.into();
        valid["participatingPublicKeysHex"] = serde_json::json!([]);
        let (status, response) = post(&app, DESCRIPTIONS_PATH, &headers, serde_json::to_vec(&valid).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["version"], DESCRIPTIONS_VERSION_V3);
        assert_eq!(response["results"], serde_json::json!([
            {"publicKeyHex": key(1), "state": "notShared"}, {"publicKeyHex": key(2), "state": "notShared"}]));
        let mut cases = Vec::new();
        let mut missing = valid.clone(); missing.as_object_mut().unwrap().remove("participatingPublicKeysHex"); cases.push(missing);
        let mut outside = valid.clone(); outside["participatingPublicKeysHex"] = serde_json::json!([key(3)]); cases.push(outside);
        let mut duplicate = valid.clone(); duplicate["participatingPublicKeysHex"] = serde_json::json!([key(1),key(1)]); cases.push(duplicate);
        let mut page_duplicate = valid.clone(); page_duplicate["publicKeysHex"] = serde_json::json!([key(1),key(1)]); cases.push(page_duplicate);
        let mut legacy_extra = valid.clone(); legacy_extra["version"] = DESCRIPTIONS_VERSION_V2.into(); cases.push(legacy_extra);
        let mut extra = valid.clone(); extra["annotation"] = true.into(); cases.push(extra);
        for value in cases {
            let (status, body) = post(&app, DESCRIPTIONS_PATH, &headers, serde_json::to_vec(&value).unwrap()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let error = body["error"].as_str().unwrap();
            assert_ne!(error, "unsupported descriptions version");
            assert_ne!(error, "invalid descriptions request body");
        }
        let (status, _) = post(&app, DESCRIPTIONS_PATH, &[], serde_json::to_vec(&valid).unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }).await;
}
