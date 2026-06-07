// crates/memory/src/working.rs
use std::collections::VecDeque;

use agent_core::MemoryChunk;
use parking_lot::RwLock;

/// Short-term working memory: fixed-capacity ring buffer with importance-aware eviction.
/// Thread-safe via parking_lot RwLock (multi-producer, multi-consumer).
///
/// When at capacity, the chunk with the lowest priority (importance × recency) is evicted.
/// Retrieved chunks are sorted by the same priority score.
pub struct WorkingMemory {
    buffer: RwLock<VecDeque<MemoryChunk>>,
    capacity: usize,
}

impl WorkingMemory {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Push a chunk. If at capacity, evict the one with the lowest importance.
    /// Chunks with importance=0 are evicted preferentially even before capacity.
    pub fn push(&self, chunk: MemoryChunk) {
        let mut buf = self.buffer.write();
        if buf.len() >= self.capacity {
            // Find the index of the chunk with the lowest importance
            let mut min_idx = 0;
            let mut min_imp = f64::MAX;
            for (i, c) in buf.iter().enumerate() {
                if c.importance < min_imp {
                    min_imp = c.importance;
                    min_idx = i;
                }
            }
            buf.remove(min_idx);
        }
        buf.push_back(chunk);
    }

    /// Return the `n` most important+recent chunks, sorted by priority descending.
    /// Priority = importance × (1.0 / (1.0 + age_minutes)).
    pub fn recent(&self, n: usize) -> Vec<MemoryChunk> {
        let buf = self.buffer.read();
        let now = chrono::Utc::now();
        let mut scored: Vec<(f64, &MemoryChunk)> = buf
            .iter()
            .map(|c| {
                let age_minutes = (now - c.timestamp).num_minutes().max(0) as f64;
                let recency = 1.0 / (1.0 + age_minutes * 0.1);
                let priority = c.importance * recency;
                (priority, c)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(n).map(|(_, c)| c.clone()).collect()
    }

    /// Return all chunks, sorted by priority (most important+recent first).
    pub fn all(&self) -> Vec<MemoryChunk> {
        self.recent(self.buffer.read().len())
    }

    pub fn len(&self) -> usize {
        self.buffer.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_chunk(content: &str) -> MemoryChunk {
        MemoryChunk {
            id: Uuid::new_v4(),
            content: content.to_string(),
            embedding: vec![0.0; 4],
            timestamp: chrono::Utc::now(),
            importance: 1.0,
        }
    }

    fn make_important_chunk(content: &str, imp: f64) -> MemoryChunk {
        MemoryChunk {
            id: Uuid::new_v4(),
            content: content.to_string(),
            embedding: vec![0.0; 4],
            timestamp: chrono::Utc::now(),
            importance: imp,
        }
    }

    #[test]
    fn test_capacity_eviction_evicts_least_important() {
        let wm = WorkingMemory::new(3);
        wm.push(make_important_chunk("low_priority", 0.5));
        wm.push(make_important_chunk("medium", 1.0));
        wm.push(make_important_chunk("high", 2.0));
        // Push a 4th chunk - should evict "low_priority" (importance=0.5)
        wm.push(make_chunk("new"));
        assert_eq!(wm.len(), 3);
        let contents: Vec<String> = wm.all().iter().map(|c| c.content.clone()).collect();
        assert!(
            !contents.contains(&"low_priority".to_string()),
            "least important should be evicted"
        );
        assert!(contents.contains(&"new".to_string()));
    }

    #[test]
    fn test_recent_sorts_by_priority() {
        let wm = WorkingMemory::new(5);
        wm.push(make_important_chunk("low", 0.5));
        wm.push(make_important_chunk("medium", 1.0));
        wm.push(make_important_chunk("high", 2.0));
        let top = wm.recent(2);
        // "high" should be first (highest priority)
        assert_eq!(top[0].content, "high");
        assert!(top[0].importance > top[1].importance);
    }
}
