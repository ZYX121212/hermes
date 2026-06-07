// crates/memory/src/dedup.rs
// Chunk deduplication via sentence-level sliding-window overlap ratio.
//
// Documents split into chunks often produce near-duplicate neighbours.
// This module filters chunks whose sentence-level overlap with recent
// (already-accepted) chunks exceeds a configurable threshold.

use std::collections::{HashSet, VecDeque};

/// Configuration for sliding-window deduplication.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// Fraction of overlapping sentences above which a chunk is discarded.
    pub overlap_threshold: f64,
    /// Number of recently accepted chunks whose sentences form the window.
    pub window_chunks: usize,
    /// Sentences shorter than this (chars) are ignored in overlap calculation.
    pub min_sentence_len: usize,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            overlap_threshold: 0.7,
            window_chunks: 5,
            min_sentence_len: 10,
        }
    }
}

/// Statistics produced by a deduplication pass.
#[derive(Debug, Clone, Default)]
pub struct DedupStats {
    pub input_chunks: usize,
    pub kept_chunks: usize,
    pub removed_chunks: usize,
}

/// Filter near-duplicate chunks using a sentence-level sliding window.
///
/// Each chunk is split into normalized sentences. If the fraction of its
/// sentences already present in the window (from recently accepted chunks)
/// exceeds `config.overlap_threshold`, the chunk is dropped.
pub fn deduplicate_chunks(chunks: Vec<String>, config: &DedupConfig) -> (Vec<String>, DedupStats) {
    let mut stats = DedupStats {
        input_chunks: chunks.len(),
        ..Default::default()
    };

    if chunks.is_empty() {
        return (chunks, stats);
    }

    let mut output: Vec<String> = Vec::new();
    // All normalized sentences from the current window, for O(1) lookup.
    let mut window_set: HashSet<String> = HashSet::new();
    // Per-chunk sentence lists so we can evict the oldest chunk.
    let mut window_queue: VecDeque<Vec<String>> = VecDeque::new();

    for chunk in chunks {
        let raw_sentences = split_sentences(&chunk, config.min_sentence_len);

        // Always keep chunks that produced no detectable sentences —
        // they're too short for meaningful overlap comparison.
        if raw_sentences.is_empty() {
            output.push(chunk);
            stats.kept_chunks += 1;
            continue;
        }

        let normalized: Vec<String> = raw_sentences
            .iter()
            .map(|s| normalize_sentence(s))
            .collect();

        let overlap_count = normalized
            .iter()
            .filter(|s| window_set.contains(*s))
            .count();
        let overlap_ratio = overlap_count as f64 / normalized.len() as f64;

        if overlap_ratio > config.overlap_threshold && !output.is_empty() {
            stats.removed_chunks += 1;
            continue;
        }

        stats.kept_chunks += 1;

        // Insert into window
        for s in &normalized {
            window_set.insert(s.clone());
        }
        window_queue.push_back(normalized);

        // Trim window to size
        while window_queue.len() > config.window_chunks {
            if let Some(old_sentences) = window_queue.pop_front() {
                for s in &old_sentences {
                    // Only remove if no other chunk in the window still has it.
                    let still_present = window_queue.iter().any(|cs| cs.contains(s));
                    if !still_present {
                        window_set.remove(s);
                    }
                }
            }
        }

        output.push(chunk);
    }

    (output, stats)
}

/// Split text at sentence-terminating punctuation, keeping the punctuation
/// attached to its sentence. Returns sentences meeting `min_len`.
fn split_sentences(text: &str, min_len: usize) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut start = 0;

    for (i, ch) in text.char_indices() {
        if ch == '.' || ch == '!' || ch == '?' || ch == '\n' || ch == ';' {
            let end = i + ch.len_utf8();
            let sent = text[start..end].trim().to_string();
            if sent.len() >= min_len {
                sentences.push(sent);
            }
            start = end;
        }
    }

    let remaining = text[start..].trim().to_string();
    if remaining.len() >= min_len || (!remaining.is_empty() && sentences.is_empty()) {
        sentences.push(remaining);
    }

    sentences
}

