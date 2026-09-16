//! Authenticated aggregate metrics. Registry reads run on the existing read-only
//! pool, outside the publishing mutex and Tokio's async executor.
use std::fmt::Write;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use finitesites_store::SiteMetrics;

use crate::server::{AppState, now_unix};

const TOKEN_ENV: &str = "FINITE_SITES_METRICS_TOKEN";

pub(crate) fn token_from_env() -> Result<Option<String>, String> {
    let token = match std::env::var(TOKEN_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err(format!("{TOKEN_ENV} must contain valid UTF-8")),
    };
    validate_token(token.as_deref())?;
    Ok(token)
}

pub(crate) fn validate_token(token: Option<&str>) -> Result<(), String> {
    if token.is_some_and(|value| !finitesites_proto::hex::is_hex32(value)) {
        return Err(format!(
            "{TOKEN_ENV} must be exactly 64 lowercase hex characters"
        ));
    }
    Ok(())
}

fn authorize(headers: &HeaderMap, token: Option<&str>) -> Result<(), StatusCode> {
    let expected = token.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if crate::api::constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub(crate) async fn get_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let mut response = match authorize(&headers, state.metrics_token.as_deref()) {
        Err(status) => status.into_response(),
        Ok(()) => match state
            .serving_engines
            .run(|engine| engine.site_metrics(now_unix()))
            .await
        {
            Ok(Ok(metrics)) => match render(&metrics) {
                Ok(body) => (
                    [(
                        header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    body,
                )
                    .into_response(),
                Err(()) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            },
            // Never emit zero counts or cached data when collection fails.
            _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        },
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

fn render(metrics: &SiteMetrics) -> Result<String, ()> {
    let mut out = format!(
        "# HELP finite_sites_existing Current Sites excluding soft-deleted Sites, including unpublished and disabled Sites.\n\
         # TYPE finite_sites_existing gauge\n\
         finite_sites_existing {}\n\
         # HELP finite_sites_published Current Sites with published status, regardless of visibility.\n\
         # TYPE finite_sites_published gauge\n\
         finite_sites_published {}\n\
         # HELP finite_sites_metrics_collected_at_seconds Registry snapshot collection time in Unix seconds.\n\
         # TYPE finite_sites_metrics_collected_at_seconds gauge\n\
         finite_sites_metrics_collected_at_seconds {}\n\
         # HELP finite_sites_created_by_day Site allocations by stored UTC creation date, including soft-deleted Sites; today is partial.\n\
         # TYPE finite_sites_created_by_day gauge\n",
        metrics.existing, metrics.published, metrics.collected_at
    );
    // The store bounds this vector to SITE_METRICS_HISTORY_DAYS, zero-filled.
    for day in &metrics.created_by_day {
        let timestamp = i64::try_from(day.start_unix).map_err(|_| ())?;
        let date = time::OffsetDateTime::from_unix_timestamp(timestamp)
            .map_err(|_| ())?
            .date();
        writeln!(
            out,
            "finite_sites_created_by_day{{date=\"{date}\"}} {}",
            day.count
        )
        .map_err(|_| ())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitesites_store::SiteCreationDay;

    #[test]
    fn auth_is_disabled_without_token_and_rejects_missing_wrong_or_malformed_tokens() {
        let token = "a".repeat(64);
        let mut headers = HeaderMap::new();
        assert_eq!(
            authorize(&headers, None),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(
            authorize(&headers, Some(&token)),
            Err(StatusCode::UNAUTHORIZED)
        );
        for value in [
            format!("Bearer {}", "b".repeat(64)),
            token.clone(),
            "Bearer ".into(),
            format!("Bearer {}", "a".repeat(65)),
        ] {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            assert_eq!(
                authorize(&headers, Some(&token)),
                Err(StatusCode::UNAUTHORIZED)
            );
        }
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(authorize(&headers, Some(&token)), Ok(()));
        assert!(validate_token(None).is_ok());
        assert!(validate_token(Some(&token)).is_ok());
        for invalid in ["", "secret", &"A".repeat(64)] {
            assert!(validate_token(Some(invalid)).is_err());
        }
    }

    #[test]
    fn exposition_uses_utc_dates_and_aggregate_gauges() {
        let text = render(&SiteMetrics {
            existing: 4,
            published: 2,
            collected_at: 172800,
            created_by_day: vec![
                SiteCreationDay {
                    start_unix: 86400,
                    count: 0,
                },
                SiteCreationDay {
                    start_unix: 172800,
                    count: 3,
                },
            ],
        })
        .unwrap();
        assert!(text.contains("finite_sites_existing 4\n"));
        assert!(text.contains("finite_sites_published 2\n"));
        assert!(text.contains("finite_sites_created_by_day{date=\"1970-01-02\"} 0\n"));
        assert!(text.contains("finite_sites_created_by_day{date=\"1970-01-03\"} 3\n"));
        assert_eq!(text.lines().filter(|l| l.starts_with("# TYPE")).count(), 4);
    }
}
