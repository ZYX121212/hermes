// 组合数据源：按优先级依次尝试多个 provider，自动容灾切换。
// 内置熔断器：连续失败的 provider 会被临时跳过，定期探活恢复。

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use crate::{FinancialDataProvider, FinancialQuery, FinancialResult, QueryCapability};

/// Per-provider circuit breaker state.
struct CircuitState {
    /// Consecutive failures since last success.
    consecutive_failures: AtomicU32,
    /// Timestamp (epoch ms) of last failure.
    last_failure_ms: AtomicU64,
    /// Timestamp (epoch ms) of last circuit open (provider skipped).
    opened_at_ms: AtomicU64,
}

impl CircuitState {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            last_failure_ms: AtomicU64::new(0),
            opened_at_ms: AtomicU64::new(0),
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        let now = epoch_ms();
        self.last_failure_ms.store(now, Ordering::SeqCst);
    }

    /// Whether the circuit is currently open (provider should be skipped).
    /// Opens after `threshold` consecutive failures; stays open for `cooldown`.
    fn is_open(&self, threshold: u32, cooldown: Duration) -> bool {
        let failures = self.consecutive_failures.load(Ordering::SeqCst);
        if failures < threshold {
            return false;
        }
        let last = self.last_failure_ms.load(Ordering::SeqCst);
        let opened = self.opened_at_ms.load(Ordering::SeqCst);
        let now = epoch_ms();
        // If we just crossed the threshold, mark open time
        if opened == 0 || now - opened > cooldown.as_millis() as u64 {
            self.opened_at_ms.store(now, Ordering::SeqCst);
        }
        now - last < cooldown.as_millis() as u64
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 按优先级排列的多个 provider，带熔断保护。
/// - 连续失败 ≥ `failure_threshold` 的 provider 将被跳过
/// - 冷却 `cooldown` 后自动恢复探活
pub struct CompositeProvider {
    providers: Vec<Box<dyn FinancialDataProvider>>,
    circuits: Vec<CircuitState>,
    name: String,
    failure_threshold: u32,
    cooldown: Duration,
}

impl CompositeProvider {
    pub fn new(providers: Vec<Box<dyn FinancialDataProvider>>) -> Self {
        assert!(
            !providers.is_empty(),
            "CompositeProvider requires at least one provider"
        );
        let count = providers.len();
        let names: Vec<&str> = providers.iter().map(|p| p.provider_name()).collect();
        let name = format!("composite({})", names.join(","));
        let circuits = (0..count).map(|_| CircuitState::new()).collect();
        Self {
            providers,
            circuits,
            name,
            failure_threshold: 3,
            cooldown: Duration::from_secs(30),
        }
    }

    /// Override circuit breaker settings.
    pub fn with_circuit_breaker(mut self, failure_threshold: u32, cooldown_secs: u64) -> Self {
        self.failure_threshold = failure_threshold;
        self.cooldown = Duration::from_secs(cooldown_secs);
        self
    }
}

#[async_trait]
impl FinancialDataProvider for CompositeProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Vec<QueryCapability> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut caps = Vec::new();
        for p in &self.providers {
            for c in p.capabilities() {
                if seen.insert(c.name.clone()) {
                    caps.push(c);
                }
            }
        }
        caps
    }

    async fn query(&self, q: FinancialQuery) -> anyhow::Result<FinancialResult> {
        let mut last_err: Option<anyhow::Error> = None;
        let mut last_business_failure: Option<FinancialResult> = None;
        let mut skipped = 0usize;

        for (i, p) in self.providers.iter().enumerate() {
            let name = p.provider_name();

            // Circuit breaker: skip providers that are failing repeatedly
            if self.circuits[i].is_open(self.failure_threshold, self.cooldown) {
                skipped += 1;
                tracing::debug!(provider = name, "Circuit open — skipping provider");
                continue;
            }

            match p.query(q.clone()).await {
                Ok(r) if r.success => {
                    self.circuits[i].record_success();
                    if i > 0 || skipped > 0 {
                        tracing::info!(
                            provider = name,
                            "Composite fallback succeeded after primary failure"
                        );
                    }
                    return Ok(r);
                }
                Ok(r) => {
                    tracing::debug!(
                        provider = name,
                        error = ?r.error,
                        "Provider returned business failure, trying next"
                    );
                    last_business_failure = Some(r);
                }
                Err(e) => {
                    self.circuits[i].record_failure();
                    let failures = self.circuits[i].consecutive_failures.load(Ordering::SeqCst);
                    tracing::warn!(
                        provider = name,
                        error = %e,
                        consecutive_failures = failures,
                        "Provider failed, trying next"
                    );
                    last_err = Some(e);
                }
            }
        }

        if skipped == self.providers.len() {
            return Err(anyhow::anyhow!("所有金融数据源均被熔断保护，请稍后再试"));
        }

        if let Some(err) = last_err {
            Err(err.context("所有金融数据源均失败"))
        } else if let Some(r) = last_business_failure {
            Ok(r)
        } else {
            Ok(FinancialResult::err("所有金融数据源均不可用"))
        }
    }
}
