// crates/hermess-gateway/src/feedback.rs
// Runtime feedback learning for model routing.
//
// Each LLM call outcome (success/failure, latency) feeds into per-model
// EMA-based latency estimates and reliability scores. The strategy layer
// queries these adjusted scores so routing drifts toward empirically
// better models over time.
//
// State is persisted to .hermes_feedback.json on shutdown and reloaded
// on startup so routing intelligence survives restarts.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-model feedback accumulated from LLM call outcomes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerModelFeedback {
    /// Exponential moving average of observed latency (ms).
    pub latency_ema_ms: f64,
    /// Number of successful upstream calls.
    pub success_count: u64,
    /// Number of failed upstream calls (errors, timeouts).
    pub failure_count: u64,
}

/// JSON shape for persisting feedback state to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeedbackFile {
    models: HashMap<String, PerModelFeedback>,
}

/// Info about a persisted memory file from another instance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryFileInfo {
    pub instance: String,
    pub file: String,
    pub size_bytes: u64,
    pub model_count: usize,
}

/// Lock-free-read, write-guarded tracker for model performance feedback.
///
/// Reads (strategy decisions, health checks) only take a shared lock.
/// Writes (recording outcomes) take an exclusive lock but are short.
pub struct FeedbackTracker {
    models: RwLock<HashMap<String, PerModelFeedback>>,
    alpha: f64,
    /// True if state was loaded from a persisted file on startup.
    persisted: bool,
}

