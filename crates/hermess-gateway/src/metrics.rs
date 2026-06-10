use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight routing metrics backed by lock-free atomics.
#[derive(Default)]
pub struct RouteMetrics {
    /// Total requests handled
    pub total_requests: AtomicU64,
    /// Requests routed via "auto" (not direct passthrough)
    pub auto_routed: AtomicU64,
    /// SHG (Short-Hard-Guard) triggers
    pub shg_triggers: AtomicU64,
    /// Classifier calls that succeeded
    pub classifier_ok: AtomicU64,
    /// Classifier calls that timed out
    pub classifier_timeout: AtomicU64,
    /// Classifier calls that fell back to default
    pub classifier_fallback: AtomicU64,
    /// Route decisions by strategy
    pub cost_first_decisions: AtomicU64,
    pub quality_first_decisions: AtomicU64,
    pub latency_first_decisions: AtomicU64,
    /// Decomposer triggers
    pub decomposer_triggers: AtomicU64,
    /// Upstream errors
    pub upstream_errors: AtomicU64,
}

impl RouteMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_total(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_auto(&self) {
        self.auto_routed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_shg(&self) {
        self.shg_triggers.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_classifier_ok(&self) {
        self.classifier_ok.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_classifier_timeout(&self) {
        self.classifier_timeout.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_classifier_fallback(&self) {
        self.classifier_fallback.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_strategy(&self, mode: &crate::models::RouteMode) {
        match mode {
            crate::models::RouteMode::CostFirst => {
                self.cost_first_decisions.fetch_add(1, Ordering::Relaxed)
            }
            crate::models::RouteMode::QualityFirst => {
                self.quality_first_decisions.fetch_add(1, Ordering::Relaxed)
            }
            crate::models::RouteMode::LatencyFirst => {
                self.latency_first_decisions.fetch_add(1, Ordering::Relaxed)
            }
        };
    }

    pub fn inc_decomposer(&self) {
        self.decomposer_triggers.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_upstream_error(&self) {
        self.upstream_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot current values for logging or /metrics endpoint.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            auto_routed: self.auto_routed.load(Ordering::Relaxed),
            shg_triggers: self.shg_triggers.load(Ordering::Relaxed),
            classifier_ok: self.classifier_ok.load(Ordering::Relaxed),
            classifier_timeout: self.classifier_timeout.load(Ordering::Relaxed),
            classifier_fallback: self.classifier_fallback.load(Ordering::Relaxed),
            cost_first_decisions: self.cost_first_decisions.load(Ordering::Relaxed),
            quality_first_decisions: self.quality_first_decisions.load(Ordering::Relaxed),
            latency_first_decisions: self.latency_first_decisions.load(Ordering::Relaxed),
            decomposer_triggers: self.decomposer_triggers.load(Ordering::Relaxed),
            upstream_errors: self.upstream_errors.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of all metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub auto_routed: u64,
    pub shg_triggers: u64,
    pub classifier_ok: u64,
    pub classifier_timeout: u64,
    pub classifier_fallback: u64,
    pub cost_first_decisions: u64,
    pub quality_first_decisions: u64,
    pub latency_first_decisions: u64,
    pub decomposer_triggers: u64,
    pub upstream_errors: u64,
}

impl MetricsSnapshot {
    /// Export as Prometheus text format.
    pub fn to_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# HELP hermess_gateway_requests_total Total requests handled by the gateway\n",
        );
        out.push_str("# TYPE hermess_gateway_requests_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_requests_total {}\n",
            self.total_requests
        ));
        out.push_str(
            "# HELP hermess_gateway_auto_routed_total Requests routed via auto strategy\n",
        );
        out.push_str("# TYPE hermess_gateway_auto_routed_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_auto_routed_total {}\n",
            self.auto_routed
        ));
        out.push_str("# HELP hermess_gateway_shg_triggers_total SHG detection triggers\n");
        out.push_str("# TYPE hermess_gateway_shg_triggers_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_shg_triggers_total {}\n",
            self.shg_triggers
        ));
        out.push_str("# HELP hermess_gateway_classifier_ok_total Classifier successes\n");
        out.push_str("# TYPE hermess_gateway_classifier_ok_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_classifier_ok_total {}\n",
            self.classifier_ok
        ));
        out.push_str("# HELP hermess_gateway_classifier_timeout_total Classifier timeouts\n");
        out.push_str("# TYPE hermess_gateway_classifier_timeout_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_classifier_timeout_total {}\n",
            self.classifier_timeout
        ));
        out.push_str("# HELP hermess_gateway_classifier_fallback_total Classifier fallbacks\n");
        out.push_str("# TYPE hermess_gateway_classifier_fallback_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_classifier_fallback_total {}\n",
            self.classifier_fallback
        ));
        out.push_str("# HELP hermess_gateway_upstream_errors_total Upstream provider errors\n");
        out.push_str("# TYPE hermess_gateway_upstream_errors_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_upstream_errors_total {}\n",
            self.upstream_errors
        ));
        out.push_str(
            "# HELP hermess_gateway_decomposer_triggers_total Prompt decomposer triggers\n",
        );
        out.push_str("# TYPE hermess_gateway_decomposer_triggers_total counter\n");
        out.push_str(&format!(
            "hermess_gateway_decomposer_triggers_total {}\n",
            self.decomposer_triggers
        ));
        // strategy counts
        out.push_str("# HELP hermess_gateway_route_decisions_total Route decisions by strategy (1=cost_first, 2=quality_first, 3=latency_first)\n");
        out.push_str("# TYPE hermess_gateway_route_decisions_total gauge\n");
        out.push_str(&format!(
            "hermess_gateway_route_decisions_total{{strategy=\"cost_first\"}} {}\n",
            self.cost_first_decisions
        ));
        out.push_str(&format!(
            "hermess_gateway_route_decisions_total{{strategy=\"quality_first\"}} {}\n",
            self.quality_first_decisions
        ));
        out.push_str(&format!(
            "hermess_gateway_route_decisions_total{{strategy=\"latency_first\"}} {}\n",
            self.latency_first_decisions
        ));
        out
    }
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "requests(total={}, auto={}, shg={}) classifier(ok={}, timeout={}, fallback={}) strategy(cost={}, quality={}, latency={}) decomposer={} errors={}",
            self.total_requests,
            self.auto_routed,
            self.shg_triggers,
            self.classifier_ok,
            self.classifier_timeout,
            self.classifier_fallback,
            self.cost_first_decisions,
            self.quality_first_decisions,
            self.latency_first_decisions,
            self.decomposer_triggers,
            self.upstream_errors,
        )
    }
}
