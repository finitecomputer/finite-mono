use super::*;
use crate::support::{
    SubmitSupportResult, SupportInput, SupportReceipt, SupportService, valid_email,
};
use axum::Extension;

pub(super) async fn submit_support(
    State(state): State<CoreApiState>,
    service: Option<Extension<SupportService>>,
    headers: HeaderMap,
    Json(input): Json<SupportInput>,
) -> Result<(StatusCode, Json<SupportReceipt>), ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let Some(Extension(service)) = service else {
        return Err(ApiError::service_unavailable(
            "Support reports are unavailable. Please email your support contact directly.",
        ));
    };
    if input.support_email != service.email || input.reply_to != identity.email {
        return Err(ApiError::conflict(
            "Your support contact or reply email changed. Reload and review the report again.",
        ));
    }
    if input.message.trim().is_empty()
        || input.message.chars().count() > 4000
        || input.message.contains('\0')
        || !valid_email(&identity.email)
        || input.idempotency_key.is_empty()
        || input.idempotency_key.len() > 128
        || !input
            .idempotency_key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(ApiError::bad_request(
            "Enter a report of 1–4000 characters with a valid request identifier.",
        ));
    }
    link_verified_identity(&state, &identity).await?;
    if let Some(project_id) = &input.project_id {
        let visible = state
            .store
            .visible_projects_for_workos_user(&identity.workos_user_id)
            .await?;
        if !visible
            .iter()
            .any(|project| &project.project.id == project_id)
        {
            return Err(ApiError::not_found("Agent was not found."));
        }
    }
    match state
        .store
        .submit_support(&identity.workos_user_id, &input)
        .await?
    {
        SubmitSupportResult::Receipt(receipt) => Ok((StatusCode::ACCEPTED, Json(receipt))),
        SubmitSupportResult::Conflict => Err(ApiError::conflict(
            "This request identifier already belongs to a different report.",
        )),
        SubmitSupportResult::RateLimited => Err(ApiError::too_many_requests(
            "You have sent five reports in the last hour. Please email support directly or try again later.",
        )),
    }
}
