use super::*;

#[test]
fn hosted_hermes_origins_reject_invalid_configuration() {
    for value in [
        "[]",
        r#"{"host":"https://a.example.test","host":"https://b.example.test"}"#,
        r#"{"host":null}"#,
        r#"{" Host ":"https://agents.example.test"}"#,
        r#"{"bad/host":"https://agents.example.test"}"#,
        r#"{"host":"http://agents.example.test"}"#,
        r#"{"host":"https://agents.example.test:0"}"#,
        r#"{"host":"https://agents.example.test:65536"}"#,
        r#"{"host":"https://*.example.test"}"#,
        r#"{"host":"https://{env.DOMAIN}"}"#,
        r#"{"host":"https://user:password@agents.example.test"}"#,
        r#"{"host":"https://agents.example.test/path"}"#,
        r#"{"host":"https://agents.example.test/?target=other"}"#,
        r#"{"host":"https://agents.example.test/#fragment"}"#,
    ] {
        assert!(HostedHermesOrigins::from_json(value).is_err());
    }
}

#[test]
fn hosted_hermes_locations_are_opaque_and_never_claim_readiness() {
    let origins =
        HostedHermesOrigins::from_json(r#"{"host":"https://agents.example.test:8443/"}"#).unwrap();
    let location = serde_json::to_value(origins.location("host", "agent/runtime?x=1")).unwrap();
    assert_eq!(
        location["baseUrl"],
        "https://agents.example.test:8443/runtimes/agent%2Fruntime%3Fx=1/"
    );
    assert_eq!(location["availability"], "unqualified");
    let missing = serde_json::to_value(origins.location("other-host", "agent-runtime")).unwrap();
    assert!(missing["baseUrl"].is_null());
    assert_eq!(missing["availability"], "not_configured");
}