impl Default for FeedbackTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackTracker {
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            alpha: 0.2,
            persisted: false,
        }
    }

    /// Whether any data was loaded from disk on startup.
    pub fn is_persisted(&self) -> bool {
        self.persisted
    }

    /// Persist current feedback state to a JSON file (atomic: write to .tmp then rename).
    pub fn save_to_file(&self, path: &str) -> Result<(), String> {
        let map = self.models.read();
        let file = FeedbackFile {
            models: map.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| format!("serialize: {e}"))?;
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, &json).map_err(|e| format!("write {tmp}: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp} -> {path}: {e}"))?;
        tracing::info!(path, count = map.len(), "Saved feedback state");
        Ok(())
    }

    /// Load feedback state from a JSON file. Returns a fresh tracker if the
    /// file doesn't exist or can't be parsed.
    pub fn load_from_file(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(json) => match serde_json::from_str::<FeedbackFile>(&json) {
                Ok(file) => {
                    let count = file.models.len();
                    tracing::info!(path, count, "Loaded feedback state from disk");
                    Self {
                        models: RwLock::new(file.models),
                        alpha: 0.2,
                        persisted: true,
                    }
                }
                Err(e) => {
                    tracing::warn!(path, error = %e, "Corrupt feedback file, starting fresh");
                    Self::new()
                }
            },
            Err(e) => {
                tracing::debug!(path, error = %e, "No feedback file found, starting fresh");
                Self::new()
            }
        }
    }

    /// Merge feedback data from an external file into the current tracker.
    /// Success/failure counts are summed; latency EMA is blended by observation count.
    pub fn merge_from_file(&self, path: &str) -> Result<usize, String> {
        let other = Self::load_from_file(path);
        let other_map = other.models.read();
        if other_map.is_empty() {
            return Ok(0);
        }
        let count = other_map.len();
        let mut map = self.models.write();
        for (model, fb) in other_map.iter() {
            let entry = map.entry(model.clone()).or_default();
            let total_self = entry.success_count + entry.failure_count;
            let total_other = fb.success_count + fb.failure_count;
            let total = total_self + total_other;
            if total > 0 {
                entry.latency_ema_ms = (entry.latency_ema_ms * total_self as f64
                    + fb.latency_ema_ms * total_other as f64)
                    / total as f64;
            }
            entry.success_count += fb.success_count;
            entry.failure_count += fb.failure_count;
        }
        tracing::info!(path, models = count, "Merged feedback from file");
        Ok(count)
    }

    /// Replace current feedback state with data loaded from a file.
    pub fn replace_from_file(&self, path: &str) -> Result<usize, String> {
        let other = Self::load_from_file(path);
        let other_map = other.models.read();
        let count = other_map.len();
        let mut map = self.models.write();
        *map = other_map.clone();
        tracing::info!(path, models = count, "Replaced feedback from file");
        Ok(count)
    }

    /// Scan for available memory files and return instance names with metadata.
    pub fn list_available_files() -> Vec<MemoryFileInfo> {
        let mut files = Vec::new();
        let dir = match std::fs::read_dir(".") {
            Ok(d) => d,
            Err(_) => return files,
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let prefix = ".hermes_feedback_";
            let suffix = ".json";
            if name_str.starts_with(prefix) && name_str.ends_with(suffix) {
                let instance = &name_str[prefix.len()..name_str.len() - suffix.len()];
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let path = entry.path();
                // Quick peek to get model count without full parse
                let model_count = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<FeedbackFile>(&s).ok())
                    .map(|f| f.models.len())
                    .unwrap_or(0);
                files.push(MemoryFileInfo {
                    instance: instance.to_string(),
                    file: name_str.to_string(),
                    size_bytes: size,
                    model_count,
                });
            }
        }
        files.sort_by(|a, b| b.instance.cmp(&a.instance));
        files
    }

    /// Record a successful call with its observed latency.
    pub fn record_success(&self, model: &str, latency_ms: u64) {
        let mut map = self.models.write();
        let entry = map.entry(model.to_string()).or_default();
        entry.success_count += 1;
        entry.latency_ema_ms =
            self.alpha * latency_ms as f64 + (1.0 - self.alpha) * entry.latency_ema_ms;
    }

    /// Record a failed call (error or timeout).
    pub fn record_failure(&self, model: &str, latency_ms: u64) {
        let mut map = self.models.write();
        let entry = map.entry(model.to_string()).or_default();
        entry.failure_count += 1;
        // Still update the latency EMA — the model did respond (or timeout at known duration)
        entry.latency_ema_ms =
            self.alpha * latency_ms as f64 + (1.0 - self.alpha) * entry.latency_ema_ms;
    }

    /// Laplace-smoothed reliability estimate: 1.0 when no data, decays with failures.
    pub fn reliability(&self, model: &str) -> f64 {
        let map = self.models.read();
        match map.get(model) {
            Some(fb) => reliability_of(fb),
            None => 1.0,
        }
    }

    /// Adjusted speed estimate. Unreliable models get a latency penalty.
    /// Falls back to `base_speed` when no feedback has been collected.
    pub fn adjusted_speed_ms(&self, model: &str, base_speed: u64) -> u64 {
        let map = self.models.read();
        match map.get(model) {
            Some(fb) if fb.success_count + fb.failure_count > 0 => {
                let rel = reliability_of(fb);
                let adjusted = fb.latency_ema_ms * (1.0 + (1.0 - rel) * 2.0);
                adjusted as u64
            }
            _ => base_speed,
        }
    }

    /// Adjusted capability score. Unreliable models get a proportional downgrade.
    pub fn adjusted_capability(&self, model: &str, base_cap: f64) -> f64 {
        base_cap * self.reliability(model)
    }

    /// Number of models with feedback data.
    pub fn model_count(&self) -> usize {
        self.models.read().len()
    }

    /// Snapshot of feedback state for observability.
    pub fn snapshot(&self) -> FeedbackSnapshot {
        let map = self.models.read();
        let models: Vec<_> = map
            .iter()
            .map(|(name, fb)| ModelFeedbackSnapshot {
                name: name.clone(),
                latency_ema_ms: fb.latency_ema_ms,
                success_count: fb.success_count,
                failure_count: fb.failure_count,
                reliability: reliability_of(fb),
            })
            .collect();
        FeedbackSnapshot { models }
    }
}

/// Point-in-time snapshot for /health and /metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FeedbackSnapshot {
    pub models: Vec<ModelFeedbackSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelFeedbackSnapshot {
    pub name: String,
    pub latency_ema_ms: f64,
    pub success_count: u64,
    pub failure_count: u64,
    pub reliability: f64,
}

