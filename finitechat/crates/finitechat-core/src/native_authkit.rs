use std::sync::{Arc, Mutex};

use workos::user_management::AuthenticateWithCodeParams;
use workos::{AuthKitAuthorizationUrlParams, Error as WorkOSError, PublicClient, SecretString};

use crate::FiniteChatCoreError;

const AUTHKIT_REDIRECT_URI: &str = "https://finite.computer/auth/ios/callback";
const MAX_AUTHKIT_VALUE_BYTES: usize = 16 * 1024;

/// One short-lived native AuthKit authorization-code flow.
///
/// PKCE state, the verifier, and the resulting bearer token remain behind this
/// Rust object boundary. The iOS layer only presents `authorization_url` and
/// returns the exact callback URL from `ASWebAuthenticationSession`.
#[derive(uniffi::Object)]
pub struct NativeAuthKitSession {
    client: PublicClient,
    authorization_url: String,
    state: Mutex<NativeAuthKitState>,
}

enum NativeAuthKitState {
    AwaitingCallback {
        code_verifier: String,
        state: String,
    },
    Exchanging,
    Authenticated {
        access_token: SecretString,
    },
}

#[uniffi::export]
impl NativeAuthKitSession {
    #[uniffi::constructor]
    pub fn start(client_id: String) -> Result<Arc<Self>, FiniteChatCoreError> {
        validate_client_id(&client_id)?;
        let client = PublicClient::new(client_id);
        let authorization = client
            .authkit_authorization_url(AuthKitAuthorizationUrlParams {
                redirect_uri: AUTHKIT_REDIRECT_URI.to_owned(),
                provider: Some("authkit".to_owned()),
                ..Default::default()
            })
            .map_err(|_| authkit_error("could not start secure sign in"))?;

        Ok(Arc::new(Self {
            client,
            authorization_url: authorization.url,
            state: Mutex::new(NativeAuthKitState::AwaitingCallback {
                code_verifier: authorization.code_verifier,
                state: authorization.state,
            }),
        }))
    }

    pub fn authorization_url(&self) -> String {
        self.authorization_url.clone()
    }

    /// Validate the redirect binding and exchange its single-use code.
    ///
    /// The refresh token returned by WorkOS is intentionally dropped. This
    /// flow only needs a bearer token for the bounded device-link session.
    pub fn complete(&self, callback_url: String) -> Result<(), FiniteChatCoreError> {
        let (expected_state, code_verifier) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
            let values = match &*state {
                NativeAuthKitState::AwaitingCallback {
                    code_verifier,
                    state,
                } => (state.clone(), code_verifier.clone()),
                NativeAuthKitState::Exchanging | NativeAuthKitState::Authenticated { .. } => {
                    return Err(authkit_error("secure sign in is already complete"));
                }
            };
            *state = NativeAuthKitState::Exchanging;
            values
        };
        let code = parse_callback(&callback_url, &expected_state)?;
        let mut params = AuthenticateWithCodeParams::new(code);
        params.code_verifier = Some(code_verifier);
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|_| authkit_error("could not finish secure sign in"))?;
        let response = runtime
            .block_on(
                self.client
                    .client()
                    .user_management()
                    .authenticate_with_code(params),
            )
            .map_err(authkit_exchange_error)?;
        validate_access_token(response.access_token.expose())?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        *state = NativeAuthKitState::Authenticated {
            access_token: response.access_token,
        };
        Ok(())
    }
}

impl NativeAuthKitSession {
    pub(crate) fn access_token(&self) -> Result<String, FiniteChatCoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        match &*state {
            NativeAuthKitState::Authenticated { access_token } => {
                Ok(access_token.expose().to_owned())
            }
            NativeAuthKitState::AwaitingCallback { .. } | NativeAuthKitState::Exchanging => {
                Err(authkit_error("secure sign in is not complete"))
            }
        }
    }
}

fn parse_callback(callback_url: &str, expected_state: &str) -> Result<String, FiniteChatCoreError> {
    if callback_url.is_empty() {
        return Err(invalid_callback("empty"));
    }
    if callback_url.len() > MAX_AUTHKIT_VALUE_BYTES {
        return Err(invalid_callback("oversize"));
    }
    if callback_url.chars().any(char::is_control) {
        return Err(invalid_callback("control_character"));
    }
    let parsed =
        reqwest::Url::parse(callback_url).map_err(|_| invalid_callback("malformed_url"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("finite.computer")
        || parsed.path() != "/auth/ios/callback"
        || parsed.fragment().is_some()
    {
        return Err(invalid_callback("redirect_mismatch"));
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in parsed.query_pairs() {
        let destination = match key.as_ref() {
            "code" => &mut code,
            "state" => &mut state,
            "error" => &mut error,
            _ => continue,
        };
        if destination.replace(value.into_owned()).is_some() {
            return Err(invalid_callback("duplicate_parameter"));
        }
    }
    let state = state
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_callback("missing_state"))?;
    if state != expected_state {
        return Err(invalid_callback("state_mismatch"));
    }
    if code.is_some() && error.is_some() {
        return Err(invalid_callback("code_and_error"));
    }
    if let Some(error) = error {
        return Err(authkit_authorization_error(&error));
    }
    let code = code.filter(|value| {
        !value.is_empty()
            && value.len() <= MAX_AUTHKIT_VALUE_BYTES
            && !value.chars().any(char::is_control)
    });
    code.ok_or_else(|| invalid_callback("missing_or_invalid_code"))
}

fn invalid_callback(reason: &'static str) -> FiniteChatCoreError {
    // The reason is a closed, value-free diagnostic class. It deliberately
    // never contains the authorization code, OAuth state, or callback URL.
    authkit_error(format!(
        "secure sign in returned an invalid callback ({reason})"
    ))
}

fn validate_client_id(value: &str) -> Result<(), FiniteChatCoreError> {
    if !value.starts_with("client_")
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(authkit_error("AuthKit is not configured"))
    } else {
        Ok(())
    }
}

fn validate_access_token(value: &str) -> Result<(), FiniteChatCoreError> {
    if value.is_empty()
        || value.len() > MAX_AUTHKIT_VALUE_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(authkit_error("secure sign in returned an invalid session"))
    } else {
        Ok(())
    }
}

