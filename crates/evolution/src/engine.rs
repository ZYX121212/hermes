// crates/evolution/src/engine.rs
// Core evolution engine: manages strategy weights lock-free
// and persists insights to long-term memory asynchronously.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::{Insight, MemoryStore};
use dashmap::DashMap;

use crate::insight::InsightStats;
use crate::weight::{adaptive_lr, clamp};

/// Per-tool execution statistics for reliability-based planning.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ToolStat {
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
}

impl ToolStat {
    /// Reliability score in [0.0, 1.0]. Uses Laplace smoothing (start at 0.5).
    pub fn reliability(&self) -> f64 {
        let total = self.successes + self.failures;
        if total == 0 {
            return 0.5; // unknown → neutral
        }
        // Laplace smoothing: (s + 1) / (n + 2), starts at 0.5 for n=0
        (self.successes as f64 + 1.0) / (total as f64 + 2.0)
    }

    pub fn avg_latency_ms(&self) -> f64 {
        let total = self.successes + self.failures;
        if total == 0 {
            return 0.0;
        }
        self.total_latency_ms as f64 / total as f64
    }
}

/// Core evolution engine: manages strategy weights lock-free
/// and persists insights to long-term memory asynchronously.
///
/// # Concurrency
/// - Strategy weights: DashMap (lock-free concurrent HashMap)
/// - Learning rate: AtomicU64 storing f64 bits
/// - Insight count: AtomicU64 for adaptive learning-rate decay
///
/// # Memory writes
/// Long-term memory writes are spawned as detached tokio tasks.
/// Failures are logged but do not block the main evolution loop.
pub struct EvolutionEngine {
    /// Strategy ID -> weight score (lock-free concurrent HashMap).
    strategy_weights: Arc<DashMap<String, f64>>,
    /// Per-tool execution statistics for reliability-based planning.
    tool_stats: Arc<DashMap<String, ToolStat>>,
    /// Long-term vector memory (Qdrant backed).
    memory_store: Arc<dyn MemoryStore>,
    /// Learning rate stored as f64 bits in an atomic u64 for lock-free access.
    learning_rate_bits: AtomicU64,
    /// Total number of insights processed (for adaptive learning-rate decay).
    insight_count: AtomicU64,
    /// Count since last auto-save.
    insights_since_save: AtomicU64,
    /// Auto-save path (None = no auto-save).
    auto_save_path: Option<String>,
    /// Accumulated statistics about insights.
    stats: parking_lot::RwLock<InsightStats>,
}

impl EvolutionEngine {
    pub fn new(lr: f64, memory: Arc<dyn MemoryStore>) -> Self {
        Self {
            strategy_weights: Arc::new(DashMap::new()),
            tool_stats: Arc::new(DashMap::new()),
            memory_store: memory,
            learning_rate_bits: AtomicU64::new(lr.to_bits()),
            insight_count: AtomicU64::new(0),
            insights_since_save: AtomicU64::new(0),
            auto_save_path: None,
            stats: parking_lot::RwLock::new(InsightStats::default()),
        }
    }

    /// Enable automatic periodic persistence to the given file path.
    /// Saves after every 5 insights and on extreme scores.
    pub fn with_auto_save(mut self, path: &str) -> Self {
        self.auto_save_path = Some(path.to_string());
        self
    }

