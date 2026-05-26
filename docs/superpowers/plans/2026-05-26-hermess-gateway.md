# Hermess Gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone LLM API proxy gateway with 3-layer smart routing (SHG detection + complexity classification + prompt decomposition) exposing OpenAI-compatible endpoints.

**Architecture:** New crate `hermess-gateway` in the existing workspace. Reuses `llm` crate adapters for backend model calls. Axum-based HTTP server with OpenAI-format request/response. TOML config with env var interpolation.

**Tech Stack:** Rust, axum 0.8, tokio, serde, toml, reqwest (via llm crate), clap

---

## File Map

```
crates/hermess-gateway/
├── Cargo.toml                        # dep: llm, axum, tokio, serde, toml, clap
├── src/
│   ├── main.rs                       # CLI entry, config load, server start
│   ├── lib.rs                        # pub mod declarations
│   ├── models.rs                     # RouteMode, ModelEntry, RouteTarget, Classification + OpenAI request/response types
│   ├── config.rs                     # GatewayConfig deserialization + env var interpolation
│   ├── registry.rs                   # ModelRegistry — add/lookup/filter by tags/cost/capability
│   ├── shg.rs                        # ShgDetector — short-hard-guard pattern matching
│   ├── classifier.rs                 # Complexity classifier — calls lightweight LLM with timeout
│   ├── strategy.rs                   # RouteStrategy — three mode implementations
│   ├── decision.rs                   # RoutingDecision — ties SHG + classifier + strategy
│   ├── decomposer.rs                 # PromptDecomposer — splits prompt via LLM
│   ├── merger.rs                     # ResultMerger — combines multi-model responses
│   ├── distiller.rs                  # ContextDistiller — compresses context to 20%
│   ├── gateway.rs                    # Gateway orchestrator — full pipeline
│   └── server.rs                     # axum HTTP routes + OpenAI format serialization
config/
└── gateway.toml.example              # Default config with comments
```

All existing crates are **unchanged**. Only `src/main.rs` gets a new `Gateway` subcommand (Task 13).

---

### Task 1: Crate scaffold

**Files:**
- Create: `crates/hermess-gateway/Cargo.toml`
- Create: `crates/hermess-gateway/src/lib.rs`
- Create: `crates/hermess-gateway/src/main.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `src/main.rs` (gateway subcommand stub)

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p crates/hermess-gateway/src
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "hermess-gateway"
version = "0.1.0"
edition = "2021"

[dependencies]
llm = { path = "../llm" }
axum = "0.8"
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
toml.workspace = true
clap = { workspace = true, features = ["derive"] }
tracing.workspace = true
tracing-subscriber.workspace = true
anyhow.workspace = true
async-trait.workspace = true
futures.workspace = true
uuid = { workspace = true, features = ["v4"] }
chrono.workspace = true
parking_lot.workspace = true
reqwest.workspace = true
```

- [ ] **Step 3: Write lib.rs**

```rust
pub mod classifier;
pub mod config;
pub mod decision;
pub mod decomposer;
pub mod distiller;
pub mod gateway;
pub mod merger;
pub mod models;
pub mod registry;
pub mod server;
pub mod shg;
pub mod strategy;
```

- [ ] **Step 4: Write main.rs stub**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "hermes-gateway", about = "Hermess LLM Routing Gateway")]
struct Cli {
    #[arg(short, long, default_value = "config/gateway.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let _cli = Cli::parse();
    tracing::info!("Hermess Gateway placeholder — implementation in progress");
    Ok(())
}
```

- [ ] **Step 5: Add to workspace members**

In root `Cargo.toml`, add `"crates/hermess-gateway"` to the `members` list.

- [ ] **Step 6: Verify builds**

Run: `cargo build -p hermess-gateway`
Expected: compiles successfully (only stub).

- [ ] **Step 7: Commit**

```bash
git add crates/hermess-gateway/ Cargo.toml
git commit -m "feat(gateway): scaffold hermess-gateway crate"
```

---

### Task 2: Data models

**Files:**
- Create: `crates/hermess-gateway/src/models.rs`

- [ ] **Step 1: Write models.rs with all data types**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    CostFirst,
    QualityFirst,
    LatencyFirst,
}

impl std::str::FromStr for RouteMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cost-first" | "cost_first" => Ok(Self::CostFirst),
            "quality-first" | "quality_first" => Ok(Self::QualityFirst),
            "latency-first" | "latency_first" => Ok(Self::LatencyFirst),
            other => Err(format!("unknown route mode: {other}")),
        }
    }
}

impl std::fmt::Display for RouteMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CostFirst => write!(f, "cost-first"),
            Self::QualityFirst => write!(f, "quality-first"),
            Self::LatencyFirst => write!(f, "latency-first"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapability {
    pub reasoning: f64,
    pub coding: f64,
    pub creative: f64,
    pub knowledge: f64,
    pub speed_ms: u64,
}

impl Default for ModelCapability {
    fn default() -> Self {
        Self { reasoning: 0.5, coding: 0.5, creative: 0.5, knowledge: 0.5, speed_ms: 1000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
    #[serde(default)]
    pub capability: ModelCapability,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum RouteTarget {
    Single(String),
    Decomposed { critical: String, regular: String },
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub complexity: f64,
    pub is_short_hard: bool,
    pub suggested_tags: Vec<String>,
}

impl Default for Classification {
    fn default() -> Self {
        Self { complexity: 0.5, is_short_hard: false, suggested_tags: vec![] }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub target: RouteTarget,
    pub reasoning: String,
}

// ── OpenAI-compatible request/response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Extra gateway-specific params
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: UsageData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
}

/// Decomposed prompt — critical vs regular parts
#[derive(Debug, Clone)]
pub struct DecomposedPrompt {
    pub critical: String,
    pub regular: String,
}

/// Splitting result — used by gateway to dispatch
#[derive(Debug, Clone)]
pub enum GatewayOutput {
    Single {
        model: String,
        prompt: String,
    },
    Decomposed {
        critical_model: String,
        critical_prompt: String,
        regular_model: String,
        regular_prompt: String,
    },
}
```

- [ ] **Step 2: Write unit tests for RouteMode parsing**

