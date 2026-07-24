use std::sync::{Arc, Mutex};

use workos::user_management::AuthenticateWithCodeParams;
use workos::{AuthKitAuthorizationUrlParams, PublicClient, SecretString};

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
            .map_err(|_| authkit_error("secure sign in was rejected"))?;
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
    if callback_url.is_empty()
        || callback_url.len() > MAX_AUTHKIT_VALUE_BYTES
        || callback_url.chars().any(char::is_control)
    {
        return Err(authkit_error("secure sign in returned an invalid callback"));
    }
    let parsed = reqwest::Url::parse(callback_url)
        .map_err(|_| authkit_error("secure sign in returned an invalid callback"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("finite.computer")
        || parsed.path() != "/auth/ios/callback"
        || parsed.fragment().is_some()
    {
        return Err(authkit_error("secure sign in returned an invalid callback"));
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
            return Err(authkit_error("secure sign in returned an invalid callback"));
        }
    }
    if error.is_some() {
        return Err(authkit_error("secure sign in was canceled"));
    }
    let state = state
        .filter(|value| !value.is_empty())
        .ok_or_else(|| authkit_error("secure sign in returned an invalid callback"))?;
    if state != expected_state {
        return Err(authkit_error("secure sign in returned an invalid callback"));
    }
    let code = code.filter(|value| {
        !value.is_empty()
            && value.len() <= MAX_AUTHKIT_VALUE_BYTES
            && !value.chars().any(char::is_control)
    });
    code.ok_or_else(|| authkit_error("secure sign in returned an invalid callback"))
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
                "https://finite.computer/auth/ios/callback?code=a&code=b&state=expected",
                "expected"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "https://finite.computer/auth/ios/callback?error=access_denied&state=expected",
                "expected"
            )
            .is_err()
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
