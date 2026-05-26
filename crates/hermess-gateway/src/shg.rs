use crate::config::ShgConfig;

/// Short-Hard-Guard detector. Identifies short prompts that require
/// deep reasoning and should skip the lightweight classifier entirely.
#[derive(Debug, Clone)]
pub struct ShgDetector {
    enabled: bool,
    prompt_len_threshold: usize,
    patterns: Vec<String>,
    force_model: Option<String>,
}

impl ShgDetector {
    pub fn new(config: &ShgConfig) -> Self {
        Self {
            enabled: config.enabled,
            prompt_len_threshold: config.prompt_len_threshold,
            patterns: config.hard_patterns.clone(),
            force_model: config.force_model.clone(),
        }
    }

    /// Check a prompt. Returns Some(model_name) if SHG triggers,
    /// meaning the request should be routed directly to `force_model`.
    pub fn check(&self, prompt: &str) -> Option<String> {
        if !self.enabled || self.force_model.is_none() {
            return None;
        }
        if prompt.chars().count() > self.prompt_len_threshold {
            return None;
        }
        let lower = prompt.to_lowercase();
        for pat in &self.patterns {
            if lower.contains(&pat.to_lowercase()) {
                tracing::debug!(
                    pattern = %pat,
                    prompt_len = prompt.chars().count(),
                    "SHG triggered"
                );
                return self.force_model.clone();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shg() -> ShgDetector {
        ShgDetector {
            enabled: true,
            prompt_len_threshold: 200,
            patterns: vec![
                "时间复杂度".into(),
                "formal proof".into(),
                "cryptograph".into(),
            ],
            force_model: Some("claude-opus-4-6".into()),
        }
    }

    #[test]
    fn triggers_on_pattern_match() {
        let det = shg();
        let result = det.check("分析这段代码的时间复杂度");
        assert_eq!(result, Some("claude-opus-4-6".into()));
    }

    #[test]
    fn triggers_case_insensitive() {
        let det = shg();
        let result = det.check("Do a Formal Proof of this theorem");
        assert_eq!(result, Some("claude-opus-4-6".into()));
    }

    #[test]
    fn no_trigger_long_prompt() {
        let det = shg();
        let long = "a".repeat(201);
        let result = det.check(&long);
        assert_eq!(result, None);
    }

    #[test]
    fn no_trigger_no_match() {
        let det = shg();
        let result = det.check("write a hello world in python");
        assert_eq!(result, None);
    }

    #[test]
    fn disabled_detector_returns_none() {
        let mut det = shg();
        det.enabled = false;
        let result = det.check("分析时间复杂度");
        assert_eq!(result, None);
    }

    #[test]
    fn no_force_model_returns_none() {
        let mut det = shg();
        det.force_model = None;
        let result = det.check("分析时间复杂度");
        assert_eq!(result, None);
    }
}