Add to bottom of models.rs:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_route_mode_cost() {
        assert_eq!("cost-first".parse::<RouteMode>().unwrap(), RouteMode::CostFirst);
        assert_eq!("cost_first".parse::<RouteMode>().unwrap(), RouteMode::CostFirst);
    }

    #[test]
    fn parse_route_mode_quality() {
        assert_eq!("quality-first".parse::<RouteMode>().unwrap(), RouteMode::QualityFirst);
    }

    #[test]
    fn parse_route_mode_latency() {
        assert_eq!("latency-first".parse::<RouteMode>().unwrap(), RouteMode::LatencyFirst);
    }

    #[test]
    fn parse_route_mode_invalid() {
        assert!("garbage".parse::<RouteMode>().is_err());
    }

    #[test]
    fn route_mode_display() {
        assert_eq!(RouteMode::CostFirst.to_string(), "cost-first");
        assert_eq!(RouteMode::QualityFirst.to_string(), "quality-first");
        assert_eq!(RouteMode::LatencyFirst.to_string(), "latency-first");
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-gateway/src/models.rs
git commit -m "feat(gateway): add data models and OpenAI-compatible types"
```

---

### Task 3: Config parser with env var interpolation

**Files:**
- Create: `crates/hermess-gateway/src/config.rs`

- [ ] **Step 1: Write config.rs**

```rust
use serde::Deserialize;

use crate::models::{ModelEntry, RouteMode};

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub gateway: GatewaySection,
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
    pub classifier: ClassifierConfig,
    #[serde(default)]
    pub shg: ShgConfig,
    #[serde(default)]
    pub optimizer: OptimizerConfig,
}

fn default_listen() -> String { "0.0.0.0:9090".into() }
fn default_mode() -> RouteMode { RouteMode::CostFirst }

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierConfig {
    #[serde(default = "default_classifier_model")]
    pub model: String,
    #[serde(default = "default_classifier_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_classifier_model() -> String { "qwen-3-turbo".into() }
fn default_classifier_timeout_ms() -> u64 { 50 }

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self { model: default_classifier_model(), timeout_ms: default_classifier_timeout_ms() }
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

fn default_shg_enabled() -> bool { true }
fn default_shg_prompt_len() -> usize { 200 }

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

#[derive(Debug, Clone, Deserialize)]
pub struct OptimizerConfig {
    #[serde(default)]
    pub decompose_enabled: bool,
    #[serde(default)]
    pub distill_enabled: bool,
    #[serde(default = "default_distill_ratio")]
    pub distill_keep_ratio: f64,
}

fn default_distill_ratio() -> f64 { 0.2 }

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self { decompose_enabled: false, distill_enabled: false, distill_keep_ratio: 0.2 }
    }
}

impl GatewayConfig {
    /// Load from TOML file with `${ENV_VAR}` interpolation.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let interpolated = Self::interpolate_env(&raw);
        Ok(toml::from_str(&interpolated)?)
    }

    /// Replace `${VAR_NAME}` patterns with env var values.
    /// `${VAR:default}` provides a fallback value.
    pub fn interpolate_env(raw: &str) -> String {
        let mut result = raw.to_string();
        let re = regex_lite::Regex::new(r"\$\{(\w+)(?::([^}]*))?\}").unwrap();
        // We do a simple manual interpolation to avoid adding a regex dep.
        // For now, use a simple scan approach.
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                let mut var = String::new();
                let mut default = String::new();
                let mut in_default = false;
                while let Some(c) = chars.next() {
                    if c == ':' && !in_default {
                        in_default = true;
                    } else if c == '}' {
                        break;
                    } else if in_default {
                        default.push(c);
                    } else {
                        var.push(c);
                    }
                }
                let val = std::env::var(&var).unwrap_or(default);
                out.push_str(&val);
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
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: all config tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/config.rs
git commit -m "feat(gateway): add config parser with env var interpolation"
```

---

### Task 4: Model registry

**Files:**
- Create: `crates/hermess-gateway/src/registry.rs`

- [ ] **Step 1: Write registry.rs**

```rust
use crate::models::ModelEntry;

/// In-memory model registry keyed by model name.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    models: Vec<ModelEntry>,
}

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
        self.models
            .iter()
            .min_by(|a, b| {
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
        self.models
            .iter()
            .min_by_key(|m| m.capability.speed_ms)
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
        self.models
            .iter()
            .filter(min_cap)
            .min_by(|a, b| {
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
            capability: ModelCapability { reasoning: 0.3, coding: 0.4, creative: 0.3, knowledge: 0.5, speed_ms: 100 },
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
            capability: ModelCapability { reasoning: 0.95, coding: 0.9, creative: 0.8, knowledge: 0.9, speed_ms: 2000 },
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
            capability: ModelCapability { reasoning: 0.7, coding: 0.8, creative: 0.6, knowledge: 0.7, speed_ms: 500 },
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
        let reasoning: Vec<_> = reg.by_tags(&["reasoning".into()]).into_iter().map(|m| &m.name).collect();
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
        // min reasoning 0.8: only smart-model qualifies
        let pick = reg.best_balanced("reasoning", 0.8);
        assert_eq!(pick.unwrap().name, "smart-model");

        // min reasoning 0.5: mid-model cheaper than smart-model
        let pick = reg.best_balanced("reasoning", 0.5);
        assert_eq!(pick.unwrap().name, "mid-model");
    }

    #[test]
    fn best_balanced_no_match_returns_none() {
        let reg = registry();
        assert!(reg.best_balanced("reasoning", 1.0).is_none());
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: 7 new tests pass (total ~13 including previous tasks).

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/registry.rs
git commit -m "feat(gateway): add model registry with tag/cost/capability queries"
```

---

### Task 5: SHG detector

**Files:**
- Create: `crates/hermess-gateway/src/shg.rs`

- [ ] **Step 1: Write shg.rs**

```rust
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
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: 6 new SHG tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/shg.rs
git commit -m "feat(gateway): add SHG short-hard-guard detector"
```

---

### Task 6: Complexity classifier

**Files:**
- Create: `crates/hermess-gateway/src/classifier.rs`

- [ ] **Step 1: Write classifier.rs**

```rust
use std::sync::Arc;
use tokio::time::{timeout, Duration};

use crate::config::ClassifierConfig;
use crate::models::Classification;
use crate::registry::ModelRegistry;

/// Uses a lightweight LLM to classify request complexity.
#[derive(Clone)]
pub struct ComplexityClassifier {
    config: ClassifierConfig,
    registry: Arc<ModelRegistry>,
}

impl ComplexityClassifier {
    pub fn new(config: ClassifierConfig, registry: Arc<ModelRegistry>) -> Self {
        Self { config, registry }
    }

    /// Classify a prompt. Returns default classification on timeout or error.
    pub async fn classify(&self, prompt: &str) -> Classification {
        let classifier_model = match self.registry.get(&self.config.model) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    model = %self.config.model,
                    "Classifier model not found in registry, using default classification"
                );
                return Classification::default();
            }
        };

        let classification_prompt = format!(
            "Classify this request. Respond with ONLY valid JSON, no other text.\n\
             {{\"complexity\": <0.0-1.0>, \"tags\": [\"<tag1>\", \"<tag2>\"]}}\n\
             complexity: 0.0=trivial, 1.0=extremely complex reasoning needed.\n\
             tags from: reasoning, coding, creative, knowledge, general.\n\n\
             Request: {prompt}"
        );

        let adapter_result = self.build_adapter(classifier_model);
        let adapter = match adapter_result {
            Some(a) => a,
            None => return Classification::default(),
        };

        let future = adapter.complete(classification_prompt);
        match timeout(Duration::from_millis(self.config.timeout_ms), future).await {
            Ok(Ok(response)) => self.parse_classification(&response),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Classifier LLM call failed, using default");
                Classification::default()
            }
            Err(_elapsed) => {
                tracing::debug!("Classifier timed out after {}ms", self.config.timeout_ms);
                Classification::default()
            }
        }
    }

    fn build_adapter(&self, model: &crate::models::ModelEntry) -> Option<Box<dyn llm::LlmAdapter>> {
        match model.provider.as_str() {
            "openai" | "deepseek" => Some(Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: model.api_key.clone(),
                model: model.name.clone(),
                max_tokens: 128,
                base_url: if model.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    model.base_url.clone()
                },
            }))),
            "anthropic" => Some(Box::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: model.api_key.clone(),
                model: model.name.clone(),
                max_tokens: 128,
            }))),
            other => {
                tracing::warn!(provider = %other, "Unknown classifier provider");
                None
            }
        }
    }

    fn parse_classification(&self, raw: &str) -> Classification {
        let trimmed = raw.trim();
        // Trim possible markdown code fences or stray tokens before JSON
        let json_str = trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(val) => {
                let complexity = val["complexity"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                let suggested_tags: Vec<String> = val["tags"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                Classification {
                    complexity,
                    is_short_hard: false, // SHG is checked separately
                    suggested_tags,
                }
            }
            Err(_) => {
                tracing::debug!(raw = %trimmed, "Classifier returned non-JSON, using default");
                Classification::default()
            }
        }
    }

    pub fn timeout_ms(&self) -> u64 {
        self.config.timeout_ms
    }
}
```

- [ ] **Step 2: Run build check**

```bash
cargo build -p hermess-gateway
```
Expected: compiles (no tests yet — classifier requires a live LLM; integration will mock).

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/classifier.rs
git commit -m "feat(gateway): add complexity classifier with timeout fallback"
```

