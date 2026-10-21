use super::*;

pub(crate) fn http_scoped_description_lookup(
    base_url: &str,
    credential: String,
) -> Result<ScopedDescriptionLookup, String> {
    let url = format!(
        "{}{DESCRIPTIONS_PATH}",
        validate_private_base_url(base_url)?
    );
    Ok(Arc::new(move |request| {
        let mut report = ScopedLookupReport {
            result: Ok(None),
            legacy: false,
            negotiation: "direct",
        };
        if let Err(error) = request.validate() {
            report.result = Err(error);
            return report;
        }
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .timeout(CORE_TOTAL_TIMEOUT)
            .redirects(0)
            .build();
        let deadline = Instant::now() + CORE_TOTAL_TIMEOUT;
        match post_v3(&agent, &url, &credential, request, deadline) {
            Ok(response) => {
                report.result = validate_scoped_response(request, response).map(Some);
                return report;
            }
            Err(ScopedAttempt::Failed(failure)) => {
                report.result = Err(failure);
                return report;
            }
            Err(ScopedAttempt::Negotiate(reason)) => {
                report.legacy = true;
                report.negotiation = reason;
            }
        }
        // This constructor is the only bridge from the full page to legacy.
        let Some(legacy) = request.legacy() else {
            return report;
        };
        let post =
            |version| post_descriptions(&agent, &url, &credential, version, &legacy, deadline);
        let result = match post(DESCRIPTIONS_VERSION_V2) {
            Err(Attempt::VersionUnsupported) => post(DESCRIPTIONS_VERSION_V1),
            other => other,
        };
        report.result = result
            .map_err(|attempt| match attempt {
                Attempt::VersionUnsupported => CoreLookupFailure::Unsupported,
                Attempt::Failed(failure) => failure,
            })
            .and_then(|response| validate_core_response(&legacy, response))
            .map(|r| Some(r.into()));
        report
    }))
}

enum ScopedAttempt {
    Negotiate(&'static str),
    Failed(CoreLookupFailure),
}
fn post_v3(
    agent: &ureq::Agent,
    url: &str,
    credential: &str,
    request: &ScopedLookupRequest,
    deadline: Instant,
) -> Result<ScopedCoreResponse, ScopedAttempt> {
    let fail =
        |reason: &str| ScopedAttempt::Failed(CoreLookupFailure::Unavailable(reason.to_owned()));
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| fail("timeout"))?;
    let body = serde_json::json!({ "version": DESCRIPTIONS_VERSION_V3,
        "brainServer": request.brain_server, "brainId": request.brain_id,
        "requestedByPublicKeyHex": request.requested_by_hex, "publicKeysHex": request.keys,
        "participatingPublicKeysHex": request.participating_keys })
    .to_string();
    if body.len() > 64 * 1024 {
        return Err(fail("request too large"));
    }
    let response = match agent
        .post(url)
        .timeout(remaining)
        .set("content-type", "application/json")
        .set(DESCRIPTION_CREDENTIAL_HEADER, credential)
        .send_string(&body)
    {
        Ok(response) => response,
        Err(ureq::Error::Status(400, response)) => {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(MAX_ERROR_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| fail("error read failed"))?;
            if bytes.len() as u64 > MAX_ERROR_BYTES {
                return Err(fail("error too large"));
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| fail("malformed error"))?;
            return match body.get("error").and_then(serde_json::Value::as_str) {
                Some(UNSUPPORTED_VERSION_ERROR) => {
                    Err(ScopedAttempt::Negotiate("afterVersionRejection"))
                }
                Some("invalid descriptions request body") => {
                    Err(ScopedAttempt::Negotiate("afterSchemaRejection"))
                }
                _ => Err(fail("status 400")),
            };
        }
        Err(ureq::Error::Status(401 | 403, _)) => {
            return Err(ScopedAttempt::Failed(CoreLookupFailure::Unauthorized));
        }
        Err(ureq::Error::Status(status, _)) => return Err(fail(&format!("status {status}"))),
        Err(ureq::Error::Transport(_)) => return Err(fail("transport")),
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("read failed"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(fail("response too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| fail("malformed response"))
}
