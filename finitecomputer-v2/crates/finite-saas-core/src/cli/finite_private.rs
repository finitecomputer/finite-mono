use super::*;

pub(crate) async fn finite_private_grant_approve(
    email: String,
    workos_user_id: Option<String>,
    limit_profile_id: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: email,
            workos_user_id,
            limit_profile_id,
            now,
        })
        .await
        .map_err(Into::into)
}

pub(crate) struct FinitePrivateFriendKeyIssueArgs {
    pub(crate) email: String,
    pub(crate) workos_user_id: Option<String>,
    pub(crate) limit_profile_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) agent_runtime_id: Option<String>,
    pub(crate) raw_key_env: Option<String>,
    pub(crate) now: Option<String>,
    pub(crate) mode: ImportMode,
}

pub(crate) async fn finite_private_friend_key_issue(
    input: FinitePrivateFriendKeyIssueArgs,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(input.mode).await?;
    let raw_key = raw_key_from_env_or_generate(input.raw_key_env.as_deref())?;
    let issued = store
        .issue_finite_private_friend_key(IssueFinitePrivateFriendKeyInput {
            verified_email: input.email,
            workos_user_id: input.workos_user_id,
            limit_profile_id: input.limit_profile_id,
            raw_key: raw_key.value.clone(),
            project_id: input.project_id,
            agent_runtime_id: input.agent_runtime_id,
            now: input.now,
        })
        .await?;
    Ok(issued_key_output(
        Some(issued.grant),
        issued.api_key,
        raw_key,
    ))
}

pub(crate) async fn finite_private_api_key_issue(
    grant_id: String,
    project_id: Option<String>,
    agent_runtime_id: Option<String>,
    raw_key_env: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(mode).await?;
    let raw_key = raw_key_from_env_or_generate(raw_key_env.as_deref())?;
    let api_key = store
        .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id,
            raw_key: raw_key.value.clone(),
            project_id,
            agent_runtime_id,
            now,
        })
        .await?;
    Ok(issued_key_output(None, api_key, raw_key))
}

pub(crate) async fn finite_private_api_key_rotate(
    key_id: String,
    raw_key_env: Option<String>,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateIssuedKeyOutput> {
    let store = core_store_for_mode(mode).await?;
    let raw_key = raw_key_from_env_or_generate(raw_key_env.as_deref())?;
    let api_key = store
        .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
            key_id,
            raw_key: raw_key.value.clone(),
            now,
        })
        .await?;
    Ok(issued_key_output(None, api_key, raw_key))
}

pub(crate) async fn finite_private_api_key_revoke(
    key_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateApiKey> {
    let store = core_store_for_mode(mode).await?;
    store
        .revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput { key_id, now })
        .await
        .map_err(Into::into)
}

pub(crate) async fn finite_private_grant_revoke(
    grant_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .revoke_finite_private_grant(RevokeFinitePrivateGrantInput { grant_id, now })
        .await
        .map_err(Into::into)
}

pub(crate) async fn finite_private_window_reset(
    grant_id: String,
    now: Option<String>,
    mode: ImportMode,
) -> Result<FinitePrivateGrant> {
    let store = core_store_for_mode(mode).await?;
    store
        .reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput { grant_id, now })
        .await
        .map_err(Into::into)
}

/// Build the store for an admin command.
///
/// Both modes talk to real Postgres. A dry run reads committed production
/// state and rolls its writes back, so the preview reflects what the operator
/// is actually about to change.
pub(crate) async fn core_store_for_mode(mode: ImportMode) -> Result<CoreStore> {
    postgres_store_from_env(mode).await
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct FinitePrivateIssuedKeyOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) grant: Option<FinitePrivateGrant>,
    pub(crate) api_key: FinitePrivateApiKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) raw_api_key: Option<String>,
    pub(crate) raw_api_key_generated: bool,
    pub(crate) raw_api_key_note: &'static str,
}

pub(crate) struct RawKeyMaterial {
    pub(crate) value: String,
    pub(crate) generated: bool,
}

pub(crate) fn issued_key_output(
    grant: Option<FinitePrivateGrant>,
    api_key: FinitePrivateApiKey,
    raw_key: RawKeyMaterial,
) -> FinitePrivateIssuedKeyOutput {
    FinitePrivateIssuedKeyOutput {
        grant,
        api_key,
        raw_api_key: raw_key.generated.then_some(raw_key.value),
        raw_api_key_generated: raw_key.generated,
        raw_api_key_note: if raw_key.generated {
            "Raw key is shown once. Store it in a secret manager before sending it."
        } else {
            "Raw key was read from raw_key_env and is not echoed."
        },
    }
}

pub(crate) fn raw_key_from_env_or_generate(raw_key_env: Option<&str>) -> Result<RawKeyMaterial> {
    if let Some(env_name) = raw_key_env {
        return Ok(RawKeyMaterial {
            value: required_env(env_name)?,
            generated: false,
        });
    }
    Ok(RawKeyMaterial {
        value: generate_finite_private_api_key()?,
        generated: true,
    })
}

pub(crate) fn generate_finite_private_api_key() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).context("failed to generate Finite Private API key")?;
    let mut key = String::with_capacity("fpk_live_".len() + bytes.len() * 2);
    key.push_str("fpk_live_");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}")?;
    }
    Ok(key)
}