---

### Task 7: Route strategy

**Files:**
- Create: `crates/hermess-gateway/src/strategy.rs`

- [ ] **Step 1: Write strategy.rs**

```rust
use crate::models::{Classification, ModelEntry, RouteMode, RouteTarget};

/// Three routing strategies.
pub struct RouteStrategy;

impl RouteStrategy {
    /// Pick a model based on mode, classification, and available models.
    pub fn decide(
        mode: &RouteMode,
        classification: &Classification,
        models: &[ModelEntry],
    ) -> Option<String> {
        if models.is_empty() {
            return None;
        }
        match mode {
            RouteMode::CostFirst => Self::cost_first(classification, models),
            RouteMode::QualityFirst => Self::quality_first(classification, models),
            RouteMode::LatencyFirst => Self::latency_first(classification, models),
        }
    }

    fn cost_first(classification: &Classification, models: &[ModelEntry]) -> Option<String> {
        let min_cap = Self::min_capability_from_complexity(classification.complexity);
        models
            .iter()
            .filter(|m| m.capability.reasoning >= min_cap)
            .min_by(|a, b| {
                (a.cost_per_1m_input + a.cost_per_1m_output)
                    .partial_cmp(&(b.cost_per_1m_input + b.cost_per_1m_output))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|m| m.name.clone())
    }

    fn quality_first(classification: &Classification, models: &[ModelEntry]) -> Option<String> {
        // Pick highest reasoning score
        models
            .iter()
            .max_by(|a, b| {
                a.capability.reasoning
                    .partial_cmp(&b.capability.reasoning)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|m| m.name.clone())
    }

    fn latency_first(classification: &Classification, models: &[ModelEntry]) -> Option<String> {
        let min_cap = Self::min_capability_from_complexity(classification.complexity);
        models
            .iter()
            .filter(|m| m.capability.reasoning >= min_cap)
            .min_by_key(|m| m.capability.speed_ms)
            .map(|m| m.name.clone())
    }

    /// Map complexity score to a minimum capability bar.
    /// High complexity → higher bar, low complex → relaxed bar.
    fn min_capability_from_complexity(complexity: f64) -> f64 {
        if complexity > 0.8 {
            0.7
        } else if complexity > 0.5 {
            0.4
        } else {
            0.1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelCapability;

    fn models() -> Vec<ModelEntry> {
        vec![
            ModelEntry {
                name: "cheap".into(), provider: "openai".into(),
                base_url: String::new(), api_key: "k".into(),
                cost_per_1m_input: 0.3, cost_per_1m_output: 0.6,
                capability: ModelCapability { reasoning: 0.4, coding: 0.5, creative: 0.3, knowledge: 0.5, speed_ms: 50 },
                tags: vec!["fast".into()],
            },
            ModelEntry {
                name: "mid".into(), provider: "openai".into(),
                base_url: String::new(), api_key: "k".into(),
                cost_per_1m_input: 1.0, cost_per_1m_output: 4.0,
                capability: ModelCapability { reasoning: 0.7, coding: 0.8, creative: 0.6, knowledge: 0.7, speed_ms: 200 },
                tags: vec!["general".into()],
            },
            ModelEntry {
                name: "smart".into(), provider: "anthropic".into(),
                base_url: String::new(), api_key: "k".into(),
                cost_per_1m_input: 15.0, cost_per_1m_output: 75.0,
                capability: ModelCapability { reasoning: 0.95, coding: 0.9, creative: 0.8, knowledge: 0.9, speed_ms: 2000 },
                tags: vec!["reasoning".into()],
            },
        ]
    }

    #[test]
    fn cost_first_low_complexity_picks_cheapest() {
        let cls = Classification { complexity: 0.2, is_short_hard: false, suggested_tags: vec![] };
        let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models());
        assert_eq!(pick, Some("cheap".into()));
    }

    #[test]
    fn cost_first_high_complexity_picks_balanced() {
        // At high complexity (0.9) min cap = 0.7, cheap (0.4) is excluded
        let cls = Classification { complexity: 0.9, is_short_hard: false, suggested_tags: vec![] };
        let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models());
        assert_eq!(pick, Some("mid".into())); // mid meets 0.7 bar and is cheaper than smart
    }

    #[test]
    fn quality_first_always_picks_smartest() {
        let cls = Classification { complexity: 0.1, is_short_hard: false, suggested_tags: vec![] };
        let pick = RouteStrategy::decide(&RouteMode::QualityFirst, &cls, &models());
        assert_eq!(pick, Some("smart".into()));
    }

    #[test]
    fn latency_first_low_complexity_picks_fastest() {
        let cls = Classification { complexity: 0.2, is_short_hard: false, suggested_tags: vec![] };
        let pick = RouteStrategy::decide(&RouteMode::LatencyFirst, &cls, &models());
        assert_eq!(pick, Some("cheap".into())); // fastest is 50ms, and 0.2 complexity → 0.1 bar
    }

    #[test]
    fn latency_first_high_complexity_excludes_slow_but_cheap() {
        // At high complexity cheap is excluded (reasoning 0.4 < 0.7 bar), but mid is faster (200ms) than smart (2000ms)
        let cls = Classification { complexity: 0.9, is_short_hard: false, suggested_tags: vec![] };
        let pick = RouteStrategy::decide(&RouteMode::LatencyFirst, &cls, &models());
        assert_eq!(pick, Some("mid".into()));
    }

    #[test]
    fn empty_models_returns_none() {
        let cls = Classification::default();
        assert_eq!(RouteStrategy::decide(&RouteMode::CostFirst, &cls, &[]), None);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: 6 new strategy tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/strategy.rs
git commit -m "feat(gateway): add three-mode route strategy (cost/quality/latency)"
```

