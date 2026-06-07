use crate::models::ModelEntry;

/// In-memory model registry keyed by model name.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    models: Vec<ModelEntry>,
}

#[allow(dead_code)]
impl ModelRegistry {
    pub fn new() -> Self {
        Self { models: Vec::new() }
    }

    pub fn from_entries(entries: Vec<ModelEntry>) -> Self {
        Self { models: entries }
    }

    pub fn add(&mut self, entry: ModelEntry) {
        self.models.push(entry);
    }

    pub fn get(&self, name: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.name == name)
    }

    pub fn all(&self) -> &[ModelEntry] {
        &self.models
    }

    /// Return all models matching every given tag.
    pub fn by_tags(&self, tags: &[String]) -> Vec<&ModelEntry> {
        self.models
            .iter()
            .filter(|m| tags.iter().all(|t| m.tags.contains(t)))
            .collect()
    }

    /// Return the cheapest model (lowest combined input+output cost).
    pub fn cheapest(&self) -> Option<&ModelEntry> {
        self.models.iter().min_by(|a, b| {
            (a.cost_per_1m_input + a.cost_per_1m_output)
                .partial_cmp(&(b.cost_per_1m_input + b.cost_per_1m_output))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Return the model with the highest capability score for a given dimension.
    pub fn most_capable(&self, dim: &str) -> Option<&ModelEntry> {
        self.models.iter().max_by(|a, b| {
            let va = match dim {
                "reasoning" => a.capability.reasoning,
                "coding" => a.capability.coding,
                "creative" => a.capability.creative,
                "knowledge" => a.capability.knowledge,
                _ => 0.5,
            };
            let vb = match dim {
                "reasoning" => b.capability.reasoning,
                "coding" => b.capability.coding,
                "creative" => b.capability.creative,
                "knowledge" => b.capability.knowledge,
                _ => 0.5,
            };
            va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Return the fastest model (lowest speed_ms).
    pub fn fastest(&self) -> Option<&ModelEntry> {
        self.models.iter().min_by_key(|m| m.capability.speed_ms)
    }

    /// Return model satisfying min capability score on a dimension, with lowest cost.
    pub fn best_balanced(&self, dim: &str, min_score: f64) -> Option<&ModelEntry> {
        let min_cap = move |m: &&ModelEntry| -> bool {
            let score = match dim {
                "reasoning" => m.capability.reasoning,
                "coding" => m.capability.coding,
                "creative" => m.capability.creative,
                "knowledge" => m.capability.knowledge,
                _ => 0.5,
            };
            score >= min_score
        };
        self.models.iter().filter(min_cap).min_by(|a, b| {
            (a.cost_per_1m_input + a.cost_per_1m_output)
                .partial_cmp(&(b.cost_per_1m_input + b.cost_per_1m_output))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelCapability;

    fn cheap_model() -> ModelEntry {
        ModelEntry {
            name: "cheap-model".into(),
            provider: "openai".into(),
            base_url: "http://c".into(),
            api_key: "k".into(),
            cost_per_1m_input: 0.1,
            cost_per_1m_output: 0.2,
            capability: ModelCapability {
                reasoning: 0.3,
                coding: 0.4,
                creative: 0.3,
                knowledge: 0.5,
                speed_ms: 100,
            },
            tags: vec!["fast".into(), "general".into()],
        }
    }

    fn smart_model() -> ModelEntry {
        ModelEntry {
            name: "smart-model".into(),
            provider: "anthropic".into(),
            base_url: String::new(),
            api_key: "k".into(),
            cost_per_1m_input: 15.0,
            cost_per_1m_output: 75.0,
            capability: ModelCapability {
                reasoning: 0.95,
                coding: 0.9,
                creative: 0.8,
                knowledge: 0.9,
                speed_ms: 2000,
            },
            tags: vec!["reasoning".into(), "complex".into()],
        }
    }

    fn mid_model() -> ModelEntry {
        ModelEntry {
            name: "mid-model".into(),
            provider: "openai".into(),
            base_url: "http://m".into(),
            api_key: "k".into(),
            cost_per_1m_input: 1.0,
            cost_per_1m_output: 4.0,
            capability: ModelCapability {
                reasoning: 0.7,
                coding: 0.8,
                creative: 0.6,
                knowledge: 0.7,
                speed_ms: 500,
            },
            tags: vec!["general".into(), "coding".into()],
        }
    }

    fn registry() -> ModelRegistry {
        ModelRegistry::from_entries(vec![cheap_model(), smart_model(), mid_model()])
    }

    #[test]
    fn get_by_name() {
        let reg = registry();
        assert!(reg.get("cheap-model").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn filter_by_tags() {
        let reg = registry();
        let reasoning: Vec<_> = reg
            .by_tags(&["reasoning".into()])
            .into_iter()
            .map(|m| &m.name)
            .collect();
        assert_eq!(reasoning, vec!["smart-model"]);
    }

    #[test]
    fn cheapest_model() {
        let reg = registry();
        assert_eq!(reg.cheapest().unwrap().name, "cheap-model");
    }

    #[test]
    fn most_capable_reasoning() {
        let reg = registry();
        assert_eq!(reg.most_capable("reasoning").unwrap().name, "smart-model");
    }

    #[test]
    fn fastest_model() {
        let reg = registry();
        assert_eq!(reg.fastest().unwrap().name, "cheap-model");
    }

    #[test]
    fn best_balanced_respects_min_score() {
        let reg = registry();
        let pick = reg.best_balanced("reasoning", 0.8);
        assert_eq!(pick.unwrap().name, "smart-model");
        let pick = reg.best_balanced("reasoning", 0.5);
        assert_eq!(pick.unwrap().name, "mid-model");
    }

    #[test]
    fn best_balanced_no_match_returns_none() {
        let reg = registry();
        assert!(reg.best_balanced("reasoning", 1.0).is_none());
    }
}
