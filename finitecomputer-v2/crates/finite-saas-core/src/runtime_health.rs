//! Standing runtime-health reports and read-time freshness projections.

use crate::{CoreResult, RuntimeSummaryStatus, parse_time, wire_enum};
use serde::Deserialize;
use serde::Serialize;
use time::Duration;

/// Standing runtime readiness, ferried by the Runner (2026-08 audit synthesis,
/// H1 slice 3): the Runner polls each live Runtime's `/contact` on a bounded
/// cadence and posts one report per Runtime to Core. Core stores only the
/// latest report on the runtime row (migration 0022) and projects readiness
/// at read time — there is no history table and no background sweeper.
pub const RUNTIME_HEALTH_REPORT_DEFAULT_INTERVAL_SECONDS: i64 = 60;

pub const RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS: i64 = 5;

pub const RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS: i64 = 3600;

/// A report older than this many poll intervals is stale: the projection then
/// reads `unknown` ("the runner stopped reporting"), never a frozen `ready`.
pub const RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER: i64 = 3;

pub const MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS: usize = 512;

wire_enum! {
/// Read-time projection of one runtime's standing readiness. `ready` requires
/// a fresh report saying ready; `not_ready` is a fresh report saying not
/// ready (with the reported reason); `unknown` is no report, a stale report,
/// or a runtime Core does not consider online.
    RuntimeHealthStatus {
    Ready => "ready",
    NotReady => "not_ready",
    Stale => "stale",
    Unknown => "unknown",
    }
    parse: parse_runtime_health_status
}

/// The latest stored runner-ferried health report, as read back from the
/// runtime row. Every field is `None` until the runner's standing poller
/// first reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredRuntimeHealth {
    pub reported_at: Option<String>,
    pub observed_at: Option<String>,
    pub ready: Option<bool>,
    pub reason: Option<String>,
    pub report_interval_seconds: Option<i64>,
    pub reporting_npub: Option<String>,
}

/// One runtime's standing readiness as projected at read time. The raw report
/// fields always ride along as evidence; `status` is the only derived fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeHealthProjection {
    pub status: RuntimeHealthStatus,
    pub reason: Option<String>,
    pub reported_at: Option<String>,
    pub observed_at: Option<String>,
    pub agent_npub: Option<String>,
    /// The reporter's poll cadence as stored (clamped to the accepted range);
    /// `None` until a report has been recorded.
    #[serde(default)]
    pub report_interval_seconds: Option<i64>,
}

impl RuntimeHealthProjection {
    /// The projection of a runtime that has never been reported on.
    pub fn unreported() -> Self {
        Self {
            status: RuntimeHealthStatus::Unknown,
            reason: None,
            reported_at: None,
            observed_at: None,
            agent_npub: None,
            report_interval_seconds: None,
        }
    }
}

/// Project standing readiness from the latest stored report. Reports do not
/// speak for a runtime that is intentionally `offline`. Every completion that
/// brings compute up clears the stored report, so a report never speaks for
/// a previous incarnation. Freshness is measured from `reported_at` (Core's
/// receive clock), never the runner's `observed_at`, so runner clock skew
/// cannot extend freshness. A report older than
/// `RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER` intervals is the named `stale`
/// state; no report at all is `unknown`.
pub fn project_runtime_health(
    runtime_status: RuntimeSummaryStatus,
    health: &StoredRuntimeHealth,
    now: &str,
) -> CoreResult<RuntimeHealthProjection> {
    let interval_seconds = health.report_interval_seconds.map(|interval| {
        interval.clamp(
            RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS,
            RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS,
        )
    });
    let reports_speak = runtime_status != RuntimeSummaryStatus::Offline;
    let status = if !reports_speak {
        RuntimeHealthStatus::Unknown
    } else if let (Some(ready), Some(reported_at)) = (health.ready, health.reported_at.as_deref()) {
        let deadline_seconds = interval_seconds
            .unwrap_or(RUNTIME_HEALTH_REPORT_DEFAULT_INTERVAL_SECONDS)
            * RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER;
        let age = parse_time(now)? - parse_time(reported_at)?;
        if age > Duration::seconds(deadline_seconds) {
            RuntimeHealthStatus::Stale
        } else if ready {
            RuntimeHealthStatus::Ready
        } else {
            RuntimeHealthStatus::NotReady
        }
    } else {
        RuntimeHealthStatus::Unknown
    };
    Ok(RuntimeHealthProjection {
        status,
        reason: health.reason.clone(),
        reported_at: health.reported_at.clone(),
        observed_at: health.observed_at.clone(),
        agent_npub: health.reporting_npub.clone(),
        report_interval_seconds: interval_seconds,
    })
}