---

### Task 8: Routing decision engine

**Files:**
- Create: `crates/hermess-gateway/src/decision.rs`

- [ ] **Step 1: Write decision.rs**

```rust
use std::sync::Arc;

use crate::classifier::ComplexityClassifier;
use crate::config::OptimizerConfig;
use crate::models::{Classification, RouteMode, RouteTarget, RoutingDecision};
use crate::registry::ModelRegistry;
use crate::shg::ShgDetector;
use crate::strategy::RouteStrategy;

/// Ties SHG → Classifier → Strategy → Decision into a single pipeline.
pub struct DecisionEngine {
    shg: ShgDetector,
    classifier: ComplexityClassifier,
    optimizer_config: OptimizerConfig,
    registry: Arc<ModelRegistry>,
}

impl DecisionEngine {
    pub fn new(
        shg: ShgDetector,
        classifier: ComplexityClassifier,
        optimizer_config: OptimizerConfig,
        registry: Arc<ModelRegistry>,
    ) -> Self {
        Self { shg, classifier, optimizer_config, registry }
    }

    /// Full routing pipeline: SHG → classify → strategy → decision.
    /// If the optimizer is enabled and complexity is moderate, may return
    /// a Decomposed target; otherwise returns Single.
    pub async fn decide(&self, prompt: &str, mode: &RouteMode) -> RoutingDecision {
        // 1. SHG check (<1ms)
        if let Some(force_model) = self.shg.check(prompt) {
            return RoutingDecision {
                target: RouteTarget::Single(force_model),
                reasoning: "SHG: short-hard request, bypassing classifier".into(),
            };
        }

        // 2. Classify (<50ms, with timeout fallback)
        let classification = self.classifier.classify(prompt).await;

        // 3. Strategy decision
        let models = self.registry.all().to_vec();
        let model = RouteStrategy::decide(mode, &classification, &models)
            .unwrap_or_else(|| {
                tracing::warn!("No model matched strategy, using first available");
                models.first().map(|m| m.name.clone()).unwrap_or_default()
            });

        // 4. If optimizer is enabled and complexity is mid-range, decompose
        if self.optimizer_config.decompose_enabled
            && classification.complexity > 0.3
            && classification.complexity < 0.9
        {
            let small = RouteStrategy::decide(
                &RouteMode::CostFirst,
                &Classification { complexity: 0.2, ..Classification::default() },
                &models,
            );
            if let Some(small_model) = small {
                if small_model != model {
                    return RoutingDecision {
                        target: RouteTarget::Decomposed {
                            critical: model,
                            regular: small_model,
                        },
                        reasoning: format!(
                            "Optimizer: complexity={:.2}, decomposed critical→large, regular→small",
                            classification.complexity
                        ),
                    };
                }
            }
        }

        RoutingDecision {
            target: RouteTarget::Single(model),
            reasoning: format!(
                "complexity={:.2}, mode={mode}",
                classification.complexity
            ),
        }
    }
}
```

- [ ] **Step 2: Run build check**

```bash
cargo build -p hermess-gateway
```
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/decision.rs
git commit -m "feat(gateway): add routing decision engine (SHG→classify→strategy)"
```

---

### Task 9: Prompt decomposer

**Files:**
- Create: `crates/hermess-gateway/src/decomposer.rs`

- [ ] **Step 1: Write decomposer.rs**

```rust
use crate::models::DecomposedPrompt;
use crate::registry::ModelRegistry;

/// Splits a prompt into critical and regular parts using a lightweight LLM.
pub struct PromptDecomposer {
    model_name: String,
    registry: ModelRegistry,
}

