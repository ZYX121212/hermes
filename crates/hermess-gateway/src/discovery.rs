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
        if id_lower.contains("embed") || id_lower.contains("moderation") || id_lower.contains("tts") {
            continue;
        }
        // Apply name prefix filter if configured
        if let Some(ref prefix) = provider.name_prefix {
            if !id_lower.starts_with(&prefix.to_lowercase()) {
                continue;
            }
        }
        entries.push(ModelEntry {
            name: id.to_string(),
            provider: provider.provider.clone(),
            base_url: provider.base_url.clone(),
            api_key: provider.api_key.clone(),
            cost_per_1m_input: provider.default_cost_input,
            cost_per_1m_output: provider.default_cost_output,
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
        return ModelCapability { reasoning: 0.9, coding: 0.85, creative: 0.8, knowledge: 0.85, speed_ms: 1000 };
    }
    // Flash / turbo / lite models
    if lower.contains("flash") || lower.contains("turbo") || lower.contains("lite") {
        return ModelCapability { reasoning: 0.4, coding: 0.5, creative: 0.45, knowledge: 0.5, speed_ms: 80 };
    }
    // Sonnet / balanced models
    if lower.contains("sonnet") || lower.contains("chat") || lower.contains("v4") {
        return ModelCapability { reasoning: 0.65, coding: 0.8, creative: 0.55, knowledge: 0.7, speed_ms: 200 };
    }
    // Default
    ModelCapability { reasoning: 0.5, coding: 0.6, creative: 0.5, knowledge: 0.6, speed_ms: 300 }
}