    /// Perform a single evolution update: adjust strategy weight
    /// and asynchronously persist the insight to long-term memory.
    pub async fn update(&self, insight: Insight) -> anyhow::Result<()> {
        // 1. Update stats
        {
            let mut stats = self.stats.write();
            stats.update(&insight);
        }

        // 2. Adaptive learning rate decay: lr_t = lr_0 / sqrt(n + 1)
        let base_lr = f64::from_bits(self.learning_rate_bits.load(Ordering::Relaxed));
        let n = self.insight_count.fetch_add(1, Ordering::Relaxed);
        let lr_t = adaptive_lr(base_lr, n);
        let delta = insight.score * lr_t;

        // 3. Strategy weight update — get_mut holds the shard lock for atomic read-modify-write
        let strategy_id = insight.strategy_id.clone();
        let old_weight = self.strategy_weights.get(&strategy_id).map(|w| *w);
        match self.strategy_weights.get_mut(&strategy_id) {
            Some(mut w) => *w = clamp(*w + delta, -10.0, 10.0),
            None => {
                self.strategy_weights
                    .insert(strategy_id.clone(), clamp(delta, -10.0, 10.0));
            }
        }
        let new_weight = self
            .strategy_weights
            .get(&strategy_id)
            .map(|w| *w)
            .unwrap_or(0.0);

        tracing::info!(
            strategy = %strategy_id,
            old = old_weight.unwrap_or(0.0),
            new = new_weight,
            delta,
            lr = lr_t,
            score = insight.score,
            "evolution update"
        );

        // 4. Async write to long-term memory (failure does not block the main loop)
        let store = Arc::clone(&self.memory_store);
        let chunk = agent_core::MemoryChunk {
            id: uuid::Uuid::new_v4(),
            content: insight.lesson.clone(),
            embedding: insight.embedding.clone(),
            timestamp: chrono::Utc::now(),
            importance: if insight.score < 0.0 { 1.5 } else { 1.0 },
        };
        let handle = tokio::spawn(async move {
            if let Err(e) = store.upsert(chunk).await {
                tracing::warn!(error = %e, "memory upsert failed");
            }
        });
        tokio::spawn(async move {
            match handle.await {
                Ok(()) => {}
                Err(join_err) => {
                    tracing::warn!(error = %join_err, "Memory upsert task panicked");
                }
            }
        });

        // 5. Auto-save evolution state periodically (every 5 insights or extreme scores)
        let since_save = self.insights_since_save.fetch_add(1, Ordering::Relaxed) + 1;
        let should_save = since_save >= 5 || insight.score.abs() > 0.8;
        if should_save {
            if let Some(ref path) = self.auto_save_path {
                self.insights_since_save.store(0, Ordering::Relaxed);
                let path = path.clone();
                let weights = self.strategy_weights.clone();
                let tool_stats = self.tool_stats.clone();
                let stats = self.stats.read().clone();
                let insight_count = self.insight_count.load(Ordering::Relaxed);
                let lr_bits = self.learning_rate_bits.load(Ordering::Relaxed);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = Self::save_snapshot(
                        &path,
                        &weights,
                        &tool_stats,
                        &stats,
                        insight_count,
                        lr_bits,
                    ) {
                        tracing::warn!(error = %e, "auto-save evolution state failed");
                    }
                });
            }
        }

        Ok(())
    }

    /// Search long-term memory for insights relevant to the given query.
    pub async fn search_memory(
        &self,
        query: &str,
        k: usize,
    ) -> anyhow::Result<Vec<agent_core::MemoryChunk>> {
        self.memory_store.search(query, k).await
    }

    /// Record a tool execution result for reliability tracking.
    /// Called by the agent after each step completes.
    pub fn record_tool_result(&self, tool: &str, success: bool, duration_ms: u64) {
        use dashmap::mapref::entry::Entry;
        match self.tool_stats.entry(tool.to_string()) {
            Entry::Occupied(mut e) => {
                let s = e.get_mut();
                if success {
                    s.successes += 1;
                } else {
                    s.failures += 1;
                }
                s.total_latency_ms += duration_ms;
            }
            Entry::Vacant(e) => {
                e.insert(ToolStat {
                    successes: if success { 1 } else { 0 },
                    failures: if success { 0 } else { 1 },
                    total_latency_ms: duration_ms,
                });
            }
        }
    }

    /// Get the reliability score for a specific tool (0.0–1.0).
    /// Returns None if the tool has never been executed.
    pub fn tool_reliability(&self, tool: &str) -> Option<f64> {
        self.tool_stats.get(tool).map(|s| s.reliability())
    }

    /// Get the average latency for a specific tool in milliseconds.
    pub fn tool_avg_latency(&self, tool: &str) -> Option<f64> {
        self.tool_stats.get(tool).map(|s| s.avg_latency_ms())
    }

    /// Get all tool stats for debugging/display.
    pub fn all_tool_stats(&self) -> Vec<(String, ToolStat)> {
        self.tool_stats
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Query the best strategy from a set of candidates based on learned weights.
    /// Returns None if no candidates have been learned yet.
    pub fn best_strategy(&self, candidates: &[&str]) -> Option<String> {
        candidates
            .iter()
            .filter_map(|&s| self.strategy_weights.get(s).map(|w| (s.to_string(), *w)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, _)| s)
    }

    /// Get the current weight for a strategy.
    pub fn get_weight(&self, strategy_id: &str) -> Option<f64> {
        self.strategy_weights.get(strategy_id).map(|w| *w)
    }

    /// Get all strategy weights as a sorted vector (best first).
    pub fn all_weights(&self) -> Vec<(String, f64)> {
        let mut weights: Vec<(String, f64)> = self
            .strategy_weights
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        weights.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        weights
    }

    /// Number of strategies ever learned.
    pub fn strategy_count(&self) -> usize {
        self.strategy_weights.len()
    }

    /// Current insight statistics.
    pub fn stats(&self) -> InsightStats {
        self.stats.read().clone()
    }

    /// Total number of insights processed (for display).
    pub fn insight_count(&self) -> u64 {
        self.insight_count.load(Ordering::Relaxed)
    }

    /// Current effective learning rate (after decay).
    pub fn current_learning_rate(&self) -> f64 {
        let base = f64::from_bits(self.learning_rate_bits.load(Ordering::Relaxed));
        let n = self.insight_count.load(Ordering::Relaxed);
        adaptive_lr(base, n)
    }

    /// Save evolution state to a JSON file.
    pub fn save_to_file(&self, path: &str) -> anyhow::Result<()> {
        Self::save_snapshot(
            path,
            &self.strategy_weights,
            &self.tool_stats,
            &self.stats.read(),
            self.insight_count.load(Ordering::Relaxed),
            self.learning_rate_bits.load(Ordering::Relaxed),
        )
    }

    /// Static helper used by both manual and auto-save paths.
    fn save_snapshot(
        path: &str,
        weights: &DashMap<String, f64>,
        tool_stats: &DashMap<String, ToolStat>,
        stats: &InsightStats,
        insight_count: u64,
        lr_bits: u64,
    ) -> anyhow::Result<()> {
        let w: serde_json::Value = weights
            .iter()
            .map(|e| (e.key().clone(), serde_json::json!(*e.value())))
            .collect::<serde_json::Map<_, _>>()
            .into();

        let tools: serde_json::Value = tool_stats
            .iter()
            .map(|e| {
                let s = e.value();
                serde_json::json!({
                    "name": e.key().as_str(),
                    "successes": s.successes,
                    "failures": s.failures,
                    "total_latency_ms": s.total_latency_ms,
                })
            })
            .collect::<Vec<_>>()
            .into();

        let state = serde_json::json!({
            "strategy_weights": w,
            "tool_stats": tools,
            "insight_count": insight_count,
            "learning_rate": f64::from_bits(lr_bits),
            "stats": {
                "total": stats.total,
                "positive": stats.positive,
                "negative": stats.negative,
                "avg_score": stats.avg_score,
                "best_score": stats.best_score,
                "worst_score": stats.worst_score,
            }
        });

        std::fs::write(path, serde_json::to_string_pretty(&state)?)?;
        tracing::debug!(
            "Evolution state saved to {path} ({} strategies, {} insights)",
            weights.len(),
            insight_count
        );
        Ok(())
    }

    /// Load evolution state from a JSON file, restoring weights, tool stats, and counters.
    pub fn load_from_file(
        path: &str,
        lr: f64,
        memory: Arc<dyn MemoryStore>,
    ) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let state: serde_json::Value = serde_json::from_str(&data)?;

        let engine = Self::new(lr, memory);

        if let Some(weights) = state["strategy_weights"].as_object() {
            for (k, v) in weights {
                if let Some(w) = v.as_f64() {
                    engine.strategy_weights.insert(k.clone(), w);
                }
            }
        }

        // Restore tool stats
        if let Some(tools) = state["tool_stats"].as_array() {
            for t in tools {
                if let (Some(name), Some(successes), Some(failures)) = (
                    t.get("name").and_then(|v| v.as_str()),
                    t.get("successes").and_then(|v| v.as_u64()),
                    t.get("failures").and_then(|v| v.as_u64()),
                ) {
                    engine.tool_stats.insert(
                        name.to_string(),
                        ToolStat {
                            successes,
                            failures,
                            total_latency_ms: t
                                .get("total_latency_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                        },
                    );
                }
            }
        }

        if let Some(n) = state["insight_count"].as_u64() {
            engine.insight_count.store(n, Ordering::Relaxed);
        }

        if let Some(stats) = state["stats"].as_object() {
            let mut s = engine.stats.write();
            s.total = stats.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
            s.positive = stats.get("positive").and_then(|v| v.as_u64()).unwrap_or(0);
            s.negative = stats.get("negative").and_then(|v| v.as_u64()).unwrap_or(0);
            s.avg_score = stats
                .get("avg_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            s.best_score = stats
                .get("best_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            s.worst_score = stats
                .get("worst_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
        }

        tracing::info!(
            "Loaded evolution state from {path}: {} strategies, {} tools, {} insights",
            engine.strategy_count(),
            engine.tool_stats.len(),
            engine.insight_count.load(Ordering::Relaxed)
        );
        Ok(engine)
    }
}