fn authkit_authorization_error(code: &str) -> FiniteChatCoreError {
    match code {
        "access_denied" => authkit_error("secure sign in was denied"),
        "server_error" | "temporarily_unavailable" => {
            authkit_error("secure sign in is temporarily unavailable")
        }
        code if is_safe_authkit_error_code(code) => {
            authkit_error(format!("secure sign in could not be completed ({code})"))
        }
        _ => authkit_error("secure sign in could not be completed"),
    }
}

fn authkit_exchange_error(error: WorkOSError) -> FiniteChatCoreError {
    match error {
        WorkOSError::Network(error) if error.is_retryable() => {
            authkit_error("secure sign in is temporarily unavailable")
        }
        WorkOSError::Api(error) if error.status == 429 || error.status >= 500 => {
            authkit_error("secure sign in is temporarily unavailable")
        }
        WorkOSError::Api(error) => match error.code.as_deref() {
            Some("invalid_grant" | "authorization_code_expired") => {
                authkit_error("secure sign in expired")
            }
            Some(code) if is_safe_authkit_error_code(code) => {
                authkit_error(format!("secure sign in was rejected ({code})"))
            }
            _ => authkit_error("secure sign in was rejected"),
        },
        WorkOSError::Decode(_) => authkit_error("secure sign in returned an invalid session"),
        _ => authkit_error("could not finish secure sign in"),
    }
}

fn is_safe_authkit_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn authkit_error(reason: impl Into<String>) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_authkit_url_uses_fixed_callback_and_pkce() {
        let session = NativeAuthKitSession::start("client_test_123".to_owned()).unwrap();
        let url = reqwest::Url::parse(&session.authorization_url()).unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://api.workos.com/user_management/authorize")
        );
        assert_eq!(
            query.get("client_id").map(|value| value.as_ref()),
            Some("client_test_123")
        );
        assert_eq!(
            query.get("redirect_uri").map(|value| value.as_ref()),
            Some(AUTHKIT_REDIRECT_URI)
        );
        assert_eq!(
            query.get("response_type").map(|value| value.as_ref()),
            Some("code")
        );
        assert_eq!(
            query.get("provider").map(|value| value.as_ref()),
            Some("authkit")
        );
        assert_eq!(
            query
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert!(
            query
                .get("code_challenge")
                .is_some_and(|value| !value.is_empty())
        );
        assert!(query.get("state").is_some_and(|value| !value.is_empty()));
        assert!(!session.authorization_url().contains("code_verifier"));
    }

    #[test]
    fn callback_requires_exact_redirect_state_and_single_values() {
        assert_eq!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=abc&state=expected",
                "expected",
            )
            .unwrap(),
            "abc"
        );
        assert!(
            parse_callback(
                "finitechat://auth/callback?code=abc&state=expected",
                "expected",
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/other?code=abc&state=expected",
                "expected"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=abc&state=wrong",
                "expected"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=abc&state=wrong",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("state_mismatch")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=a&code=b&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate_parameter")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=access_denied&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("was denied")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=access_denied&state=wrong",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("invalid callback")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=abc",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("missing_state")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("missing_or_invalid_code")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?code=abc&error=access_denied&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("invalid callback")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=temporarily_unavailable&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("temporarily unavailable")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=organization_invalid&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .contains("organization_invalid")
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=%0Asecret&state=expected",
                "expected"
            )
            .unwrap_err()
            .to_string()
            .ends_with("could not be completed")
        );
    }

    #[test]
    fn client_id_and_token_validation_reject_unsafe_values() {
        assert!(validate_client_id("client_test_123").is_ok());
        assert!(validate_client_id("sk_secret").is_err());
        assert!(validate_client_id(" client_test_123").is_err());
        assert!(validate_access_token("header.payload.signature").is_ok());
        assert!(validate_access_token(" token").is_err());
        assert!(validate_access_token("token\n").is_err());
    }
}