impl PromptDecomposer {
    pub fn new(model_name: String, registry: ModelRegistry) -> Self {
        Self { model_name, registry }
    }

    /// Attempt to split the prompt. Falls back to a simple heuristic
    /// (entire prompt → critical, empty regular) on any error.
    pub async fn decompose(&self, prompt: &str) -> DecomposedPrompt {
        let model = match self.registry.get(&self.model_name) {
            Some(m) => m,
            None => {
                tracing::warn!("Decomposer model not found, using fallback");
                return self.fallback(prompt);
            }
        };

        let split_prompt = format!(
            "Split the following request into two parts:\n\
             - CRITICAL: the core task requiring deep reasoning or precise knowledge\n\
             - REGULAR: routine context, formatting, or simple follow-up actions\n\n\
             Respond with ONLY valid JSON: {{\"critical\": \"...\", \"regular\": \"...\"}}\n\n\
             Request: {prompt}"
        );

        let adapter: Box<dyn llm::LlmAdapter> = match model.provider.as_str() {
            "openai" | "deepseek" => Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: model.api_key.clone(),
                model: model.name.clone(),
                max_tokens: 256,
                base_url: if model.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    model.base_url.clone()
                },
            })),
            _ => return self.fallback(prompt),
        };

        match adapter.complete(split_prompt).await {
            Ok(response) => {
                let trimmed = response.trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(val) => DecomposedPrompt {
                        critical: val["critical"].as_str().unwrap_or(prompt).to_string(),
                        regular: val["regular"].as_str().unwrap_or("").to_string(),
                    },
                    Err(_) => self.fallback(prompt),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Decomposer LLM call failed, using fallback");
                self.fallback(prompt)
            }
        }
    }

    fn fallback(&self, prompt: &str) -> DecomposedPrompt {
        DecomposedPrompt {
            critical: prompt.to_string(),
            regular: String::new(),
        }
    }
}
```

- [ ] **Step 2: Run build check**

```bash
cargo build -p hermess-gateway
```
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/decomposer.rs
git commit -m "feat(gateway): add prompt decomposer with LLM-based splitting"
```

---

### Task 10: Result merger + distiller

**Files:**
- Create: `crates/hermess-gateway/src/merger.rs`
- Create: `crates/hermess-gateway/src/distiller.rs`

- [ ] **Step 1: Write merger.rs**

```rust
use crate::models::Registry;

/// Combines responses from critical and regular model calls into one output.
pub struct ResultMerger;

impl ResultMerger {
    /// Merge two responses. Currently a straightforward concatenation
    /// with the critical response leading and regular content following.
    pub fn merge(critical: &str, regular: &str) -> String {
        if regular.is_empty() {
            return critical.to_string();
        }
        if critical.is_empty() {
            return regular.to_string();
        }
        format!("{critical}\n\n{regular}")
    }

    /// Merge with an explicit separator.
    pub fn merge_with_separator(critical: &str, regular: &str, sep: &str) -> String {
        if regular.is_empty() {
            return critical.to_string();
        }
        if critical.is_empty() {
            return regular.to_string();
        }
        format!("{critical}{sep}{regular}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_both_present() {
        let result = ResultMerger::merge("Critical analysis", "Regular suggestions");
        assert_eq!(result, "Critical analysis\n\nRegular suggestions");
    }

    #[test]
    fn merge_empty_regular() {
        let result = ResultMerger::merge("Only critical", "");
        assert_eq!(result, "Only critical");
    }

    #[test]
    fn merge_empty_critical() {
        let result = ResultMerger::merge("", "Only regular");
        assert_eq!(result, "Only regular");
    }

    #[test]
    fn merge_with_separator() {
        let result = ResultMerger::merge_with_separator("A", "B", "\n---\n");
        assert_eq!(result, "A\n---\nB");
    }
}
```

- [ ] **Step 2: Write distiller.rs**

```rust
/// Compresses context to retain a configurable fraction of core information.
pub struct ContextDistiller {
    keep_ratio: f64,
}

impl ContextDistiller {
    pub fn new(keep_ratio: f64) -> Self {
        Self { keep_ratio: keep_ratio.clamp(0.0, 1.0) }
    }

    /// Distill a conversation history into a condensed summary prompt.
    /// Delegates the actual compression to an LLM; this method returns
    /// the summary prompt that the caller should send to an LLM.
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
    fn distill_small_history_below_threshold() {
        let d = ContextDistiller::new(0.2);
        let history = vec![("Q1".into(), "A1".into())];
        // 2 items * 0.2 = 0.4 → max(1.0, 0) = 1 target, 2-1=1 to summarize → Some
        let prompt = d.distill_prompt(&history);
        assert!(prompt.is_some());
    }

    #[test]
    fn distill_at_keep_ratio_one_returns_none() {
        let d = ContextDistiller::new(1.0);
        let history = vec![("Q1".into(), "A1".into()), ("Q2".into(), "A2".into())];
        // keep_ratio=1.0 → target=2, to_summarize=0 → None
        assert_eq!(d.distill_prompt(&history), None);
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p hermess-gateway
```
Expected: merger + distiller tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-gateway/src/merger.rs crates/hermess-gateway/src/distiller.rs
git commit -m "feat(gateway): add result merger and context distiller"
```

---

### Task 11: Gateway orchestrator

**Files:**
- Create: `crates/hermess-gateway/src/gateway.rs`

- [ ] **Step 1: Write gateway.rs**

```rust
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::classifier::ComplexityClassifier;
use crate::config::GatewayConfig;
use crate::decision::DecisionEngine;
use crate::decomposer::PromptDecomposer;
use crate::distiller::ContextDistiller;
use crate::merger::ResultMerger;
use crate::models::{ChatCompletionResponse, ChatMessage, Classification, GatewayOutput, RouteMode, UsageData};
use crate::registry::ModelRegistry;
use crate::shg::ShgDetector;

/// Core gateway orchestrator. Owns all layers and exposes a single `route` method.
pub struct Gateway {
    pub config: GatewayConfig,
    registry: Arc<ModelRegistry>,
    decision_engine: DecisionEngine,
    decomposer: Option<PromptDecomposer>,
    distiller: Option<ContextDistiller>,
    /// Per-session history for distillation (simple memory, cleared per session).
    session_history: Mutex<Vec<(String, String)>>,
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        let registry = Arc::new(ModelRegistry::from_entries(config.gateway.models.clone()));

