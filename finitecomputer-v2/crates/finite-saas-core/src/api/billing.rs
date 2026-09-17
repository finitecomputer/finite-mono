use super::*;

pub(super) async fn billing_overview(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<BillingOverview>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: identity.email,
                workos_user_id: identity.workos_user_id,
                now: None,
            })
            .await?,
    ))
}

pub(super) async fn link_stripe_customer(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LinkStripeCustomerRequest>,
) -> Result<Json<CustomerBillingAccount>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: identity.email,
                workos_user_id: identity.workos_user_id,
                stripe_customer_id: input.stripe_customer_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn sync_stripe_subscription(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<SyncStripeSubscriptionRequest>,
) -> Result<Json<CustomerBillingAccount>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: input.customer_org_id,
                stripe_customer_id: input.stripe_customer_id,
                stripe_subscription_id: input.stripe_subscription_id,
                stripe_price_id: input.stripe_price_id,
                expected_stripe_price_id: state.standard_stripe_price_id.clone(),
                subscription_status: input.subscription_status,
                current_period_end: input.current_period_end,
                cancel_at_period_end: input.cancel_at_period_end,
                stripe_event_id: input.stripe_event_id,
                stripe_event_created: input.stripe_event_created,
                now: input.now,
            })
            .await?,
    ))
}