fn reliability_of(fb: &PerModelFeedback) -> f64 {
    let s = fb.success_count as f64;
    let f = fb.failure_count as f64;
    (s + 1.0) / (s + f + 2.0) // Laplace smoothing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_success_updates_ema() {
        let ft = FeedbackTracker::new();
        ft.record_success("m1", 100);
        ft.record_success("m1", 200);

        let snap = ft.snapshot();
        let m = snap.models.iter().find(|m| m.name == "m1").unwrap();
        assert_eq!(m.success_count, 2);
        assert_eq!(m.failure_count, 0);
        // EMA: 0.2*100 + 0.8*0 = 20; then 0.2*200 + 0.8*20 = 40 + 16 = 56
        assert!((m.latency_ema_ms - 56.0).abs() < 0.01);
    }

    #[test]
    fn reliability_laplace_smoothing() {
        let ft = FeedbackTracker::new();
        // No data → 1.0
        assert!((ft.reliability("unknown") - 1.0).abs() < 0.001);

        ft.record_success("m1", 100);
        // 1 success, 0 failures: (1+1)/(1+0+2) = 2/3 ≈ 0.667
        assert!((ft.reliability("m1") - 0.667).abs() < 0.01);

        ft.record_failure("m1", 200);
        // 1 success, 1 failure: (1+1)/(1+1+2) = 2/4 = 0.5
        assert!((ft.reliability("m1") - 0.5).abs() < 0.001);

        // Many successes
        for _ in 0..98 {
            ft.record_success("m1", 100);
        }
        // 99 success, 1 failure: (99+1)/(100+2) = 100/102 ≈ 0.98
        assert!((ft.reliability("m1") - 0.98).abs() < 0.01);
    }

    #[test]
    fn adjusted_speed_falls_back_to_base() {
        let ft = FeedbackTracker::new();
        assert_eq!(ft.adjusted_speed_ms("no-data", 500), 500);
    }

    #[test]
    fn adjusted_speed_penalizes_unreliable() {
        let ft = FeedbackTracker::new();
        ft.record_success("fast", 50);
        ft.record_success("fast", 50);

        ft.record_failure("flakey", 50);
        ft.record_failure("flakey", 50);

        let fast_speed = ft.adjusted_speed_ms("fast", 100);
        let flakey_speed = ft.adjusted_speed_ms("flakey", 100);

        // flakey should be noticeably slower due to reliability penalty
        assert!(
            flakey_speed > fast_speed,
            "flakey={flakey_speed} should exceed fast={fast_speed}"
        );
    }

    #[test]
    fn adjusted_capability_downgrades_unreliable() {
        let ft = FeedbackTracker::new();
        ft.record_failure("bad", 100);
        ft.record_failure("bad", 100);

        let adj = ft.adjusted_capability("bad", 0.9);
        // reliability ≈ 0.33 → adjusted = 0.9 * 0.33 ≈ 0.3
        assert!(adj < 0.5);
        assert!(adj > 0.2);
    }

    #[test]
    fn record_failure_updates_ema() {
        let ft = FeedbackTracker::new();
        ft.record_failure("m1", 500);
        ft.record_failure("m1", 600);

        let snap = ft.snapshot();
        let m = snap.models.iter().find(|m| m.name == "m1").unwrap();
        assert_eq!(m.failure_count, 2);
        // EMA: 0.2*500 + 0.8*0 = 100; 0.2*600 + 0.8*100 = 120 + 80 = 200
        assert!((m.latency_ema_ms - 200.0).abs() < 0.01);
    }

    #[test]
    fn snapshot_when_empty() {
        let ft = FeedbackTracker::new();
        let snap = ft.snapshot();
        assert!(snap.models.is_empty());
        assert_eq!(ft.model_count(), 0);
    }

    // --- persistence tests ---

    #[test]
    fn save_and_load_roundtrip() {
        let ft = FeedbackTracker::new();
        ft.record_success("m1", 100);
        ft.record_success("m1", 200);
        ft.record_failure("m2", 500);

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test_feedback.json");
        let path_str = path.to_str().unwrap();

        ft.save_to_file(path_str).unwrap();
        let loaded = FeedbackTracker::load_from_file(path_str);

        assert!(loaded.is_persisted());
        assert_eq!(loaded.model_count(), 2);
        // Verify m1 data survived
        let snap = loaded.snapshot();
        let m1 = snap.models.iter().find(|m| m.name == "m1").unwrap();
        assert_eq!(m1.success_count, 2);
        assert!((m1.latency_ema_ms - 56.0).abs() < 0.01);
        // m2 failure
        let m2 = snap.models.iter().find(|m| m.name == "m2").unwrap();
        assert_eq!(m2.failure_count, 1);
    }

    #[test]
    fn load_nonexistent_file_returns_fresh() {
        let ft = FeedbackTracker::load_from_file("/nonexistent/path/feedback.json");
        assert!(!ft.is_persisted());
        assert_eq!(ft.model_count(), 0);
    }

    #[test]
    fn fresh_tracker_is_not_persisted() {
        let ft = FeedbackTracker::new();
        assert!(!ft.is_persisted());
    }
}
