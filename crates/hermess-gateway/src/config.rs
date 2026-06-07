use serde::Deserialize;

use crate::models::{ModelEntry, RouteMode};

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub gateway: GatewaySection,
}

/// A provider to auto-discover models from on startup.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryProvider {
    pub base_url: String,
    pub api_key: String,
    pub provider: String,
    /// Optional model name prefix to filter (e.g., "deepseek" only keeps those starting with "deepseek")
    #[serde(default)]
    pub name_prefix: Option<String>,
    /// Cost defaults for discovered models (per 1M tokens)
    #[serde(default = "default_discovery_cost_input")]
    pub default_cost_input: f64,
    #[serde(default = "default_discovery_cost_output")]
    pub default_cost_output: f64,
    /// Tags to assign to all discovered models from this provider
    #[serde(default)]
    pub default_tags: Vec<String>,
}

fn default_discovery_cost_input() -> f64 {
    0.5
}
fn default_discovery_cost_output() -> f64 {
    2.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewaySection {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_mode")]
    pub default_mode: RouteMode,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    #[serde(default)]
    pub discovery: Vec<DiscoveryProvider>,
    #[serde(default)]
    pub classifier: ClassifierConfig,
    #[serde(default)]
    pub shg: ShgConfig,
    #[serde(default)]
    pub optimizer: OptimizerConfig,
}

fn default_listen() -> String {
    "0.0.0.0:9090".into()
}
fn default_mode() -> RouteMode {
    RouteMode::CostFirst
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierConfig {
    #[serde(default = "default_classifier_model")]
    pub model: String,
    #[serde(default = "default_classifier_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_classifier_model() -> String {
    "qwen-3-turbo".into()
}
fn default_classifier_timeout_ms() -> u64 {
    50
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            model: default_classifier_model(),
            timeout_ms: default_classifier_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShgConfig {
    #[serde(default = "default_shg_enabled")]
    pub enabled: bool,
    #[serde(default = "default_shg_prompt_len")]
    pub prompt_len_threshold: usize,
    #[serde(default)]
    pub hard_patterns: Vec<String>,
    #[serde(default)]
    pub force_model: Option<String>,
}

fn default_shg_enabled() -> bool {
    true
}
fn default_shg_prompt_len() -> usize {
    200
}

impl Default for ShgConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt_len_threshold: 200,
            hard_patterns: vec![],
            force_model: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OptimizerConfig {
    #[serde(default)]
    pub decompose_enabled: bool,
}

impl GatewayConfig {
    /// Load from TOML file with `${ENV_VAR}` interpolation.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let interpolated = Self::interpolate_env(&raw);
        Ok(toml::from_str(&interpolated)?)
    }

    /// Replace `${VAR_NAME}` or `${VAR_NAME:default}` patterns with env var values.
    pub fn interpolate_env(raw: &str) -> String {
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                let mut var = String::new();
                let mut default = String::new();
                let mut in_default = false;
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == ':' && !in_default {
                        in_default = true;
                    } else if c == '}' {
                        found_close = true;
                        break;
                    } else if in_default {
                        default.push(c);
                    } else {
                        var.push(c);
                    }
                }
                if found_close {
                    let val = std::env::var(&var).unwrap_or(default);
                    out.push_str(&val);
                } else {
                    // Missing closing '}', pass through literal text
                    out.push_str("${");
                    out.push_str(&var);
                    if in_default {
                        out.push(':');
                        out.push_str(&default);
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_replaces_var() {
        std::env::set_var("TEST_GW_KEY", "sk-test-123");
        let input = r#"api_key = "${TEST_GW_KEY}""#;
        let result = GatewayConfig::interpolate_env(input);
        assert!(result.contains("sk-test-123"));
        assert!(!result.contains("${"));
    }

    #[test]
    fn interpolate_default_fallback() {
        let input = r#"api_key = "${MISSING_VAR:fallback-key}""#;
        let result = GatewayConfig::interpolate_env(input);
        assert!(result.contains("fallback-key"));
        assert!(!result.contains("${"));
    }

    #[test]
    fn interpolate_no_var_unchanged() {
        let input = "listen = \"0.0.0.0:9090\"";
        let result = GatewayConfig::interpolate_env(input);
        assert_eq!(result, input);
    }

    #[test]
    fn interpolate_multiple_vars() {
        std::env::set_var("A_KEY", "aaa");
        std::env::set_var("B_KEY", "bbb");
        let input = r#"a = "${A_KEY}", b = "${B_KEY}""#;
        let result = GatewayConfig::interpolate_env(input);
        assert!(result.contains("aaa"));
        assert!(result.contains("bbb"));
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[gateway]
listen = "127.0.0.1:8080"
api_key = "sk-test"
"#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.gateway.listen, "127.0.0.1:8080");
        assert_eq!(cfg.gateway.default_mode, RouteMode::CostFirst);
        assert!(cfg.gateway.models.is_empty());
    }

    #[test]
    fn parse_with_models() {
        let toml = r#"
[gateway]
listen = "0.0.0.0:9090"
default_mode = "quality-first"

[[gateway.models]]
name = "deepseek-v4"
provider = "openai"
base_url = "https://api.deepseek.com/v1"
api_key = "sk-ds"
cost_per_1m_input = 0.5
cost_per_1m_output = 2.0
capability = { reasoning = 0.6, coding = 0.8, creative = 0.5, knowledge = 0.7, speed_ms = 200 }
tags = ["general", "coding"]
"#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.gateway.models.len(), 1);
        assert_eq!(cfg.gateway.models[0].name, "deepseek-v4");
        assert_eq!(cfg.gateway.models[0].capability.coding, 0.8);
        assert_eq!(cfg.gateway.models[0].tags, vec!["general", "coding"]);
    }
}
