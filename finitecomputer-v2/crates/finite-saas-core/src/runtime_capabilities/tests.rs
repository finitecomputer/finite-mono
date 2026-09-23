use super::*;

#[test]
fn native_chat_is_opt_in_and_legacy_wire_shape_is_preserved() {
    let legacy = serde_json::json!({
        "schema": "runtime_capabilities.v1",
        "capabilities": {
            "restart": true, "recover_known_good_chat": false,
            "runtime_upgrade": false, "stop": true, "runtime_retirement": false
        }
    });
    let mut envelope: RuntimeCapabilitiesEnvelope = serde_json::from_value(legacy.clone()).unwrap();
    assert!(!envelope.v1().native_hermes_chat);
    assert_eq!(serde_json::to_value(&envelope).unwrap(), legacy);
    let RuntimeCapabilitiesEnvelope::V1(capabilities) = &mut envelope;
    capabilities.native_hermes_chat = true;
    let upgraded = serde_json::to_value(&envelope).unwrap();
    assert_eq!(upgraded["capabilities"]["native_hermes_chat"], true);
    assert_eq!(
        serde_json::from_value::<RuntimeCapabilitiesEnvelope>(upgraded).unwrap(),
        envelope
    );
}

#[test]
fn substrate_upgrade_does_not_authorize_backup_recovery_or_retirement() {
    let placement = Some(RuntimePlacement {
        runner_class: RunnerClass::Substrate,
        runtime_resource_class: crate::RuntimeResourceClass::Vcpu2Memory4Gib,
    });
    let mut envelope = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        runtime_upgrade: true,
        ..Default::default()
    });
    assert!(validate_runtime_capabilities_policy(Some(&envelope), placement).is_ok());
    assert!(
        crate::RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Substrate],
            runtime_capabilities: Some(envelope.clone()),
            ..Default::default()
        }
        .validate_runtime_capability_policy()
        .is_ok()
    );
    assert!(validate_runtime_capabilities_policy(Some(&envelope), None).is_err());
    let RuntimeCapabilitiesEnvelope::V1(capabilities) = &mut envelope;
    capabilities.recover_known_good_chat = true;
    assert!(validate_runtime_capabilities_policy(Some(&envelope), placement).is_err());
    let RuntimeCapabilitiesEnvelope::V1(capabilities) = &mut envelope;
    capabilities.recover_known_good_chat = false;
    capabilities.runtime_retirement = true;
    assert!(validate_runtime_capabilities_policy(Some(&envelope), placement).is_err());
}
