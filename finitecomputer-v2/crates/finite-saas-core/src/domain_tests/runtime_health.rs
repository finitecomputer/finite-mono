use super::*;

fn stored_health(ready: bool, reported_at: &str, interval_seconds: i64) -> StoredRuntimeHealth {
    StoredRuntimeHealth {
        reported_at: Some(reported_at.to_string()),
        observed_at: Some(reported_at.to_string()),
        ready: Some(ready),
        reason: None,
        report_interval_seconds: Some(interval_seconds),
        reporting_npub: None,
    }
}

#[test]
fn runtime_health_projection_is_ready_only_for_a_fresh_ready_report() {
    let now = "2026-08-24T12:00:00Z";
    let fresh = stored_health(true, "2026-08-24T11:59:00Z", 60);
    let projected = project_runtime_health(RuntimeSummaryStatus::Online, &fresh, now).unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::Ready);

    // A not-ready report stays not_ready inside the freshness window, and
    // its reason rides along.
    let mut not_ready = stored_health(false, "2026-08-24T11:59:00Z", 60);
    not_ready.reason = Some("unreachable".to_string());
    let projected = project_runtime_health(RuntimeSummaryStatus::Online, &not_ready, now).unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::NotReady);
    assert_eq!(projected.reason.as_deref(), Some("unreachable"));
}

#[test]
fn runtime_health_projection_names_stale_and_missing_reports() {
    let now = "2026-08-24T12:00:00Z";
    // 181s old at a 60s cadence is past the 3x staleness deadline: the
    // "died at 3am, shows ready forever" gap closes as `stale`.
    let stale = stored_health(true, "2026-08-24T11:56:59Z", 60);
    let projected = project_runtime_health(RuntimeSummaryStatus::Online, &stale, now).unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::Stale);
    assert_eq!(projected.report_interval_seconds, Some(60));

    // Just inside the deadline still projects the report.
    let edge = stored_health(true, "2026-08-24T11:57:00Z", 60);
    let projected = project_runtime_health(RuntimeSummaryStatus::Online, &edge, now).unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::Ready);

    // A slower reporter gets its own deadline: 10m cadence, 20m old is
    // fresh for it but would be stale at the default cadence.
    let slow = stored_health(true, "2026-08-24T11:40:00Z", 600);
    let projected = project_runtime_health(RuntimeSummaryStatus::Online, &slow, now).unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::Ready);

    // No report at all is unknown.
    let projected = project_runtime_health(
        RuntimeSummaryStatus::Online,
        &StoredRuntimeHealth::default(),
        now,
    )
    .unwrap();
    assert_eq!(projected.status, RuntimeHealthStatus::Unknown);
    assert_eq!(projected.report_interval_seconds, None);
}

#[test]
fn runtime_health_projection_is_silent_for_offline_runtimes() {
    let now = "2026-08-24T12:00:00Z";
    let fresh = stored_health(true, "2026-08-24T11:59:30Z", 60);
    let projected = project_runtime_health(RuntimeSummaryStatus::Offline, &fresh, now).unwrap();
    assert_eq!(
        projected.status,
        RuntimeHealthStatus::Unknown,
        "a stopped runtime carries no standing readiness claim"
    );
    // A failed control (`stale` latch) or an unconfirmed registration
    // does not silence a fresh report: freshness is the truth.
    for status in [RuntimeSummaryStatus::Stale, RuntimeSummaryStatus::Unknown] {
        let projected = project_runtime_health(status, &fresh, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Ready);
    }
}

#[test]
fn derived_summary_status_follows_report_freshness() {
    let now = "2026-08-24T12:00:00Z";
    let project = |lifecycle: RuntimeSummaryStatus, health: &StoredRuntimeHealth| {
        let projection = project_runtime_health(lifecycle, health, now).unwrap();
        derive_runtime_summary_status(lifecycle, &projection)
    };
    let fresh_ready = stored_health(true, "2026-08-24T11:59:30Z", 60);
    let fresh_not_ready = stored_health(false, "2026-08-24T11:59:30Z", 60);
    let lapsed = stored_health(true, "2026-08-24T11:00:00Z", 60);
    let never = StoredRuntimeHealth::default();

    // The table: (lifecycle latch, stored report) => user-facing status.
    let table = [
        (
            RuntimeSummaryStatus::Online,
            &fresh_ready,
            RuntimeSummaryStatus::Online,
        ),
        (
            RuntimeSummaryStatus::Online,
            &fresh_not_ready,
            RuntimeSummaryStatus::Offline,
        ),
        (
            RuntimeSummaryStatus::Online,
            &lapsed,
            RuntimeSummaryStatus::Stale,
        ),
        (
            RuntimeSummaryStatus::Online,
            &never,
            RuntimeSummaryStatus::Unknown,
        ),
        // A failed control does not hide a live runtime, and does not
        // invent one either.
        (
            RuntimeSummaryStatus::Stale,
            &fresh_ready,
            RuntimeSummaryStatus::Online,
        ),
        (
            RuntimeSummaryStatus::Stale,
            &lapsed,
            RuntimeSummaryStatus::Stale,
        ),
        (
            RuntimeSummaryStatus::Stale,
            &never,
            RuntimeSummaryStatus::Unknown,
        ),
        (
            RuntimeSummaryStatus::Unknown,
            &fresh_ready,
            RuntimeSummaryStatus::Online,
        ),
        // Deliberately stopped stays offline whatever the last report said.
        (
            RuntimeSummaryStatus::Offline,
            &fresh_ready,
            RuntimeSummaryStatus::Offline,
        ),
        (
            RuntimeSummaryStatus::Offline,
            &never,
            RuntimeSummaryStatus::Offline,
        ),
    ];
    for (lifecycle, health, expected) in table {
        assert_eq!(
            project(lifecycle, health),
            expected,
            "{lifecycle:?} / {health:?}"
        );
    }
}
