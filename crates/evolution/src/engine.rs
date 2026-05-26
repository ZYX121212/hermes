// crates/evolution/src/engine.rs
// Core evolution engine: manages strategy weights lock-free
// and persists insights to long-term memory asynchronously.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_core::{Insight, MemoryStore};
use dashmap::DashMap;

use crate::insight::InsightStats;
use crate::weight::{adaptive_lr, clamp};

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
    /// Long-term vector memory (Qdrant backed).
    memory_store: Arc<dyn MemoryStore>,
    /// Learning rate stored as f64 bits in an atomic u64 for lock-free access.
    learning_rate_bits: AtomicU64,
    /// Total number of insights processed (for adaptive learning-rate decay).
    insight_count: AtomicU64,
    /// Accumulated statistics about insights.
    stats: parking_lot::RwLock<InsightStats>,
}

impl EvolutionEngine {
    pub fn new(lr: f64, memory: Arc<dyn MemoryStore>) -> Self {
        Self {
            strategy_weights: Arc::new(DashMap::new()),
            memory_store: memory,
            learning_rate_bits: AtomicU64::new(lr.to_bits()),
            insight_count: AtomicU64::new(0),
            stats: parking_lot::RwLock::new(InsightStats::default()),
        }
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

        Ok(())
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
        let weights: serde_json::Value = self
            .strategy_weights
            .iter()
            .map(|e| (e.key().clone(), serde_json::json!(*e.value())))
            .collect::<serde_json::Map<_, _>>()
            .into();

        let stats = self.stats.read();
        let state = serde_json::json!({
            "strategy_weights": weights,
            "insight_count": self.insight_count.load(Ordering::Relaxed),
            "learning_rate": f64::from_bits(self.learning_rate_bits.load(Ordering::Relaxed)),
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
        tracing::info!("Evolution state saved to {path}");
        Ok(())
    }

    /// Load evolution state from a JSON file, restoring weights and counters.
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
            "Loaded evolution state from {path}: {} strategies, {} insights",
            engine.strategy_count(),
            engine.insight_count.load(Ordering::Relaxed)
        );
        Ok(engine)
    }
}