        let shg = ShgDetector::new(&config.gateway.shg);
        let classifier = ComplexityClassifier::new(
            config.gateway.classifier.clone(),
            Arc::clone(&registry),
        );

        let decision_engine = DecisionEngine::new(
            shg,
            classifier,
            config.gateway.optimizer.clone(),
            Arc::clone(&registry),
        );

        let decomposer = if config.gateway.optimizer.decompose_enabled {
            Some(PromptDecomposer::new(
                config.gateway.classifier.model.clone(),
                (*registry).clone(),
            ))
        } else {
            None
        };

        let distiller = if config.gateway.optimizer.distill_enabled {
            Some(ContextDistiller::new(config.gateway.optimizer.distill_keep_ratio))
        } else {
            None
        };

        Self {
            config,
            registry,
            decision_engine,
            decomposer,
            distiller,
            session_history: Mutex::new(Vec::new()),
        }
    }

    /// Route a chat completion request. Returns the output plan (which models
    /// to call with which prompts). The caller handles the actual LLM calls.
    pub async fn route(
        &self,
        prompt: &str,
        mode: Option<RouteMode>,
    ) -> GatewayOutput {
        let mode = mode.unwrap_or_else(|| self.config.gateway.default_mode.clone());
        let decision = self.decision_engine.decide(prompt, &mode).await;

        match decision.target {
            crate::models::RouteTarget::Single(model) => {
                GatewayOutput::Single {
                    model,
                    prompt: prompt.to_string(),
                }
            }
            crate::models::RouteTarget::Decomposed { critical, regular } => {
                let decomposed = if let Some(ref dec) = self.decomposer {
                    dec.decompose(prompt).await
                } else {
                    // No decomposer configured; send full prompt to critical model only
                    crate::models::DecomposedPrompt {
                        critical: prompt.to_string(),
                        regular: String::new(),
                    }
                };
                GatewayOutput::Decomposed {
                    critical_model: critical,
                    critical_prompt: decomposed.critical,
                    regular_model: regular,
                    regular_prompt: decomposed.regular,
                }
            }
        }
    }

    /// Extract the concatenated user prompt from OpenAI messages.
    pub fn extract_prompt(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Build an LLM adapter from a registry entry.
    pub fn build_adapter(entry: &crate::models::ModelEntry) -> Option<Box<dyn llm::LlmAdapter>> {
        match entry.provider.as_str() {
            "openai" | "deepseek" => Some(Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: entry.api_key.clone(),
                model: entry.name.clone(),
                max_tokens: 4096,
                base_url: if entry.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    entry.base_url.clone()
                },
            }))),
            "anthropic" => Some(Box::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: entry.api_key.clone(),
                model: entry.name.clone(),
                max_tokens: 4096,
            }))),
            _ => {
                tracing::warn!(provider = %entry.provider, "Unknown provider");
                None
            }
        }
    }

    /// Look up a model by name.
    pub fn lookup_model(&self, name: &str) -> Option<crate::models::ModelEntry> {
        self.registry.get(name).cloned()
    }

    /// Return all registered models (for GET /v1/models).
    pub fn list_models(&self) -> &[crate::models::ModelEntry] {
        self.registry.all()
    }

    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }
}
```

- [ ] **Step 2: Run build check**

```bash
cargo build -p hermess-gateway
```
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-gateway/src/gateway.rs
git commit -m "feat(gateway): add gateway orchestrator tying all layers together"
```

---

### Task 12: HTTP server with OpenAI-compatible API

**Files:**
- Create: `crates/hermess-gateway/src/server.rs`
- Modify: `crates/hermess-gateway/src/main.rs`

- [ ] **Step 1: Write server.rs**

