/// Configuration for the SHG (Short-Hard-Guard) detector.
#[derive(Debug, Clone)]
pub struct ShgConfig {
    /// Whether SHG detection is enabled.
    pub enabled: bool,
    /// Maximum prompt length in characters for SHG to consider.
    pub prompt_len_threshold: usize,
    /// Patterns that indicate a "hard" prompt requiring deep reasoning.
    pub hard_patterns: Vec<String>,
    /// Model to route to when SHG triggers.
    pub force_model: Option<String>,
}

impl ShgConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            prompt_len_threshold: 0,
            hard_patterns: vec![],
            force_model: None,
        }
    }
}
