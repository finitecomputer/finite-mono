use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const DURATION_BUCKETS_MS: [u64; 12] = [
    100, 250, 500, 1_000, 2_500, 5_000, 10_000, 60_000, 120_000, 300_000, 600_000, 1_800_000,
];

#[derive(Clone, Default)]
pub(crate) struct LimiterMetrics {
    requests: Arc<RequestCounters>,
}

#[derive(Default)]
struct RequestCounters {
    started: AtomicU64,
    admissions: [AtomicU64; 2],
    refusals: [AtomicU64; 5],
    terminal: [AtomicU64; 5],
    active: AtomicU64,
    actual_settlements: AtomicU64,
    estimated_settlements: AtomicU64,
    settlement_failures: AtomicU64,
    diagnostics_failures: AtomicU64,
    routes: Mutex<BTreeMap<(String, String), RouteStats>>,
}

#[derive(Default)]
struct RouteStats {
    terminal: [u64; 5],
    input_tokens: u64,
    output_tokens: u64,
    first_output: RouteHistogram,
    first_answer: RouteHistogram,
    duration: RouteHistogram,
}

#[derive(Default)]
struct RouteHistogram {
    buckets: [u64; DURATION_BUCKETS_MS.len() + 1],
    count: u64,
    sum_ms: u64,
}

impl RouteHistogram {
    fn observe(&mut self, elapsed: u64) {
        observe_histogram(
            &mut self.buckets,
            &mut self.count,
            &mut self.sum_ms,
            elapsed,
        );
    }
    fn render(&self, output: &mut String, name: &str, labels: &str) {
        histogram(output, name, &self.buckets, self.count, self.sum_ms, labels);
    }
}

pub(crate) struct RequestTimer {
    metrics: LimiterMetrics,
    started: Instant,
    model: String,
    endpoint: String,
    finished: AtomicU64,
    first_output: AtomicU64,
    first_answer: AtomicU64,
    duration: AtomicU64,
}