```rust
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Json,
    },
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::gateway::Gateway;
use crate::models::{
    ChatCompletionRequest, ChatCompletionResponse, ChatChoice, ChatMessage,
    ErrorDetail, ErrorResponse, GatewayOutput, ModelInfo, ModelListResponse, UsageData,
};

pub struct AppState {
    pub gateway: Gateway,
}

pub fn build_router(gateway: Gateway) -> Router {
    let state = Arc::new(AppState { gateway });
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        .with_state(state)
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn models_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let models: Vec<ModelInfo> = state
        .gateway
        .list_models()
        .iter()
        .map(|m| ModelInfo {
            id: m.name.clone(),
            object: "model".into(),
            created: chrono::Utc::now().timestamp(),
            owned_by: m.provider.clone(),
        })
        .collect();

    Json(ModelListResponse {
        object: "list".into(),
        data: models,
    })
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let mode = req
        .mode
        .as_deref()
        .and_then(|m| m.parse().ok());

    let prompt = Gateway::extract_prompt(&req.messages);

    // If model is not "auto", bypass routing and call directly
    if req.model != "auto" {
        match state.gateway.lookup_model(&req.model) {
            Some(entry) => {
                let adapter = Gateway::build_adapter(&entry);
                match adapter {
                    Some(a) => {
                        if req.stream {
                            return handle_stream(a, prompt).await;
                        }
                        return handle_non_stream(a, prompt, &req.model).await;
                    }
                    None => {
                        return Err(api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "provider_error",
                            format!("Cannot build adapter for provider: {}", entry.provider),
                        ));
                    }
                }
            }
            None => {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_model",
                    format!(
                        "Model '{}' not found. Use 'auto' for routing or one of the registered models.",
                        req.model
                    ),
                ));
            }
        }
    }

    // Route via gateway pipeline
    let output = state.gateway.route(&prompt, mode).await;

    match output {
        GatewayOutput::Single { model, prompt } => {
            let entry = match state.gateway.lookup_model(&model) {
                Some(e) => e,
                None => {
                    return Err(api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "routing_error",
                        format!("Routed model '{model}' not found in registry"),
                    ));
                }
            };
            let adapter = match Gateway::build_adapter(&entry) {
                Some(a) => a,
                None => {
                    return Err(api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "provider_error",
                        format!("Cannot build adapter for provider: {}", entry.provider),
                    ));
                }
            };
            if req.stream {
                return handle_stream(adapter, prompt).await;
            }
            handle_non_stream(adapter, prompt, &model).await
        }
        GatewayOutput::Decomposed {
            critical_model,
            critical_prompt,
            regular_model,
            regular_prompt,
        } => {
            // Run both in parallel
            let crit_entry = state.gateway.lookup_model(&critical_model);
            let reg_entry = state.gateway.lookup_model(&regular_model);

            let (crit_result, reg_result) = tokio::join!(
                async {
                    if let Some(ref entry) = crit_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            return a.complete(critical_prompt).await.ok()
                        }
                    }
                    None
                },
                async {
                    if let Some(ref entry) = reg_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            return a.complete(regular_prompt).await.ok()
                        }
                    }
                    None
                },
            );

            let merged = crate::merger::ResultMerger::merge(
                crit_result.as_deref().unwrap_or(""),
                reg_result.as_deref().unwrap_or(""),
            );

            Ok(Json(ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".into(),
                created: chrono::Utc::now().timestamp(),
                model: format!("{critical_model}+{regular_model}"),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage { role: "assistant".into(), content: merged },
                    finish_reason: "stop".into(),
                }],
                usage: UsageData { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
            }))
        }
    }
}

async fn handle_non_stream(
    adapter: Box<dyn llm::LlmAdapter>,
    prompt: String,
    model: &str,
) -> Result<Json<ChatCompletionResponse>, (StatusCode, Json<ErrorResponse>)> {
    match adapter.complete(prompt).await {
        Ok(text) => {
            let usage = adapter.last_usage();
            Ok(Json(ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".into(),
                created: chrono::Utc::now().timestamp(),
                model: model.to_string(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage { role: "assistant".into(), content: text },
                    finish_reason: "stop".into(),
                }],
                usage: UsageData {
                    prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                    completion_tokens: usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                    total_tokens: usage.as_ref().map(|u| u.total_tokens).unwrap_or(0),
                },
            }))
        }
        Err(e) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("Backend model error: {e:#}"),
        )),
    }
}

async fn handle_stream(
    adapter: Box<dyn llm::LlmAdapter>,
    prompt: String,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, Json<ErrorResponse>)> {
    match adapter.complete_stream(prompt).await {
        Ok(stream) => {
            let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
            let sse_stream = stream.map(move |chunk| {
                match chunk {
                    Ok(token) => {
                        let data = serde_json::json!({
                            "id": id,
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "choices": [{
                                "index": 0,
                                "delta": {"content": token},
                                "finish_reason": null
                            }]
                        });
                        Ok(Event::default().data(serde_json::to_string(&data).unwrap_or_default()))
                    }
                    Err(e) => {
                        let data = serde_json::json!({
                            "error": {"message": format!("{e:#}"), "type": "stream_error"}
                        });
                        Ok(Event::default().data(serde_json::to_string(&data).unwrap_or_default()))
                    }
                }
            });
            Ok(Sse::new(sse_stream))
        }
        Err(e) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("Backend stream error: {e:#}"),
        )),
    }
}

fn api_error(
    status: StatusCode,
    error_type: &str,
    message: String,
) -> (StatusCode, Json<ErrorResponse>) {
    tracing::warn!(%error_type, %message, "API error");
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                message,
                error_type: error_type.into(),
                code: None,
            },
        }),
    )
}

async fn embeddings_handler(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Transparent embedding pass-through to default model — stub
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: ErrorDetail {
                message: "Embeddings endpoint not yet implemented".into(),
                error_type: "not_implemented".into(),
                code: None,
            },
        }),
    )
}
```

- [ ] **Step 2: Update main.rs**

Replace the stub with:

```rust
use clap::Parser;

mod classifier;
mod config;
mod decision;
mod decomposer;
mod distiller;
mod gateway;
mod merger;
mod models;
mod registry;
mod server;
mod shg;
mod strategy;

#[derive(Parser)]
#[command(name = "hermes-gateway", about = "Hermess LLM Routing Gateway")]
struct Cli {
    #[arg(short, long, default_value = "config/gateway.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = config::GatewayConfig::from_file(&cli.config)?;

    let listen_addr = cfg.gateway.listen.clone();
    let gateway = gateway::Gateway::new(cfg);

    tracing::info!(addr = %listen_addr, "Hermess Gateway starting");
    tracing::info!(models = gateway.list_models().len(), "Registered models");

    let app = server::build_router(gateway);
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

- [ ] **Step 3: Run build check**

```bash
cargo build -p hermess-gateway
```
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-gateway/src/server.rs crates/hermess-gateway/src/main.rs
git commit -m "feat(gateway): add OpenAI-compatible HTTP server with streaming"
```

---

### Task 13: Configuration example + wire to hermes CLI

**Files:**
- Create: `config/gateway.toml.example`
- Modify: `src/main.rs`

- [ ] **Step 1: Write config/gateway.toml.example**

```toml
# Hermess Gateway configuration
[gateway]
listen = "0.0.0.0:9090"
api_key = "sk-gateway-local"
default_mode = "cost-first"

# ── Default models ──
[[gateway.models]]
name = "qwen-3-turbo"
provider = "openai"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
api_key = "${QWEN_API_KEY}"
cost_per_1m_input = 0.3
cost_per_1m_output = 0.6
capability = { reasoning = 0.3, coding = 0.5, creative = 0.4, knowledge = 0.6, speed_ms = 50 }
tags = ["fast", "classifier"]

[[gateway.models]]
name = "deepseek-v4"
provider = "openai"
base_url = "https://api.deepseek.com/v1"
api_key = "${DEEPSEEK_API_KEY}"
cost_per_1m_input = 0.5
cost_per_1m_output = 2.0
capability = { reasoning = 0.6, coding = 0.8, creative = 0.5, knowledge = 0.7, speed_ms = 200 }
tags = ["general", "coding"]

[[gateway.models]]
name = "claude-opus-4-6"
provider = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
cost_per_1m_input = 15.0
cost_per_1m_output = 75.0
capability = { reasoning = 0.95, coding = 0.9, creative = 0.85, knowledge = 0.9, speed_ms = 2000 }
tags = ["reasoning", "complex"]

# ── Classifier ──
[gateway.classifier]
model = "qwen-3-turbo"
timeout_ms = 50

# ── SHG ──
[gateway.shg]
enabled = true
prompt_len_threshold = 200
hard_patterns = [
    "时间复杂度",
    "formal proof",
    "prove",
    "security audit",
    "cryptograph",
    "distributed consensus",
]
force_model = "claude-opus-4-6"

# ── Token optimizer (optional, disabled by default) ──
[gateway.optimizer]
decompose_enabled = false
distill_enabled = false
distill_keep_ratio = 0.2
```

