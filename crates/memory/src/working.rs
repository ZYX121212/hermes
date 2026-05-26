// crates/memory/src/working.rs
use std::collections::VecDeque;

use agent_core::MemoryChunk;
use parking_lot::RwLock;

/// Short-term working memory: fixed-capacity ring buffer.
/// Thread-safe via parking_lot RwLock (multi-producer, multi-consumer).
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

    /// Push a chunk onto the ring buffer, evicting the oldest if at capacity.
    pub fn push(&self, chunk: MemoryChunk) {
        let mut buf = self.buffer.write();
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(chunk);
    }

    /// Return the `n` most recent chunks (newest first).
    pub fn recent(&self, n: usize) -> Vec<MemoryChunk> {
        let buf = self.buffer.read();
        buf.iter().rev().take(n).cloned().collect()
    }

    /// Return all chunks.
    pub fn all(&self) -> Vec<MemoryChunk> {
        let buf = self.buffer.read();
        buf.iter().cloned().collect()
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
        }
    }

    #[test]
    fn test_capacity_eviction() {
        let wm = WorkingMemory::new(3);
        wm.push(make_chunk("a"));
        wm.push(make_chunk("b"));
        wm.push(make_chunk("c"));
        wm.push(make_chunk("d"));
        assert_eq!(wm.len(), 3);
        let recent = wm.recent(3);
        assert_eq!(recent[0].content, "d");
        assert_eq!(recent[2].content, "b"); // "a" was evicted
    }
}
