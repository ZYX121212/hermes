// crates/hermess-web/src/lib.rs
use serde::Deserialize;

pub mod feishu;
pub mod server;
pub mod session;

// ── Config types shared across the crate ─────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WebAppConfig {
    pub feishu: feishu_config::FeishuConfig,
    pub server: ServerConfig,
    pub learning_rate: f64,
    pub working_memory_size: usize,
    pub max_concurrency: usize,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub qdrant: QdrantConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub scorer: ScorerConfig,
    #[serde(default)]
    pub api_key: String,
}

impl WebAppConfig {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let interpolated = Self::interpolate_env(&raw);
        Ok(toml::from_str(&interpolated)?)
    }

    fn interpolate_env(raw: &str) -> String {
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next();
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

/// 飞书应用配置
pub mod feishu_config {
    use serde::Deserialize;

    #[derive(Debug, Clone, Deserialize)]
    pub struct FeishuConfig {
        pub app_id: String,
        pub app_secret: String,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8080 }

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub max_tokens: u32,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5-20251001".into(),
            max_tokens: 4096,
            api_key: String::new(),
            base_url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QdrantConfig {
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    #[serde(default = "default_collection")]
    pub collection: String,
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: default_qdrant_url(),
            collection: default_collection(),
            embedding_dim: default_embedding_dim(),
        }
    }
}

fn default_qdrant_url() -> String { "http://localhost:6334".into() }
fn default_collection() -> String { "hermes_memory".into() }
fn default_embedding_dim() -> usize { 1024 }

#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_search_endpoint")]
    pub endpoint: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self { api_key: None, endpoint: default_search_endpoint() }
    }
}

fn default_search_endpoint() -> String {
    "https://api.search.brave.com/res/v1/web/search".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScorerConfig {
    #[serde(default = "default_success_weight")]
    pub success_weight: f64,
    #[serde(default = "default_latency_weight")]
    pub latency_weight: f64,
    #[serde(default = "default_quality_weight")]
    pub quality_weight: f64,
    #[serde(default = "default_latency_target")]
    pub latency_target_ms: u64,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            success_weight: default_success_weight(),
            latency_weight: default_latency_weight(),
            quality_weight: default_quality_weight(),
            latency_target_ms: default_latency_target(),
        }
    }
}

fn default_success_weight() -> f64 { 0.6 }
fn default_latency_weight() -> f64 { 0.2 }
fn default_quality_weight() -> f64 { 0.2 }
fn default_latency_target() -> u64 { 2000 }
