//! `fbrain admin member remove` pre-flight: ask the server which native
//! Folders the removal must rotate.
//!
//! The server owns the rule (Folders the Member reads plus Folders where it
//! still holds the current key without entitlement) and enforces the same set
//! on removal. The client cannot compute the second half from Brain metadata,
//! which carries no per-key grant detail, so it asks instead of guessing.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{CliEnvironment, CliError, signed_json_request};

/// Server answer to `GET .../members/{target}/removal-plan`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemberRemovalPlan {
    brain_id: String,
    target_npub: String,
    pub(crate) folder_rotations: Vec<PlannedFolderRotation>,
}

/// One native Folder the removal must rotate; every planned Folder is
/// rotated whatever its reason.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlannedFolderRotation {
    pub(crate) folder_id: String,
    pub(crate) current_key_version: u32,
    pub(crate) reason: MemberRemovalRotationReason,
}

/// Why the server planned a Folder. An unknown reason fails decoding, so a
/// future server rule cannot be silently treated as one of these.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MemberRemovalRotationReason {
    /// The Member reads the Folder today.
    Readable,
    /// The Member holds the current key without entitlement
    /// (`revocationIncomplete`).
    StaleCurrentGrant,
}

pub(crate) fn member_removal_plan_route(brain_id: &str, target_npub: &str) -> String {
    format!("/v1/admin/brains/{brain_id}/members/{target_npub}/removal-plan")
}

/// Fetch and check the removal plan. Fails closed against a server that
/// predates the plan route: such a server would accept a removal that leaves
/// stale current wraps in place, which is exactly what this command prevents.
pub(crate) fn fetch_member_removal_plan(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
    target_npub: &str,
) -> Result<MemberRemovalPlan, CliError> {
    let route = member_removal_plan_route(brain_id, target_npub);
    let response = signed_json_request(env, args, "GET", &route, None)
        .map_err(|error| unsupported_plan_server(error, &route))?;
    let plan: MemberRemovalPlan = serde_json::from_value(response)
        .map_err(|error| CliError::HttpResponseDecode(error.to_string()))?;
    validate_plan(plan, brain_id, target_npub)
}

fn validate_plan(
    plan: MemberRemovalPlan,
    brain_id: &str,
    target_npub: &str,
) -> Result<MemberRemovalPlan, CliError> {
    let distinct = plan
        .folder_rotations
        .iter()
        .map(|rotation| rotation.folder_id.as_str())
        .collect::<BTreeSet<_>>();
    if plan.brain_id != brain_id
        || plan.target_npub != target_npub
        || distinct.len() != plan.folder_rotations.len()
    {
        return Err(CliError::InvalidInput(
            "Member removal plan did not match the requested Brain and target".to_owned(),
        ));
    }
    Ok(plan)
}

/// An older Brain server answers the plan route with its unknown-route 404
/// (or 405). Nothing was changed; say so instead of surfacing a raw error.
fn unsupported_plan_server(error: CliError, route: &str) -> CliError {
    match error {
        CliError::HttpStatus { status: 404, body } if body.contains("route not found") => {
            unsupported_plan(route)
        }
        CliError::HttpStatus { status: 405, .. } => unsupported_plan(route),
        other => other,
    }
}

fn unsupported_plan(route: &str) -> CliError {
    CliError::Unsupported(format!(
        "this Brain server cannot say which Folders member removal must rotate (GET {route}); nothing was changed. Upgrade the Brain server before removing members with this fbrain"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(brain_id: &str, target: &str, folders: &[&str]) -> MemberRemovalPlan {
        MemberRemovalPlan {
            brain_id: brain_id.to_owned(),
            target_npub: target.to_owned(),
            folder_rotations: folders
                .iter()
                .map(|folder_id| PlannedFolderRotation {
                    folder_id: (*folder_id).to_owned(),
                    current_key_version: 1,
                    reason: MemberRemovalRotationReason::StaleCurrentGrant,
                })
                .collect(),
        }
    }

    #[test]
    fn plan_decodes_the_server_contract() {
        let decoded: MemberRemovalPlan = serde_json::from_value(serde_json::json!({
            "brainId": "org",
            "targetNpub": "npub-removed",
            "folderRotations": [
                {"folderId": "general", "currentKeyVersion": 1, "reason": "readable"},
                {"folderId": "research", "currentKeyVersion": 3, "reason": "staleCurrentGrant"}
            ]
        }))
        .unwrap();
        let checked = validate_plan(decoded, "org", "npub-removed").unwrap();
        assert_eq!(checked.folder_rotations.len(), 2);
        assert_eq!(checked.folder_rotations[1].folder_id, "research");
        assert_eq!(checked.folder_rotations[1].current_key_version, 3);
        assert_eq!(
            checked.folder_rotations[0].reason,
            MemberRemovalRotationReason::Readable
        );
        assert_eq!(
            checked.folder_rotations[1].reason,
            MemberRemovalRotationReason::StaleCurrentGrant
        );
    }

    #[test]
    fn plan_with_an_unknown_reason_fails_closed() {
        let decoded = serde_json::from_value::<MemberRemovalPlan>(serde_json::json!({
            "brainId": "org",
            "targetNpub": "npub-removed",
            "folderRotations": [
                {"folderId": "general", "currentKeyVersion": 1, "reason": "somethingNew"}
            ]
        }));
        assert!(decoded.is_err());
    }

    #[test]
    fn plan_for_another_brain_target_or_with_duplicates_is_refused() {
        for (candidate, brain, target) in [
            (plan("other", "npub-removed", &["a"]), "org", "npub-removed"),
            (plan("org", "npub-someone", &["a"]), "org", "npub-removed"),
            (
                plan("org", "npub-removed", &["a", "a"]),
                "org",
                "npub-removed",
            ),
        ] {
            let error = validate_plan(candidate, brain, target).unwrap_err();
            assert!(
                error.to_string().contains("did not match"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn old_server_without_the_plan_route_fails_closed_with_upgrade_guidance() {
        let route = member_removal_plan_route("org", "npub-removed");
        for error in [
            CliError::HttpStatus {
                status: 404,
                body: r#"{"error":"Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"}"#.to_owned(),
            },
            CliError::HttpStatus {
                status: 405,
                body: String::new(),
            },
        ] {
            let mapped = unsupported_plan_server(error, &route).to_string();
            assert!(mapped.contains("nothing was changed"), "{mapped}");
            assert!(mapped.contains("Upgrade the Brain server"), "{mapped}");
            assert!(mapped.contains("/removal-plan"), "{mapped}");
        }
        // Any other failure, such as a refused non-member target, passes through.
        let refused = CliError::HttpStatus {
            status: 400,
            body: "broken invariant: brain member does not exist".to_owned(),
        };
        assert!(matches!(
            unsupported_plan_server(refused, &route),
            CliError::HttpStatus { status: 400, .. }
        ));
    }
}
