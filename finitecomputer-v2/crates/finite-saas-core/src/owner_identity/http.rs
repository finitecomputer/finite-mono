//! WorkOS user management and the hosted chat device over HTTP.

use super::*;

const DEFAULT_WORKOS_API_BASE_URL: &str = "https://api.workos.com";

/// WorkOS (`WORKOS_API_KEY`, `WORKOS_API_BASE_URL`,
/// `FC_WORKOS_OPERATOR_ORG_ID`, as Core's authenticator reads them) and the
/// hosted device (`FC_HOSTED_WEB_DEVICE_URL`, `FINITECHAT_HOSTED_API_TOKEN`,
/// as the dashboard reads them). Deliberately not `Debug`: it holds secrets.
pub struct HttpOwnerIdentityBackend {
    client: Client,
    workos_base: Url,
    workos_api_key: String,
    operator_org_id: String,
    hosted_base: Url,
    hosted_token: String,
}

#[derive(Deserialize)]
struct ListResponse<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct Membership {
    user_id: String,
    organization_id: String,
    status: String,
}

#[derive(Deserialize)]
struct HostedState {
    identity: Option<HostedIdentity>,
}

#[derive(Deserialize)]
struct HostedIdentity {
    account_id: Option<String>,
}

impl HttpOwnerIdentityBackend {
    pub fn new(
        workos_api_base_url: &str,
        workos_api_key: &str,
        operator_org_id: &str,
        hosted_device_url: &str,
        hosted_api_token: &str,
    ) -> CoreResult<Self> {
        let required = |name: &str, value: &str| {
            let value = value.trim();
            if value.is_empty() {
                Err(CoreError::OwnerIdentityUnavailable(format!(
                    "{name} is required"
                )))
            } else {
                Ok(value.to_string())
            }
        };
        Ok(Self {
            client: http_client()?,
            workos_base: parse_base_url("WORKOS_API_BASE_URL", workos_api_base_url)?,
            workos_api_key: required("WORKOS_API_KEY", workos_api_key)?,
            operator_org_id: required("FC_WORKOS_OPERATOR_ORG_ID", operator_org_id)?,
            hosted_base: parse_base_url("FC_HOSTED_WEB_DEVICE_URL", hosted_device_url)?,
            hosted_token: required("FINITECHAT_HOSTED_API_TOKEN", hosted_api_token)?,
        })
    }

    pub fn from_env() -> CoreResult<Option<Self>> {
        let env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let (hosted_url, hosted_token) = match (
            env("FC_HOSTED_WEB_DEVICE_URL"),
            env("FINITECHAT_HOSTED_API_TOKEN"),
        ) {
            (None, None) => return Ok(None),
            (Some(url), Some(token)) => (url, token),
            _ => {
                return Err(CoreError::OwnerIdentityUnavailable(
                    "set both FC_HOSTED_WEB_DEVICE_URL and FINITECHAT_HOSTED_API_TOKEN, or neither"
                        .into(),
                ));
            }
        };
        Self::new(
            &env("WORKOS_API_BASE_URL").unwrap_or_else(|| DEFAULT_WORKOS_API_BASE_URL.into()),
            &env("WORKOS_API_KEY").unwrap_or_default(),
            &env("FC_WORKOS_OPERATOR_ORG_ID").unwrap_or_default(),
            &hosted_url,
            &hosted_token,
        )
        .map(Some)
    }

    fn workos_url(&self, segments: &[&str]) -> CoreResult<Url> {
        let mut url = self.workos_base.clone();
        url.path_segments_mut()
            .map_err(|_| unavailable("WorkOS", None))?
            .clear()
            .extend(segments);
        Ok(url)
    }

    async fn workos_json<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> CoreResult<Option<T>> {
        let response = request
            .bearer_auth(&self.workos_api_key)
            .send()
            .await
            .map_err(|_| unavailable("WorkOS", None))?;
        match response.status() {
            status if status.is_success() => response
                .json::<T>()
                .await
                .map(Some)
                .map_err(|_| unavailable("WorkOS", Some(status))),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(unavailable("WorkOS", Some(status))),
        }
    }
}

impl OwnerIdentityBackend for HttpOwnerIdentityBackend {
    fn workos_users_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<Vec<WorkosUserRecord>>> {
        Box::pin(async move {
            let mut url = self.workos_url(&["user_management", "users"])?;
            url.query_pairs_mut()
                .append_pair("email", email)
                .append_pair("limit", "10");
            Ok(self
                .workos_json::<ListResponse<WorkosUserRecord>>(self.client.get(url))
                .await?
                .map(|list| list.data)
                .unwrap_or_default())
        })
    }

    fn create_workos_user<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<WorkosUserRecord>> {
        Box::pin(async move {
            let url = self.workos_url(&["user_management", "users"])?;
            // Unverified: the person proves the inbox at their first sign-in.
            let body = serde_json::json!({ "email": email, "email_verified": false });
            self.workos_json::<WorkosUserRecord>(self.client.post(url).json(&body))
                .await?
                .ok_or_else(|| unavailable("WorkOS user creation", Some(StatusCode::NOT_FOUND)))
        })
    }

    fn workos_user<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, CoreResult<Option<WorkosUserRecord>>> {
        Box::pin(async move {
            if id.is_empty() {
                return Ok(None);
            }
            let url = self.workos_url(&["user_management", "users", id])?;
            self.workos_json(self.client.get(url)).await
        })
    }

    fn operator_membership_active<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<bool>> {
        Box::pin(async move {
            let mut url = self.workos_url(&["user_management", "organization_memberships"])?;
            url.query_pairs_mut()
                .append_pair("user_id", workos_user_id)
                .append_pair("organization_id", &self.operator_org_id)
                .append_pair("statuses", "active");
            Ok(self
                .workos_json::<ListResponse<Membership>>(self.client.get(url))
                .await?
                .is_some_and(|list| {
                    list.data.iter().any(|membership| {
                        membership.user_id == workos_user_id
                            && membership.organization_id == self.operator_org_id
                            && membership.status == "active"
                    })
                }))
        })
    }

    fn hosted_chat_account_id<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<String>> {
        Box::pin(async move {
            let mut url = self.hosted_base.clone();
            url.set_path("/v1/app/state");
            let response = self
                .client
                .get(url)
                .bearer_auth(&self.hosted_token)
                .header("x-finite-workos-user-id", workos_user_id)
                .header("accept", "application/json")
                .send()
                .await
                .map_err(|_| unavailable("hosted chat", None))?;
            let status = response.status();
            if !status.is_success() {
                return Err(unavailable("hosted chat", Some(status)));
            }
            response
                .json::<HostedState>()
                .await
                .map_err(|_| unavailable("hosted chat", Some(status)))?
                .identity
                .and_then(|identity| identity.account_id)
                .map(|id| id.trim().to_ascii_lowercase())
                .ok_or_else(|| {
                    CoreError::OwnerIdentityUnavailable("hosted chat has no identity".into())
                })
        })
    }
}