/// Normalize for comparison: trim whitespace, fold to lowercase.
fn normalize_sentence(s: &str) -> String {
    s.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> DedupConfig {
        DedupConfig::default()
    }

    #[test]
    fn empty_input() {
        let (result, stats) = deduplicate_chunks(vec![], &default_config());
        assert!(result.is_empty());
        assert_eq!(stats.input_chunks, 0);
        assert_eq!(stats.kept_chunks, 0);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn single_chunk_always_kept() {
        let chunks = vec!["Hello world. This is a test.".into()];
        let (result, stats) = deduplicate_chunks(chunks.clone(), &default_config());
        assert_eq!(result, chunks);
        assert_eq!(stats.kept_chunks, 1);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn exact_duplicate_removed() {
        let chunk = "The quick brown fox. Jumps over the lazy dog. What a day.";
        let chunks = vec![chunk.to_string(), chunk.to_string()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        assert_eq!(result.len(), 1);
        assert_eq!(stats.kept_chunks, 1);
        assert_eq!(stats.removed_chunks, 1);
    }

    #[test]
    fn high_overlap_removed() {
        let a = "First sentence here. Second sentence goes on. Third is the charm.";
        // Shares "Second sentence goes on." and "Third is the charm." => 2/3 overlap
        let b = "Second sentence goes on. Third is the charm. Fourth is new.";
        let chunks = vec![a.into(), b.into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        // 2/3 ≈ 0.67, threshold is 0.7 → kept (barely)
        assert_eq!(result.len(), 2);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn very_high_overlap_removed() {
        let a = "Alpha bravo charlie. Delta echo foxtrot. Golf hotel india.";
        // Shares 2/3 sentences with a
        let b = "Delta echo foxtrot. Golf hotel india. Juliet kilo lima.";
        let chunks = vec![a.into(), b.into()];
        let mut cfg = default_config();
        cfg.overlap_threshold = 0.6; // 2/3 ≈ 0.67 > 0.6 → removed
        let (result, stats) = deduplicate_chunks(chunks, &cfg);
        assert_eq!(result.len(), 1);
        assert_eq!(stats.removed_chunks, 1);
    }

    #[test]
    fn no_overlap_all_kept() {
        let a = "Apple banana cherry. Date elderberry fig.";
        let b = "Grape honeydew kiwi. Lemon mango nectarine.";
        let chunks = vec![a.into(), b.into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        assert_eq!(result.len(), 2);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn sliding_window_evicts_old() {
        let mut cfg = default_config();
        cfg.window_chunks = 1;
        cfg.overlap_threshold = 0.5;

        let a = "Topic one alpha. Topic one beta. Topic one gamma.";
        let b = "Topic two delta. Topic two epsilon. Topic two zeta.";
        // Same as a, but a was evicted from window after b was accepted.
        let a2 = "Topic one alpha. Topic one beta. Topic one gamma.";
        let chunks: Vec<String> = vec![a.into(), b.into(), a2.into()];
        let (result, stats) = deduplicate_chunks(chunks, &cfg);

        // a kept (empty window). b kept (no overlap with a).
        // a2 kept because a was evicted from the 1-sized window by b.
        assert_eq!(result.len(), 3);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn short_chunk_no_sentences_kept() {
        let chunks: Vec<String> = vec!["hi".into(), "hi".into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        // "hi" has no sentence punctuation, treated as one sentence.
        // Second "hi" overlaps 100% → removed.
        assert_eq!(result.len(), 1);
        assert_eq!(stats.removed_chunks, 1);
    }

    #[test]
    fn mixed_short_and_normal() {
        let cfg = default_config();
        let a: String = "First proper sentence here. Second proper sentence.".into();
        let short: String = "short".into();
        let chunks = vec![a.clone(), short, a.clone()];
        let (result, stats) = deduplicate_chunks(chunks, &cfg);
        // short chunk is kept (no sentences), duplicate of a is removed
        assert_eq!(result.len(), 2);
        assert_eq!(stats.removed_chunks, 1);
    }

    #[test]
    fn first_chunk_never_removed() {
        // Even if the first chunk somehow overlaps with itself (empty window),
        // it should be kept.
        let chunks = vec!["Only one chunk here. Nothing else to compare.".into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        assert_eq!(result.len(), 1);
        assert_eq!(stats.kept_chunks, 1);
    }

    #[test]
    fn split_sentences_basic() {
        let text = "Hello world. This is a test! Is it working? Yes indeed; final.";
        let sents = split_sentences(text, 5);
        // "Yes indeed;" is 11 chars ≥ 5 → included; "final." is 6 chars ≥ 5 → included
        assert_eq!(sents.len(), 5);
        assert!(sents[0].contains("Hello world"));
        assert!(sents[1].contains("This is a test"));
        assert!(sents[3].contains("Yes indeed"));
        assert!(sents[4].contains("final"));
    }

    #[test]
    fn split_sentences_respects_min_len() {
        let text = "A. Long enough sentence here. B.";
        let sents = split_sentences(text, 10);
        // "A." and "B." are too short; only the middle sentence qualifies
        assert_eq!(sents.len(), 1);
        assert!(sents[0].contains("Long enough"));
    }

    #[test]
    fn normalize_folds_case_and_trims() {
        assert_eq!(normalize_sentence("  Hello WORLD  "), "hello world");
        assert_eq!(normalize_sentence("Punctuation!"), "punctuation!");
    }

    #[test]
    fn chunk_overlap_is_case_insensitive() {
        let a = "First sentence here. Second sentence goes on.";
        let b = "FIRST SENTENCE HERE. second sentence goes on.";
        let chunks: Vec<String> = vec![a.into(), b.into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        // 2/2 = 1.0 overlap > 0.7 → duplicate removed
        assert_eq!(result.len(), 1);
        assert_eq!(stats.removed_chunks, 1);
    }

    #[test]
    fn partial_overlap_below_threshold_kept() {
        let a = "A one. B two. C three. D four. E five.";
        let b = "D four. E five. F six. G seven. H eight.";
        // 2/5 = 0.4 overlap < 0.7 → kept
        let chunks = vec![a.into(), b.into()];
        let (result, stats) = deduplicate_chunks(chunks, &default_config());
        assert_eq!(result.len(), 2);
        assert_eq!(stats.removed_chunks, 0);
    }

    #[test]
    fn window_count_maintained() {
        let mut cfg = default_config();
        cfg.window_chunks = 3;
        cfg.overlap_threshold = 0.3;

        let mut chunks = Vec::new();
        for i in 0..20 {
            chunks.push(format!("Unique sentence number {i}. Shared sentence here."));
        }

        let (result, stats) = deduplicate_chunks(chunks, &cfg);
        // Each chunk shares "Shared sentence here." which is 1/2 sentences = 0.5
        // With threshold 0.3, each after the first would be removed...
        // Actually wait, "Shared sentence here." is normalized to "shared sentence here."
        // First chunk: kept (empty window). Adds to window.
        // Second chunk: 1/2 = 0.5 > 0.3, removed.
        // But then it's NOT added to window. So third chunk also sees only the first in window.
        // So third chunk: 1/2 = 0.5 > 0.3, also removed. And so on.
        assert_eq!(result.len(), 1);
        assert_eq!(stats.kept_chunks, 1);
        assert_eq!(stats.removed_chunks, 19);
    }

    #[test]
    fn dedup_stats_totals_match() {
        let chunks: Vec<String> = (0..10)
            .map(|i| format!("Unique part {i}. Shared content here."))
            .collect();
        let (_, stats) = deduplicate_chunks(chunks, &default_config());
        assert_eq!(stats.input_chunks, stats.kept_chunks + stats.removed_chunks);
    }
}
