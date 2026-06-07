// crates/evolution/src/insight.rs
// Insight data structures for the evolution engine.
use agent_core::Insight;

/// Summary statistics about the insights accumulated so far.
#[derive(Debug, Clone, Default)]
pub struct InsightStats {
    /// Total number of insights processed.
    pub total: u64,
    /// Number of positive insights (score > 0).
    pub positive: u64,
    /// Number of negative insights (score <= 0).
    pub negative: u64,
    /// Running average score.
    pub avg_score: f64,
    /// Highest score ever recorded.
    pub best_score: f64,
    /// Lowest score ever recorded.
    pub worst_score: f64,
}

impl InsightStats {
    pub fn update(&mut self, insight: &Insight) {
        self.total += 1;
        if insight.score > 0.0 {
            self.positive += 1;
        } else {
            self.negative += 1;
        }
        self.avg_score =
            (self.avg_score * (self.total - 1) as f64 + insight.score) / self.total as f64;
        // On the first insight, initialize best/worst unconditionally
        if self.total == 1 {
            self.best_score = insight.score;
            self.worst_score = insight.score;
        } else {
            if insight.score > self.best_score {
                self.best_score = insight.score;
            }
            if insight.score < self.worst_score {
                self.worst_score = insight.score;
            }
        }
    }

    /// Win rate: fraction of insights that were positive.
    pub fn win_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.positive as f64 / self.total as f64
        }
    }
}
