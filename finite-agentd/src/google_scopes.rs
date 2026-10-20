use std::collections::BTreeSet;

use crate::AgentdError;

/// The dashboard binds the connected email through Google's userinfo endpoint.
const REQUIRED_IDENTITY_SCOPES: [&str; 2] =
    ["openid", "https://www.googleapis.com/auth/userinfo.email"];
const IDENTITY_SCOPES: [&str; 3] = [
    "openid",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
];

/// People may untick individual permissions on Google's consent screen.
/// Accept any grant within the installed contract that keeps the account
/// identity and some Workspace access. Callers must record exactly these:
/// the Python helper requests its recorded scopes on refresh.
pub(crate) fn accepted_google_scopes(
    supplied: &[String],
    contract: &BTreeSet<String>,
) -> Result<BTreeSet<String>, AgentdError> {
    let granted = supplied.iter().cloned().collect::<BTreeSet<_>>();
    if !granted.is_subset(contract) {
        return Err(AgentdError::InvalidPayload(
            "Google granted access outside the Finite Workspace contract".to_owned(),
        ));
    }
    if REQUIRED_IDENTITY_SCOPES
        .iter()
        .any(|scope| !granted.contains(*scope))
    {
        return Err(AgentdError::InvalidPayload(
            "Google did not share the connected account email".to_owned(),
        ));
    }
    if granted
        .iter()
        .all(|scope| IDENTITY_SCOPES.contains(&scope.as_str()))
    {
        return Err(AgentdError::InvalidPayload(
            "Google did not grant any Finite Workspace access".to_owned(),
        ));
    }
    Ok(granted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVE: &str = "https://www.googleapis.com/auth/drive";
    const GMAIL_SEND: &str = "https://www.googleapis.com/auth/gmail.send";
    const EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";
    const PROFILE: &str = "https://www.googleapis.com/auth/userinfo.profile";

    fn contract() -> BTreeSet<String> {
        BTreeSet::from([DRIVE, GMAIL_SEND, "openid", EMAIL, PROFILE].map(str::to_owned))
    }

    fn grant(scopes: &[&str]) -> Vec<String> {
        scopes.iter().map(|scope| (*scope).to_owned()).collect()
    }

    #[test]
    fn a_partial_grant_that_keeps_the_account_email_is_accepted() {
        let full = grant(&[DRIVE, GMAIL_SEND, "openid", EMAIL, PROFILE]);
        assert_eq!(
            accepted_google_scopes(&full, &contract()).unwrap(),
            contract()
        );

        let without_send = grant(&["openid", EMAIL, DRIVE, DRIVE]);
        assert_eq!(
            accepted_google_scopes(&without_send, &contract()).unwrap(),
            BTreeSet::from(["openid", EMAIL, DRIVE].map(str::to_owned))
        );
    }

    #[test]
    fn grants_outside_the_contract_or_without_workspace_access_fail_closed() {
        for scopes in [
            &[
                "openid",
                EMAIL,
                DRIVE,
                "https://www.googleapis.com/auth/script.projects",
            ][..],
            &["openid", DRIVE],
            &[EMAIL, DRIVE],
            &["openid", EMAIL, PROFILE],
            &[],
        ] {
            assert!(
                matches!(
                    accepted_google_scopes(&grant(scopes), &contract()),
                    Err(AgentdError::InvalidPayload(_))
                ),
                "{scopes:?}"
            );
        }
    }
}