- [ ] **Step 2: Add Gateway subcommand to hermes CLI**

In `src/main.rs`, add under the existing `Cli` enum (add at bottom of `Cli` struct, before the closing `}`):

```rust
    /// Start the LLM routing gateway
    #[arg(long)]
    gateway: bool,
    /// Gateway config path (only used with --gateway)
    #[arg(long, default_value = "config/gateway.toml")]
    gateway_config: String,
```

Then in `main()`, after `let cli = Cli::parse();` but before loading the existing config, add:

```rust
    if cli.gateway {
        return run_gateway(&cli.gateway_config).await;
    }
```

And add the function at the bottom of the file:

```rust
async fn run_gateway(config_path: &str) -> anyhow::Result<()> {
    let cfg = hermess_gateway::config::GatewayConfig::from_file(config_path)?;
    let listen_addr = cfg.gateway.listen.clone();
    let gateway = hermess_gateway::gateway::Gateway::new(cfg);

    tracing::info!(addr = %listen_addr, "Hermess Gateway starting");
    let app = hermess_gateway::server::build_router(gateway);
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

Add `hermess-gateway` to root `Cargo.toml` dependencies:

```toml
hermess-gateway = { path = "crates/hermess-gateway" }
```

- [ ] **Step 3: Build and verify**

```bash
cargo build
```
Expected: full workspace builds successfully.

```bash
cargo test
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add config/gateway.toml.example src/main.rs Cargo.toml
git commit -m "feat(gateway): add config example and wire to hermes CLI"
```

---

### Task 14: Integration test

**Files:**
- Create: `crates/hermess-gateway/tests/integration.rs`

- [ ] **Step 1: Write integration test**

```rust
use hermess_gateway::models::*;
use hermess_gateway::registry::ModelRegistry;
use hermess_gateway::shg::ShgDetector;
use hermess_gateway::strategy::RouteStrategy;

/// Test the full SHG → strategy decision pipeline without a live LLM.
#[tokio::test]
async fn full_pipeline_shg_triggers() {
    // SHG triggers on "formal proof" → routes directly to smart model
    let shg = ShgDetector {
        enabled: true,
        prompt_len_threshold: 200,
        patterns: vec!["formal proof".into()],
        force_model: Some("claude-opus-4-6".into()),
    };

    let result = shg.check("Give a formal proof of the theorem");
    assert_eq!(result, Some("claude-opus-4-6".into()));
}

#[test]
fn strategy_pipeline_cost_first() {
    let models = vec![
        ModelEntry {
            name: "cheap".into(), provider: "openai".into(),
            base_url: String::new(), api_key: "k".into(),
            cost_per_1m_input: 0.3, cost_per_1m_output: 0.6,
            capability: ModelCapability { reasoning: 0.4, coding: 0.5, creative: 0.3, knowledge: 0.5, speed_ms: 50 },
            tags: vec!["fast".into()],
        },
        ModelEntry {
            name: "smart".into(), provider: "anthropic".into(),
            base_url: String::new(), api_key: "k".into(),
            cost_per_1m_input: 15.0, cost_per_1m_output: 75.0,
            capability: ModelCapability { reasoning: 0.95, coding: 0.9, creative: 0.8, knowledge: 0.9, speed_ms: 2000 },
            tags: vec!["reasoning".into()],
        },
    ];

    // Low complexity → cheapest model
    let cls = Classification { complexity: 0.2, is_short_hard: false, suggested_tags: vec![] };
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models);
    assert_eq!(pick, Some("cheap".into()));

    // High complexity → smart model (cheap fails min cap threshold)
    let cls = Classification { complexity: 0.9, is_short_hard: false, suggested_tags: vec![] };
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models);
    assert_eq!(pick, Some("smart".into()));
}

#[test]
fn registry_capability_lookup() {
    let mut reg = ModelRegistry::new();
    reg.add(ModelEntry {
        name: "m1".into(), provider: "openai".into(),
        base_url: String::new(), api_key: "k".into(),
        cost_per_1m_input: 1.0, cost_per_1m_output: 2.0,
        capability: ModelCapability { reasoning: 0.9, coding: 0.5, creative: 0.5, knowledge: 0.5, speed_ms: 100 },
        tags: vec!["reasoning".into()],
    });
    reg.add(ModelEntry {
        name: "m2".into(), provider: "openai".into(),
        base_url: String::new(), api_key: "k".into(),
        cost_per_1m_input: 0.1, cost_per_1m_output: 0.1,
        capability: ModelCapability { reasoning: 0.2, coding: 0.5, creative: 0.5, knowledge: 0.5, speed_ms: 50 },
        tags: vec!["fast".into()],
    });

    assert_eq!(reg.most_capable("reasoning").unwrap().name, "m1");
    assert_eq!(reg.cheapest().unwrap().name, "m2");
    assert_eq!(reg.fastest().unwrap().name, "m2");
}
```

- [ ] **Step 2: Add [[test]] to Cargo.toml**

Add to `crates/hermess-gateway/Cargo.toml`:

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["full"] }
```

- [ ] **Step 3: Run integration tests**

```bash
cargo test -p hermess-gateway
```
Expected: all unit + integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-gateway/tests/ crates/hermess-gateway/Cargo.toml
git commit -m "test(gateway): add integration tests for routing pipeline"
```

---

### Task 15: Final verification

- [ ] **Step 1: Full workspace build**

```bash
cargo build
```
Expected: all crates compile without errors or warnings.

- [ ] **Step 2: Full test suite**

```bash
cargo test
```
Expected: all tests pass across all crates.

- [ ] **Step 3: Clippy**

```bash
cargo clippy -- -D warnings
```
Expected: no warnings treated as errors.

- [ ] **Step 4: Start gateway smoke test**

```bash
cargo run -- --gateway &
sleep 2
curl -s http://localhost:9090/health
curl -s http://localhost:9090/v1/models | jq .
kill %1
```
Expected: health returns "ok", models returns JSON list.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore(gateway): final verification, all tests and clippy pass"
```
