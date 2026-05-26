/// Compresses context to retain a configurable fraction of core information.
pub struct ContextDistiller {
    #[allow(dead_code)]
    keep_ratio: f64,
}

impl ContextDistiller {
    pub fn new(keep_ratio: f64) -> Self {
        Self { keep_ratio: keep_ratio.clamp(0.0, 1.0) }
    }

    /// Returns a prompt for an LLM that compresses history, or None if no compression needed.
    #[allow(dead_code)]
    pub fn distill_prompt(&self, history: &[(String, String)]) -> Option<String> {
        if history.is_empty() {
            return None;
        }

        let target_items = (history.len() as f64 * self.keep_ratio).max(1.0) as usize;
        let to_summarize = history.len().saturating_sub(target_items);
        if to_summarize == 0 {
            return None;
        }

        let entries: Vec<String> = history
            .iter()
            .take(to_summarize)
            .map(|(q, a)| format!("Q: {q}\nA: {a}"))
            .collect();

        Some(format!(
            "Compress the following conversation history into a concise summary. \
             Preserve key context, decisions, and outcomes.\n\n{}",
            entries.join("\n\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distill_empty_history() {
        let d = ContextDistiller::new(0.2);
        assert_eq!(d.distill_prompt(&[]), None);
    }

    #[test]
    fn distill_small_history() {
        let d = ContextDistiller::new(0.2);
        let history = vec![("Q1".into(), "A1".into()), ("Q2".into(), "A2".into())];
        let prompt = d.distill_prompt(&history);
        assert!(prompt.is_some());
    }

    #[test]
    fn distill_at_keep_ratio_one_returns_none() {
        let d = ContextDistiller::new(1.0);
        let history = vec![("Q1".into(), "A1".into()), ("Q2".into(), "A2".into())];
        assert_eq!(d.distill_prompt(&history), None);
    }
}