impl LimiterMetrics {
    pub(crate) fn request_observed(&self) {
        self.requests.started.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn request_started_for_route(&self, model: &str, endpoint: &str) -> RequestTimer {
        self.requests.started.fetch_add(1, Ordering::Relaxed);
        self.requests.active.fetch_add(1, Ordering::Relaxed);
        RequestTimer {
            metrics: self.clone(),
            started: Instant::now(),
            model: model.to_string(),
            endpoint: endpoint.to_string(),
            finished: AtomicU64::new(0),
            first_output: AtomicU64::new(0),
            first_answer: AtomicU64::new(0),
            duration: AtomicU64::new(0),
        }
    }

    pub(crate) fn admission(&self, admitted: bool) {
        self.requests.admissions[usize::from(!admitted)].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn refusal(&self, reason: &str) {
        let index = match reason {
            "invalid_api_key" => 0,
            "unsupported_model" => 1,
            "usage_api_unavailable" => 2,
            "burst_window_limit_exceeded" | "weekly_limit_exceeded" => 3,
            _ => 4,
        };
        self.requests.refusals[index].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn terminal(&self, outcome: TerminalOutcome) {
        self.requests.terminal[outcome as usize].fetch_add(1, Ordering::Relaxed);
    }

    fn terminal_for_route(&self, model: &str, endpoint: &str, outcome: TerminalOutcome) {
        self.terminal(outcome);
        let mut routes = self.requests.routes.lock().expect("metrics mutex poisoned");
        routes
            .entry((model.to_string(), endpoint.to_string()))
            .or_default()
            .terminal[outcome as usize] += 1;
    }

    pub(crate) fn settlement(&self, actual: bool, success: bool) {
        if actual {
            self.requests
                .actual_settlements
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.requests
                .estimated_settlements
                .fetch_add(1, Ordering::Relaxed);
        }
        if !success {
            self.requests
                .settlement_failures
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn diagnostics_failed(&self) {
        self.requests
            .diagnostics_failures
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn render(&self, admission_mode: &str) -> String {
        let c = &self.requests;
        let mut output = String::from("# Finite Private limiter metrics\n");
        metric(&mut output, "finite_private_limiter_live", "", 1);
        metric(
            &mut output,
            "finite_private_limiter_admission_mode",
            &format!("mode=\"{admission_mode}\""),
            1,
        );
        metric(
            &mut output,
            "finite_private_limiter_active_requests",
            "",
            c.active.load(Ordering::Relaxed),
        );
        metric(
            &mut output,
            "finite_private_limiter_requests_started_total",
            "",
            c.started.load(Ordering::Relaxed),
        );
        for (outcome, index) in [("admitted", 0), ("refused", 1)] {
            metric(
                &mut output,
                "finite_private_limiter_admissions_total",
                &format!("mode=\"{admission_mode}\",outcome=\"{outcome}\""),
                c.admissions[index].load(Ordering::Relaxed),
            );
        }
        for (reason, index) in [
            ("invalid_api_key", 0),
            ("unsupported_model", 1),
            ("usage_api_unavailable", 2),
            ("limit_exceeded", 3),
            ("other", 4),
        ] {
            metric(
                &mut output,
                "finite_private_limiter_refusals_total",
                &format!("mode=\"{admission_mode}\",reason=\"{reason}\""),
                c.refusals[index].load(Ordering::Relaxed),
            );
        }
        for outcome in TerminalOutcome::ALL {
            metric(
                &mut output,
                "finite_private_limiter_requests_total",
                &format!("outcome=\"{}\"", outcome.as_str()),
                c.terminal[outcome as usize].load(Ordering::Relaxed),
            );
        }
        let routes = self.requests.routes.lock().expect("metrics mutex poisoned");
        for ((model, endpoint), route) in routes.iter() {
            let model = escape_label(model);
            let endpoint = escape_label(endpoint);
            let labels = format!("model=\"{model}\",endpoint=\"{endpoint}\"");
            metric(
                &mut output,
                "finite_private_limiter_route_input_tokens_total",
                &labels,
                route.input_tokens,
            );
            metric(
                &mut output,
                "finite_private_limiter_route_output_tokens_total",
                &labels,
                route.output_tokens,
            );
            route.first_output.render(
                &mut output,
                "finite_private_limiter_route_first_output_seconds",
                &labels,
            );
            route.first_answer.render(
                &mut output,
                "finite_private_limiter_route_first_answer_seconds",
                &labels,
            );
            route.duration.render(
                &mut output,
                "finite_private_limiter_route_request_duration_seconds",
                &labels,
            );
            for outcome in TerminalOutcome::ALL {
                metric(
                    &mut output,
                    "finite_private_limiter_route_requests_total",
                    &format!(
                        "model=\"{model}\",endpoint=\"{endpoint}\",outcome=\"{}\"",
                        outcome.as_str()
                    ),
                    route.terminal[outcome as usize],
                );
            }
        }
        metric(
            &mut output,
            "finite_private_limiter_settlements_total",
            "kind=\"actual\"",
            c.actual_settlements.load(Ordering::Relaxed),
        );
        metric(
            &mut output,
            "finite_private_limiter_settlements_total",
            "kind=\"estimate\"",
            c.estimated_settlements.load(Ordering::Relaxed),
        );
        metric(
            &mut output,
            "finite_private_limiter_settlement_failures_total",
            "",
            c.settlement_failures.load(Ordering::Relaxed),
        );
        metric(
            &mut output,
            "finite_private_limiter_diagnostic_persistence_failures_total",
            "",
            c.diagnostics_failures.load(Ordering::Relaxed),
        );
        output
    }
}

#[derive(Clone, Copy)]
pub(crate) enum TerminalOutcome {
    Success = 0,
    UpstreamError = 1,
    ClientCancellation = 2,
    AdmissionError = 3,
    UpstreamTimeout = 4,
}

impl TerminalOutcome {
    const ALL: [Self; 5] = [
        Self::Success,
        Self::UpstreamError,
        Self::ClientCancellation,
        Self::AdmissionError,
        Self::UpstreamTimeout,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::UpstreamError => "upstream_error",
            Self::ClientCancellation => "client_cancellation",
            Self::AdmissionError => "admission_error",
            Self::UpstreamTimeout => "upstream_timeout",
        }
    }
}

impl RequestTimer {
    pub(crate) fn tokens(&self, input: Option<i64>, output: Option<i64>) {
        let mut routes = self
            .metrics
            .requests
            .routes
            .lock()
            .expect("metrics mutex poisoned");
        let route = routes
            .entry((self.model.clone(), self.endpoint.clone()))
            .or_default();
        route.input_tokens += input.filter(|v| *v >= 0).unwrap_or(0) as u64;
        route.output_tokens += output.filter(|v| *v >= 0).unwrap_or(0) as u64;
    }

    pub(crate) fn timing(&self) -> (Option<i64>, Option<i64>, Option<i64>) {
        let optional = |value: &AtomicU64| {
            let value = value.load(Ordering::Relaxed);
            (value > 0).then_some(value as i64)
        };
        (
            optional(&self.first_output),
            optional(&self.first_answer),
            optional(&self.duration),
        )
    }
    pub(crate) fn observe_first_output(&self) {
        let elapsed = self.started.elapsed().as_millis() as u64;
        if self
            .first_output
            .compare_exchange(0, elapsed.max(1), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.metrics
                .requests
                .routes
                .lock()
                .expect("metrics mutex poisoned")
                .entry((self.model.clone(), self.endpoint.clone()))
                .or_default()
                .first_output
                .observe(elapsed);
        }
    }

    pub(crate) fn observe_first_answer(&self) {
        let elapsed = self.started.elapsed().as_millis() as u64;
        if self
            .first_answer
            .compare_exchange(0, elapsed.max(1), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.metrics
                .requests
                .routes
                .lock()
                .expect("metrics mutex poisoned")
                .entry((self.model.clone(), self.endpoint.clone()))
                .or_default()
                .first_answer
                .observe(elapsed);
        }
    }

    pub(crate) fn finish(&self, outcome: TerminalOutcome) {
        if self
            .finished
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.metrics.requests.active.fetch_sub(1, Ordering::Relaxed);
            self.metrics
                .terminal_for_route(&self.model, &self.endpoint, outcome);
            let elapsed = self.started.elapsed().as_millis() as u64;
            self.duration.store(elapsed.max(1), Ordering::Relaxed);
            self.metrics
                .requests
                .routes
                .lock()
                .expect("metrics mutex poisoned")
                .entry((self.model.clone(), self.endpoint.clone()))
                .or_default()
                .duration
                .observe(elapsed);
        }
    }
}

impl Drop for RequestTimer {
    fn drop(&mut self) {
        self.finish(TerminalOutcome::ClientCancellation);
    }
}

fn observe_histogram(buckets: &mut [u64], count: &mut u64, sum_ms: &mut u64, value_ms: u64) {
    *count += 1;
    *sum_ms += value_ms;
    let index = DURATION_BUCKETS_MS
        .iter()
        .position(|bound| value_ms <= *bound)
        .unwrap_or(DURATION_BUCKETS_MS.len());
    buckets[index] += 1;
}

fn histogram(
    output: &mut String,
    name: &str,
    buckets: &[u64],
    count: u64,
    sum_ms: u64,
    labels: &str,
) {
    let prefix = if labels.is_empty() {
        String::new()
    } else {
        format!("{labels},")
    };
    let mut cumulative = 0;
    for (index, bound) in DURATION_BUCKETS_MS.iter().enumerate() {
        cumulative += buckets[index];
        metric(
            output,
            &format!("{name}_bucket"),
            &format!("{prefix}le=\"{}\"", *bound as f64 / 1000.0),
            cumulative,
        );
    }
    cumulative += buckets[DURATION_BUCKETS_MS.len()];
    metric(
        output,
        &format!("{name}_bucket"),
        &format!("{prefix}le=\"+Inf\""),
        cumulative,
    );
    metric(output, &format!("{name}_count"), labels, count);
    metric(
        output,
        &format!("{name}_sum"),
        labels,
        sum_ms as f64 / 1000.0,
    );
}

fn metric(output: &mut String, name: &str, labels: &str, value: impl std::fmt::Display) {
    if labels.is_empty() {
        output.push_str(&format!("{name} {value}\n"));
    } else {
        output.push_str(&format!("{name}{{{labels}}} {value}\n"));
    }
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_authoritative_route_metrics_once_per_request() {
        let metrics = LimiterMetrics::default();
        metrics.admission(true);
        metrics.admission(false);
        metrics.settlement(true, true);
        let timer = metrics.request_started_for_route("glm-5-3-flash", "/v1/chat/completions");
        timer.tokens(Some(4), Some(7));
        timer.observe_first_output();
        timer.observe_first_output();
        timer.observe_first_answer();
        timer.observe_first_answer();
        timer.finish(TerminalOutcome::Success);
        timer.finish(TerminalOutcome::UpstreamError);
        let output = metrics.render("usage-api");
        assert!(output.contains(
            "finite_private_limiter_admissions_total{mode=\"usage-api\",outcome=\"admitted\"} 1"
        ));
        assert!(output.contains(
            "finite_private_limiter_admissions_total{mode=\"usage-api\",outcome=\"refused\"} 1"
        ));
        assert!(output.contains("finite_private_limiter_admission_mode{mode=\"usage-api\"} 1"));
        assert!(output.contains("finite_private_limiter_requests_started_total 1"));
        let route = "model=\"glm-5-3-flash\",endpoint=\"/v1/chat/completions\"";
        assert!(output.contains(&format!(
            "finite_private_limiter_route_input_tokens_total{{{route}}} 4"
        )));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_output_tokens_total{{{route}}} 7"
        )));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_requests_total{{{route},outcome=\"success\"}} 1"
        )));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_requests_total{{{route},outcome=\"upstream_error\"}} 0"
        )));
        for metric in [
            "route_request_duration_seconds",
            "route_first_output_seconds",
            "route_first_answer_seconds",
        ] {
            assert!(output.contains(&format!(
                "finite_private_limiter_{metric}_count{{{route}}} 1"
            )));
            assert!(output.contains(&format!(
                "finite_private_limiter_{metric}_bucket{{{route},le=\"+Inf\"}} 1"
            )));
        }
        assert!(!output.contains("finite_private_limiter_input_tokens_total"));
        assert!(!output.contains("finite_private_limiter_output_tokens_total"));
        assert!(!output.contains("finite_private_limiter_route_admissions_total"));
    }

    #[test]
    fn dropped_request_finishes_once_as_cancellation() {
        let metrics = LimiterMetrics::default();
        let timer = metrics.request_started_for_route("glm-5-3-flash", "/v1/chat/completions");
        timer.observe_first_output();
        drop(timer);
        let output = metrics.render("usage-api");
        let route = "model=\"glm-5-3-flash\",endpoint=\"/v1/chat/completions\"";
        assert!(output.contains("finite_private_limiter_active_requests 0"));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_requests_total{{{route},outcome=\"client_cancellation\"}} 1"
        )));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_request_duration_seconds_count{{{route}}} 1"
        )));
        assert!(output.contains(&format!(
            "finite_private_limiter_route_first_output_seconds_count{{{route}}} 1"
        )));
    }

    #[test]
    fn histogram_keeps_cumulative_buckets_and_sum() {
        let mut histogram = RouteHistogram::default();
        histogram.observe(50);
        histogram.observe(300);
        let mut output = String::new();
        histogram.render(&mut output, "latency_seconds", "model=\"test\"");
        assert!(output.contains("latency_seconds_bucket{model=\"test\",le=\"0.1\"} 1"));
        assert!(output.contains("latency_seconds_bucket{model=\"test\",le=\"0.25\"} 1"));
        assert!(output.contains("latency_seconds_bucket{model=\"test\",le=\"0.5\"} 2"));
        assert!(output.contains("latency_seconds_bucket{model=\"test\",le=\"+Inf\"} 2"));
        assert!(output.contains("latency_seconds_count{model=\"test\"} 2"));
        assert!(output.contains("latency_seconds_sum{model=\"test\"} 0.35"));
    }
}