/// The one user-facing summary rule: what a runtime's status *is* derives
/// from report freshness, never from the last lifecycle outcome alone.
///
/// - an intentionally stopped runtime (`offline` latch) stays `offline`;
/// - otherwise a fresh `ready` report is `online`, a fresh not-ready report
///   is `offline` (nobody answers, or the guest says it is not ready), a
///   report past the freshness deadline is `stale`, and a runtime that has
///   not been reported on since its compute last came up is `unknown`.
///
/// The latched lifecycle fact stays visible to operators under its own
/// field; this value is what the summary status wire fields carry.
pub fn derive_runtime_summary_status(
    lifecycle_status: RuntimeSummaryStatus,
    health: &RuntimeHealthProjection,
) -> RuntimeSummaryStatus {
    match lifecycle_status {
        RuntimeSummaryStatus::Offline => RuntimeSummaryStatus::Offline,
        RuntimeSummaryStatus::Online
        | RuntimeSummaryStatus::Stale
        | RuntimeSummaryStatus::Unknown => match health.status {
            RuntimeHealthStatus::Ready => RuntimeSummaryStatus::Online,
            RuntimeHealthStatus::NotReady => RuntimeSummaryStatus::Offline,
            RuntimeHealthStatus::Stale => RuntimeSummaryStatus::Stale,
            RuntimeHealthStatus::Unknown => RuntimeSummaryStatus::Unknown,
        },
    }
}

/// One runtime the runner's standing-health poller should poll, as listed by
/// Core for the runner credential's host each cycle: every live runtime on
/// that host whose lifecycle latch is not `offline`. Core is the only source
/// of the target set; the runner keeps no registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthTarget {
    pub agent_runtime_id: String,
    pub source_machine_id: String,
    /// The contact endpoint Core holds for the runtime; `None` for rows that
    /// never recorded one (the runner cannot poll those).
    pub contact_endpoint: Option<String>,
    /// The Agent Principal npub Core holds for the runtime, if any — seeded
    /// from the launch-verified principal when its compute last came up, or
    /// from its first report. The runner attributes answers to it only when
    /// they present this principal, and Core rejects reports that carry
    /// another, so a reallocated port cannot wear this runtime's name.
    #[serde(default)]
    pub agent_npub: Option<String>,
    pub lifecycle_status: RuntimeSummaryStatus,
    /// The cadence the latest stored report declared, if any.
    #[serde(default)]
    pub report_interval_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthTargetList {
    pub source_host_id: String,
    pub targets: Vec<RuntimeHealthTarget>,
}

/// The runner's wire request for `POST /api/core/v1/runtime-health-reports`.
/// The source host comes from the runner credential, never from the body, so
/// a runner can only report for runtimes on its own host; a body naming a
/// runtime outside the credential's scope is rejected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthReportRequest {
    pub agent_runtime_id: String,
    pub ready: bool,
    /// Bounded not-ready reason: the guest's `/contact` error or the runner's
    /// `unreachable` marker for a transport failure.
    #[serde(default)]
    pub reason: Option<String>,
    /// When the runner read `/contact` (runner clock; evidence only).
    pub observed_at: String,
    /// The Agent Principal npub the runner pinned and observed; the
    /// anti-port-squat cross-check evidence.
    #[serde(default)]
    pub agent_npub: Option<String>,
    /// The runner's poll cadence; the read-time projection declares staleness
    /// after `RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER` intervals.
    #[serde(default)]
    pub report_interval_seconds: Option<i64>,
    #[serde(default)]
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordRuntimeHealthReportInput {
    pub source_host_id: String,
    pub agent_runtime_id: String,
    pub ready: bool,
    pub reason: Option<String>,
    pub observed_at: String,
    pub agent_npub: Option<String>,
    pub report_interval_seconds: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthReportAck {
    pub agent_runtime_id: String,
    pub recorded_at: String,
}
