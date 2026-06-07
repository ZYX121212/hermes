use crate::config::DiscoveryProvider;
use crate::models::{ModelCapability, ModelEntry};

/// Query an upstream provider's GET /v1/models and return discovered ModelEntry items.
pub async fn discover(provider: &DiscoveryProvider) -> Vec<ModelEntry> {
    let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
    tracing::info!(%url, "Discovering models from upstream");

    let client = reqwest::Client::new();
    let resp = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(%url, error = %e, "Failed to discover models from upstream");
            return vec![];
        }
    };

    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(%url, error = %e, "Failed to read discovery response body");
            return vec![];
        }
    };

    if !status.is_success() {
        tracing::warn!(%url, %status, %body_text, "Discovery request failed");
        return vec![];
    }

    let parsed: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(%url, error = %e, "Discovery response is not valid JSON");
            return vec![];
        }
    };

    let models = parsed["data"]
        .as_array()
        .map(|arr| arr.iter())
        .unwrap_or_else(|| {
            tracing::warn!(%url, "Discovery response has no 'data' array");
            [].iter()
        });

    let mut entries = Vec::new();
    for m in models {
        let id = m["id"].as_str().unwrap_or("unknown");
        // Skip non-chat models (embedding, moderation, etc.)
        let id_lower = id.to_lowercase();
        if id_lower.contains("embed") || id_lower.contains("moderation") || id_lower.contains("tts")
        {
            continue;
        }
        // Apply name prefix filter if configured
        if let Some(ref prefix) = provider.name_prefix {
            if !id_lower.starts_with(&prefix.to_lowercase()) {
                continue;
            }
        }
        let (cost_in, cost_out) = infer_cost(
            id,
            provider.default_cost_input,
            provider.default_cost_output,
        );
        entries.push(ModelEntry {
            name: id.to_string(),
            provider: provider.provider.clone(),
            base_url: provider.base_url.clone(),
            api_key: provider.api_key.clone(),
            cost_per_1m_input: cost_in,
            cost_per_1m_output: cost_out,
            capability: infer_capability(id),
            tags: provider.default_tags.clone(),
        });
    }

    tracing::info!(%url, count = entries.len(), "Discovered models");
    entries
}

/// Heuristically assign capability scores based on model name.
fn infer_capability(model_name: &str) -> ModelCapability {
    let lower = model_name.to_lowercase();
    // Reasoning / pro models
    if lower.contains("reasoner") || lower.contains("pro") || lower.contains("opus") {
        return ModelCapability {
            reasoning: 0.9,
            coding: 0.85,
            creative: 0.8,
            knowledge: 0.85,
            speed_ms: 1000,
        };
    }
    // Flash / turbo / lite models
    if lower.contains("flash") || lower.contains("turbo") || lower.contains("lite") {
        return ModelCapability {
            reasoning: 0.4,
            coding: 0.5,
            creative: 0.45,
            knowledge: 0.5,
            speed_ms: 80,
        };
    }
    // Sonnet / balanced models
    if lower.contains("sonnet") || lower.contains("chat") || lower.contains("v4") {
        return ModelCapability {
            reasoning: 0.65,
            coding: 0.8,
            creative: 0.55,
            knowledge: 0.7,
            speed_ms: 200,
        };
    }
    // Default
    ModelCapability {
        reasoning: 0.5,
        coding: 0.6,
        creative: 0.5,
        knowledge: 0.6,
        speed_ms: 300,
    }
}

/// Heuristically infer cost tier from model name.
/// Returns (cost_input, cost_output) per 1M tokens.
/// Falls back to the provider defaults if the model name doesn't match a known tier.
pub fn infer_cost(model_name: &str, default_input: f64, default_output: f64) -> (f64, f64) {
    let lower = model_name.to_lowercase();
    // Reasoning / pro tier — most expensive
    if lower.contains("reasoner") || lower.contains("pro") || lower.contains("opus") {
        return (default_input * 4.0, default_output * 4.0);
    }
    // Flash / turbo / lite tier — cheapest, check before v4/v3 to avoid e.g. "v4-flash" matching v4 first
    if lower.contains("flash") || lower.contains("turbo") || lower.contains("lite") {
        return (default_input * 0.5, default_output * 0.5);
    }
    // Sonnet / chat tier — medium cost
    if lower.contains("sonnet")
        || lower.contains("chat")
        || lower.contains("v4")
        || lower.contains("v3")
    {
        return (default_input * 1.5, default_output * 1.5);
    }
    // Unknown tier — use defaults as-is
    (default_input, default_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_cost_reasoner_tier() {
        let (i, o) = infer_cost("deepseek-reasoner", 0.27, 1.10);
        assert!((i - 1.08).abs() < 0.01, "expected ~1.08, got {i}");
        assert!((o - 4.40).abs() < 0.01, "expected ~4.40, got {o}");
    }

    #[test]
    fn infer_cost_pro_tier() {
        let (i, o) = infer_cost("deepseek-v4-pro", 0.27, 1.10);
        assert!((i - 1.08).abs() < 0.01);
        assert!((o - 4.40).abs() < 0.01);
    }

    #[test]
    fn infer_cost_flash_tier() {
        let (i, o) = infer_cost("deepseek-v4-flash", 0.27, 1.10);
        assert!((i - 0.135).abs() < 0.01, "expected ~0.135, got {i}");
        assert!((o - 0.55).abs() < 0.01, "expected ~0.55, got {o}");
    }

    #[test]
    fn infer_cost_chat_tier() {
        let (i, o) = infer_cost("deepseek-chat", 0.27, 1.10);
        assert!((i - 0.405).abs() < 0.01);
        assert!((o - 1.65).abs() < 0.01);
    }

    #[test]
    fn infer_cost_unknown_tier() {
        let (i, o) = infer_cost("some-unknown-model", 0.5, 2.0);
        assert!((i - 0.5).abs() < 0.01);
        assert!((o - 2.0).abs() < 0.01);
    }

    #[test]
    fn infer_capability_pro() {
        let cap = infer_capability("deepseek-reasoner");
        assert!(cap.reasoning > 0.8);
        assert!(cap.speed_ms > 100);
    }

    #[test]
    fn infer_capability_flash() {
        let cap = infer_capability("deepseek-v4-flash");
        assert!(cap.reasoning < 0.5);
        assert!(cap.speed_ms < 200);
    }

    #[test]
    fn infer_capability_chat() {
        let cap = infer_capability("deepseek-chat");
        assert!(cap.reasoning > 0.5 && cap.reasoning < 0.8);
    }
}
