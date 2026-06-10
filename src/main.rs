// src/main.rs
// Hermes Agent CLI — assembles all subsystems per the arch spec.
use std::str::FromStr;
use std::sync::Arc;

use agent_core::{AgentEvent, HermesAgent};
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use tokio::sync::Mutex;

// ── CLI ──────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "hermes", about = "Small Hermes Agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    // ═══ 以下为默认 agent 模式的参数 ═══
    #[arg(short, long, default_value = "config/default.toml")]
    config: String,
    /// 配置预设: dev, prod 等（自动加载 `config/profiles/{name}.toml`）
    #[arg(short, long)]
    profile: Option<String>,
    #[arg(short, long)]
    task: Option<String>,
    /// Interactive mode: multi-turn conversation, reads tasks from stdin
    #[arg(short, long)]
    interactive: bool,
    /// Launch with terminal user interface (ratatui).
    #[arg(long)]
    tui: bool,

    // ── Config overrides ────────────────────────────────
    /// LLM API key (overrides config file and env vars)
    #[arg(long)]
    api_key: Option<String>,
    /// LLM provider: "anthropic", "openai", "deepseek", or "litellm"
    #[arg(long)]
    provider: Option<String>,
    /// LLM model name
    #[arg(long)]
    model: Option<String>,
    /// LLM max tokens per request
    #[arg(long)]
    max_tokens: Option<u32>,
    /// LLM API base URL (overrides provider default)
    #[arg(long)]
    base_url: Option<String>,
    /// Search API key (Brave Search)
    #[arg(long)]
    search_api_key: Option<String>,
    /// Learning rate for evolution (0.0–1.0)
    #[arg(long)]
    learning_rate: Option<f64>,
    /// Max parallel step execution
    #[arg(long)]
    max_concurrency: Option<usize>,
    /// Danger command policy: "ask", "skip", or "deny"
    #[arg(long)]
    danger_mode: Option<String>,
    /// Max retries per step before trying fallback tools
    #[arg(long)]
    max_step_retries: Option<usize>,
    /// Max replan attempts when all steps fail
    #[arg(long)]
    max_replans: Option<usize>,
    /// Context compression threshold (conversation entries)
    #[arg(long)]
    compress_threshold: Option<usize>,
    /// Fraction of history to keep after compression (0.0–1.0)
    #[arg(long)]
    compress_keep_ratio: Option<f64>,
    /// 退出时保存会话到文件
    #[arg(long)]
    save: Option<String>,
    /// Cron 表达式定时执行（如 "0 */6 * * *" 每6小时）
    #[arg(long)]
    schedule: Option<String>,
    /// 从文件恢复之前的会话
    #[arg(long)]
    resume: Option<String>,
    /// Start the LLM routing gateway
    #[arg(long)]
    gateway: bool,
    /// Gateway config path (only used with --gateway)
    #[arg(long, default_value = "config/gateway.toml")]
    gateway_config: String,
    /// 以 MCP (Model Context Protocol) stdio server 模式运行
    #[arg(long)]
    mcp_server: bool,
    /// 启动 HTTP 服务器模式
    #[arg(long)]
    serve: Option<u16>,
    /// 预加载知识库目录（可重复指定）
    #[arg(long = "knowledge-base")]
    knowledge_base: Vec<String>,

    // ── 飞书平台 ─────────────────────────────────
    /// 飞书应用 App ID
    #[arg(long)]
    feishu_app_id: Option<String>,
    /// 飞书应用 App Secret
    #[arg(long)]
    feishu_app_secret: Option<String>,
    /// 飞书 Bot 自身 open_id（用于过滤 bot 自己的消息）
    #[arg(long)]
    feishu_bot_open_id: Option<String>,

    // ── 企业微信平台 ─────────────────────────────
    /// 企业微信 Corp ID
    #[arg(long)]
    wechat_corp_id: Option<String>,
    /// 企业微信应用 Secret
    #[arg(long)]
    wechat_corp_secret: Option<String>,
    /// 企业微信应用 Agent ID
    #[arg(long)]
    wechat_agent_id: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// 交互式配置向导，逐项设置 LLM、搜索、飞书、企业微信等
    Configure {
        /// 限定配置 section（可重复）：llm, search, finance, feishu, wechat
        #[arg(long, short)]
        section: Vec<String>,
    },
}

// ── Config ───────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct Config {
    learning_rate: f64,
    working_memory_size: usize,
    max_concurrency: usize,
    #[serde(default)]
    llm: LlmConfig,
    #[serde(default)]
    qdrant: QdrantConfig,
    #[serde(default)]
    search: SearchConfig,
    #[serde(default)]
    scorer: ScorerConfig,
    #[serde(default)]
    guard: GuardConfig,
    #[serde(default = "default_max_step_retries")]
    max_step_retries: usize,
    #[serde(default = "default_max_replans")]
    max_replans: usize,
    #[serde(default = "default_compress_threshold")]
    compress_threshold: usize,
    #[serde(default = "default_compress_keep_ratio")]
    compress_keep_ratio: f64,
    #[serde(default = "default_plugin_dirs")]
    plugin_dirs: Vec<String>,
    #[serde(default)]
    feishu: Option<FeishuPlatformConfig>,
    #[serde(default)]
    wechat: Option<WechatPlatformConfig>,
}

impl Config {
    fn from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let interpolated = Self::interpolate_env(&raw);
        Ok(toml::from_str(&interpolated)?)
    }

    /// Replace `${VAR_NAME}` or `${VAR_NAME:default}` with env var values.
    fn interpolate_env(raw: &str) -> String {
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next();
                let mut var = String::new();
                let mut default = String::new();
                let mut in_default = false;
                for c in chars.by_ref() {
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

#[derive(Debug, Clone, Deserialize)]
struct LlmConfig {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    max_tokens: u32,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    base_url: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            max_tokens: 4096,
            api_key: String::new(),
            base_url: "https://api.deepseek.com/v1".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct QdrantConfig {
    #[serde(default = "default_qdrant_url")]
    url: String,
    #[serde(default = "default_collection")]
    collection: String,
    #[serde(default = "default_embedding_dim")]
    embedding_dim: usize,
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

fn default_qdrant_url() -> String {
    "http://localhost:6334".into()
}
fn default_collection() -> String {
    "hermes_memory".into()
}
fn default_embedding_dim() -> usize {
    1024
}

#[derive(Debug, Clone, Deserialize)]
struct SearchConfig {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default = "default_search_endpoint")]
    endpoint: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            endpoint: default_search_endpoint(),
        }
    }
}

fn default_search_endpoint() -> String {
    "https://api.search.brave.com/res/v1/web/search".into()
}

#[derive(Debug, Clone, Deserialize)]
struct GuardConfig {
    #[serde(default = "default_danger_mode")]
    danger_mode: String,
    #[serde(default)]
    dangerous_patterns: Vec<String>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            danger_mode: default_danger_mode(),
            dangerous_patterns: vec![],
        }
    }
}

fn default_max_step_retries() -> usize {
    3
}
fn default_max_replans() -> usize {
    1
}
fn default_compress_threshold() -> usize {
    20
}
fn default_compress_keep_ratio() -> f64 {
    0.5
}
fn default_plugin_dirs() -> Vec<String> {
    vec!["plugins".into()]
}

// ── 平台配置 ──────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct FeishuPlatformConfig {
    app_id: String,
    app_secret: String,
    #[serde(default)]
    bot_open_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatPlatformConfig {
    corp_id: String,
    corp_secret: String,
    agent_id: u64,
}

/// 按优先级合并飞书配置：CLI > env > settings.json > config.toml
fn build_feishu_config(
    cli: &Cli,
    config: &Config,
    settings: &tui::settings_store::UserSettings,
) -> hermess_platform::adapters::FeishuConfig {
    let app_id = cli
        .feishu_app_id
        .clone()
        .or_else(|| {
            std::env::var("FEISHU_APP_ID")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| {
            if !settings.feishu_app_id.is_empty() {
                settings.feishu_app_id.clone()
            } else if let Some(ref fc) = config.feishu {
                fc.app_id.clone()
            } else {
                String::new()
            }
        });

    let app_secret = cli
        .feishu_app_secret
        .clone()
        .or_else(|| {
            std::env::var("FEISHU_APP_SECRET")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| {
            if !settings.feishu_app_secret.is_empty() {
                settings.feishu_app_secret.clone()
            } else if let Some(ref fc) = config.feishu {
                fc.app_secret.clone()
            } else {
                String::new()
            }
        });

    let bot_open_id = cli
        .feishu_bot_open_id
        .clone()
        .or_else(|| {
            std::env::var("FEISHU_BOT_OPEN_ID")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            if !settings.feishu_bot_open_id.is_empty() {
                Some(settings.feishu_bot_open_id.clone())
            } else {
                config.feishu.as_ref().and_then(|fc| fc.bot_open_id.clone())
            }
        });

    hermess_platform::adapters::FeishuConfig {
        app_id,
        app_secret,
        bot_open_id,
        ..Default::default()
    }
}

/// 按优先级合并企业微信配置：CLI > env > settings.json > config.toml
fn build_wechat_config(
    cli: &Cli,
    config: &Config,
    settings: &tui::settings_store::UserSettings,
) -> hermess_platform::adapters::WechatConfig {
    let corp_id = cli
        .wechat_corp_id
        .clone()
        .or_else(|| {
            std::env::var("WECHAT_CORP_ID")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| {
            if !settings.wechat_corp_id.is_empty() {
                settings.wechat_corp_id.clone()
            } else if let Some(ref wc) = config.wechat {
                wc.corp_id.clone()
            } else {
                String::new()
            }
        });

    let corp_secret = cli
        .wechat_corp_secret
        .clone()
        .or_else(|| {
            std::env::var("WECHAT_CORP_SECRET")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| {
            if !settings.wechat_corp_secret.is_empty() {
                settings.wechat_corp_secret.clone()
            } else if let Some(ref wc) = config.wechat {
                wc.corp_secret.clone()
            } else {
                String::new()
            }
        });

    let agent_id = cli
        .wechat_agent_id
        .or_else(|| {
            std::env::var("WECHAT_AGENT_ID")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or_else(|| {
            if !settings.wechat_agent_id.is_empty() {
                settings.wechat_agent_id.parse().unwrap_or(0)
            } else if let Some(ref wc) = config.wechat {
                wc.agent_id
            } else {
                0
            }
        });

    hermess_platform::adapters::WechatConfig {
        corp_id,
        corp_secret,
        agent_id,
        ..Default::default()
    }
}
fn default_danger_mode() -> String {
    "ask".into()
}

#[derive(Debug, Clone, Deserialize)]
struct ScorerConfig {
    #[serde(default = "default_success_weight")]
    success_weight: f64,
    #[serde(default = "default_latency_weight")]
    latency_weight: f64,
    #[serde(default = "default_quality_weight")]
    quality_weight: f64,
    #[serde(default = "default_latency_target")]
    latency_target_ms: u64,
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

fn default_success_weight() -> f64 {
    0.6
}
fn default_latency_weight() -> f64 {
    0.2
}
fn default_quality_weight() -> f64 {
    0.2
}
fn default_latency_target() -> u64 {
    2000
}

// ── Configure wizard ────────────────────────────────────────

fn section_enabled(sections: &[String], name: &str) -> bool {
    sections.is_empty() || sections.iter().any(|s| s == name)
}

/// 从 LiteLLM /v1/models 获取模型列表，按 provider 过滤。
/// 返回 (model_id, provider_name) 列表。
fn fetch_litellm_model_list(
    base_url: &str,
    api_key: &str,
    provider_filter: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let fetch_url = format!("{}/v1/models", base_url.trim_end_matches('/'));

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let mut req = client.get(&fetch_url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = req.send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            anyhow::bail!("LiteLLM 返回错误 (HTTP {status}): {msg}");
        }

        let data = body["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("响应缺少 data 数组"))?;

        let filter_lower = provider_filter.to_lowercase();
        let mut models: Vec<(String, String)> = Vec::new();
        for entry in data {
            let id = entry["id"].as_str().unwrap_or("unknown");
            let provider = entry["litellm_params"]["litellm_provider"]
                .as_str()
                .or_else(|| entry["owned_by"].as_str())
                .unwrap_or("-");

            if !filter_lower.is_empty()
                && provider.to_lowercase() != filter_lower
                && id.to_lowercase().starts_with(&filter_lower)
            {
                // Also keep models whose id starts with the provider name
                // (e.g. "openai/gpt-4" for provider "openai")
                if !id.contains('/')
                    || !id.to_lowercase().starts_with(&format!("{}/", filter_lower))
                {
                    continue;
                }
            }

            models.push((id.to_string(), provider.to_string()));
        }

        if models.is_empty() {
            anyhow::bail!("LiteLLM 中未找到 {} 的模型", provider_filter);
        }

        models.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(models)
    })
}

fn run_configure(sections: &[String]) -> anyhow::Result<()> {
    use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};

    let theme = ColorfulTheme::default();
    let existing = tui::UserSettings::load();
    let mut next = existing.clone();

    println!();
    println!("  \x1b[1;36m╔══════════════════════════════╗\x1b[0m");
    println!("  \x1b[1;36m║   Hermess 配置向导           ║\x1b[0m");
    println!("  \x1b[1;36m╚══════════════════════════════╝\x1b[0m");

    let changed = std::cell::Cell::new(false);

    // ── LLM ──
    if section_enabled(sections, "llm") {
        println!();
        if !Confirm::with_theme(&theme)
            .with_prompt("配置 LLM 模型？")
            .default(true)
            .interact()?
        {
            println!("  \x1b[2m跳过 LLM 配置\x1b[0m");
        } else {
            println!("\x1b[1;33m── LLM 模型配置 ──\x1b[0m");
            let providers = &["anthropic", "openai", "deepseek"];
            let provider_idx = providers
                .iter()
                .position(|p| *p == existing.llm_provider)
                .unwrap_or(2);
            let provider_idx = Select::with_theme(&theme)
                .with_prompt("LLM 提供商")
                .default(provider_idx)
                .items(&[
                    "anthropic   (Claude 系列)",
                    "openai      (GPT 系列)",
                    "deepseek    (DeepSeek 系列)",
                ])
                .interact()?;
            let provider = providers[provider_idx].to_string();

            let default_url = match provider.as_str() {
                "anthropic" => "https://api.anthropic.com/v1",
                "openai" => "https://api.openai.com/v1",
                _ => "https://api.deepseek.com/v1",
            };

            next.llm_provider = provider.clone();

            let current_url = if existing.llm_provider != provider
                || existing.llm_base_url.is_empty()
                || !(existing.llm_base_url.starts_with("http://")
                    || existing.llm_base_url.starts_with("https://"))
            {
                default_url.to_string()
            } else {
                existing.llm_base_url.clone()
            };

            let api_key: String = Password::with_theme(&theme)
                .with_prompt(format!(
                    "API Key (当前: {})",
                    tui::UserSettings::mask_key(&existing.llm_api_key)
                ))
                .allow_empty_password(true)
                .interact()?;
            if !api_key.is_empty() {
                next.llm_api_key = api_key;
                changed.set(true);
            }

            // ── LiteLLM endpoint (auto-fetch model list) ──
            let effective_litellm_url: Option<String> = if !existing.litellm_url.is_empty() {
                Some(existing.litellm_url.clone())
            } else {
                println!("\x1b[1;33m── LiteLLM 端点（自动获取最新模型列表）──\x1b[0m");
                let url: String = Input::with_theme(&theme)
                    .with_prompt("LiteLLM URL (可选，回车跳过)")
                    .with_initial_text(&existing.litellm_url)
                    .allow_empty(true)
                    .validate_with(|v: &String| {
                        if v.is_empty() {
                            return Ok(());
                        }
                        if !v.starts_with("http://") && !v.starts_with("https://") {
                            Err("URL 必须以 http:// 或 https:// 开头")
                        } else {
                            Ok(())
                        }
                    })
                    .interact()?;
                if !url.is_empty() {
                    next.litellm_url = url.clone();
                    changed.set(true);
                    Some(url)
                } else {
                    None
                }
            };

            // ── Model selection ──

            // Build model items: try LiteLLM first, fallback to curated list
            let mut model_items: Vec<String> = Vec::new();
            let mut model_ids: Vec<String> = Vec::new();
            let from_litellm: bool;

            if let Some(ref litellm_url) = effective_litellm_url {
                let effective_key = if next.llm_api_key.is_empty() {
                    existing.llm_api_key.clone()
                } else {
                    next.llm_api_key.clone()
                };
                println!("\x1b[1;33m── 从 LiteLLM 获取 {provider} 模型列表 ──\x1b[0m");
                println!(
                    "  \x1b[2m请求: {}/v1/models\x1b[0m",
                    litellm_url.trim_end_matches('/')
                );
                match fetch_litellm_model_list(litellm_url, &effective_key, &provider) {
                    Ok(list) => {
                        println!("  \x1b[32m✓ 获取到 {} 个模型\x1b[0m", list.len());
                        for (id, prov) in &list {
                            model_items.push(format!("{id}  ({prov})"));
                            model_ids.push(id.clone());
                        }
                        from_litellm = true;
                    }
                    Err(e) => {
                        println!("  \x1b[33m⚠ {e}\x1b[0m");
                        println!("  \x1b[2m回退到内置模型列表\x1b[0m");
                        from_litellm = false;
                    }
                }
            } else {
                from_litellm = false;
            }

            if !from_litellm {
                // Per-provider curated model list
                let curated: &[&str] = match provider.as_str() {
                    "anthropic" => &[
                        "claude-opus-4-20250514",
                        "claude-sonnet-4-20250514",
                        "claude-haiku-4-20250514",
                        "claude-opus-4-20250514 (self-serve)",
                        "claude-sonnet-4-20250514 (self-serve)",
                    ],
                    "openai" => &[
                        "gpt-4.1",
                        "gpt-4.1-mini",
                        "gpt-4o",
                        "gpt-4o-mini",
                        "gpt-5",
                        "gpt-5-mini",
                        "o4-mini",
                        "o3",
                        "o3-mini",
                    ],
                    _ => &["deepseek-chat", "deepseek-reasoner", "deepseek-chat-v3"],
                };
                for m in curated {
                    model_items.push(m.to_string());
                    model_ids.push(m.to_string());
                }
            }

            // Add "自定义输入..." at the end
            model_items.push("自定义输入...".to_string());

            let current_model =
                if existing.llm_provider != provider || existing.llm_model.is_empty() {
                    String::new()
                } else {
                    existing.llm_model.clone()
                };

            let model_default_idx = model_ids
                .iter()
                .position(|m| *m == current_model)
                .unwrap_or(model_items.len() - 1);

            println!("\x1b[1;33m── 模型选择 ──\x1b[0m");
            if !current_model.is_empty() && model_default_idx == model_items.len() - 1 {
                println!("  \x1b[2m当前模型: {current_model} (不在列表中)\x1b[0m");
            }
            let select_items: Vec<&str> = model_items.iter().map(|s| s.as_str()).collect();
            let model_idx = Select::with_theme(&theme)
                .with_prompt(format!("选择模型 (共 {} 个)", model_items.len() - 1))
                .default(model_default_idx)
                .items(&select_items)
                .interact()?;

            let model: String = if model_items[model_idx] == "自定义输入..." {
                // 仅当当前模型在已知列表中时才预填，避免垃圾值反复出现
                let initial = if model_default_idx < model_items.len() - 1 {
                    current_model.as_str()
                } else {
                    ""
                };
                Input::with_theme(&theme)
                    .with_prompt("输入模型名")
                    .with_initial_text(initial)
                    .validate_with(|v: &String| {
                        if v.trim().is_empty() {
                            Err("模型名不能为空")
                        } else {
                            Ok(())
                        }
                    })
                    .interact()?
            } else {
                model_ids[model_idx].clone()
            };

            if model != existing.llm_model {
                changed.set(true);
            }
            next.llm_model = model;

            let base_url: String = Input::with_theme(&theme)
                .with_prompt("Base URL")
                .with_initial_text(&current_url)
                .validate_with(|v: &String| {
                    if v.trim().is_empty() {
                        Err("Base URL 不能为空")
                    } else if !v.starts_with("http://") && !v.starts_with("https://") {
                        Err("URL 必须以 http:// 或 https:// 开头")
                    } else {
                        Ok(())
                    }
                })
                .interact()?;
            if base_url != existing.llm_base_url {
                changed.set(true);
            }
            next.llm_base_url = base_url;

            if next.llm_provider != existing.llm_provider {
                changed.set(true);
            }
        }
    }

    // ── Search ──
    if section_enabled(sections, "search") {
        println!();
        if !Confirm::with_theme(&theme)
            .with_prompt("配置搜索引擎？")
            .default(false)
            .interact()?
        {
            println!("  \x1b[2m跳过搜索配置\x1b[0m");
        } else {
            println!("\x1b[1;33m── 搜索配置 ──\x1b[0m");
            let enable_search = Select::with_theme(&theme)
                .with_prompt("启用搜索？")
                .default(if existing.search_enabled { 0 } else { 1 })
                .items(&["是", "否"])
                .interact()?
                == 0;
            if enable_search != existing.search_enabled {
                changed.set(true);
            }
            next.search_enabled = enable_search;

            if enable_search {
                let search_key: String = Password::with_theme(&theme)
                    .with_prompt(format!(
                        "Brave Search API Key (当前: {})",
                        tui::UserSettings::mask_key(&existing.search_api_key)
                    ))
                    .allow_empty_password(true)
                    .interact()?;
                if !search_key.is_empty() {
                    next.search_api_key = search_key;
                    changed.set(true);
                }
            }
        }
    }

    // ── Finance ──
    if section_enabled(sections, "finance") {
        println!();
        if !Confirm::with_theme(&theme)
            .with_prompt("配置金融数据源？")
            .default(false)
            .interact()?
        {
            println!("  \x1b[2m跳过金融配置\x1b[0m");
        } else {
            println!("\x1b[1;33m── 金融数据配置 ──\x1b[0m");
            let fin_providers = &["ftshare", "tushare", "sina", "eastmoney", "tencent"];
            let fin_idx = fin_providers
                .iter()
                .position(|p| *p == existing.finance_provider)
                .unwrap_or(0);
            let fin_idx = Select::with_theme(&theme)
                .with_prompt("金融数据源")
                .default(fin_idx)
                .items(fin_providers)
                .interact()?;
            let fin = fin_providers[fin_idx].to_string();
            if fin != existing.finance_provider {
                changed.set(true);
            }
            next.finance_provider = fin;

            if fin_idx == 1 {
                // tushare needs token
                let token: String = Password::with_theme(&theme)
                    .with_prompt(format!(
                        "Tushare Token (当前: {})",
                        tui::UserSettings::mask_key(&existing.finance_tushare_token)
                    ))
                    .allow_empty_password(true)
                    .interact()?;
                if !token.is_empty() {
                    next.finance_tushare_token = token;
                    changed.set(true);
                }
            }
        }
    }

    // ── Feishu ──
    if section_enabled(sections, "feishu") {
        println!();
        let has_feishu = !existing.feishu_app_id.is_empty();
        if !Confirm::with_theme(&theme)
            .with_prompt(format!(
                "配置飞书平台？{}",
                if has_feishu { "(当前已配置)" } else { "" }
            ))
            .default(has_feishu)
            .interact()?
        {
            println!("  \x1b[2m跳过飞书配置\x1b[0m");
        } else {
            println!("\x1b[1;33m── 飞书平台配置 ──\x1b[0m");
            let app_id: String = Input::with_theme(&theme)
                .with_prompt("App ID")
                .with_initial_text(&existing.feishu_app_id)
                .validate_with(|v: &String| {
                    if v.trim().is_empty() {
                        Err("App ID 不能为空")
                    } else {
                        Ok(())
                    }
                })
                .interact()?;
            if app_id != existing.feishu_app_id {
                changed.set(true);
            }
            next.feishu_app_id = app_id;

            let app_secret: String = Password::with_theme(&theme)
                .with_prompt(format!(
                    "App Secret (当前: {})",
                    tui::UserSettings::mask_key(&existing.feishu_app_secret)
                ))
                .allow_empty_password(true)
                .interact()?;
            if !app_secret.is_empty() {
                next.feishu_app_secret = app_secret;
                changed.set(true);
            }

            let bot_open_id: String = Input::with_theme(&theme)
                .with_prompt("Bot Open ID (可选，过滤自身消息，回车跳过)")
                .with_initial_text(&existing.feishu_bot_open_id)
                .allow_empty(true)
                .interact()?;
            if bot_open_id != existing.feishu_bot_open_id {
                changed.set(true);
            }
            next.feishu_bot_open_id = bot_open_id;
        }
    }

    // ── WeChat ──
    if section_enabled(sections, "wechat") {
        println!();
        let has_wechat = !existing.wechat_corp_id.is_empty();
        if !Confirm::with_theme(&theme)
            .with_prompt(format!(
                "配置企业微信？{}",
                if has_wechat { "(当前已配置)" } else { "" }
            ))
            .default(has_wechat)
            .interact()?
        {
            println!("  \x1b[2m跳过企业微信配置\x1b[0m");
        } else {
            println!("\x1b[1;33m── 企业微信配置 ──\x1b[0m");
            let corp_id: String = Input::with_theme(&theme)
                .with_prompt("Corp ID (企业ID)")
                .with_initial_text(&existing.wechat_corp_id)
                .validate_with(|v: &String| {
                    if v.trim().is_empty() {
                        Err("Corp ID 不能为空")
                    } else {
                        Ok(())
                    }
                })
                .interact()?;
            if corp_id != existing.wechat_corp_id {
                changed.set(true);
            }
            next.wechat_corp_id = corp_id;

            let corp_secret: String = Password::with_theme(&theme)
                .with_prompt(format!(
                    "Corp Secret (当前: {})",
                    tui::UserSettings::mask_key(&existing.wechat_corp_secret)
                ))
                .allow_empty_password(true)
                .interact()?;
            if !corp_secret.is_empty() {
                next.wechat_corp_secret = corp_secret;
                changed.set(true);
            }

            let agent_id: String = Input::with_theme(&theme)
                .with_prompt("Agent ID (应用AgentID)")
                .with_initial_text(&existing.wechat_agent_id)
                .validate_with(|v: &String| {
                    if v.trim().is_empty() {
                        Err("Agent ID 不能为空")
                    } else {
                        Ok(())
                    }
                })
                .interact()?;
            if agent_id != existing.wechat_agent_id {
                changed.set(true);
            }
            next.wechat_agent_id = agent_id;
        }
    }

    // ── Preview & confirm ──
    println!();
    if !changed.get() {
        println!("  \x1b[2m配置未变更，无需保存\x1b[0m\n");
        return Ok(());
    }

    println!("  \x1b[1;36m── 变更预览 ──\x1b[0m");
    show_diff("LLM 提供商", &existing.llm_provider, &next.llm_provider);
    show_diff("LLM 模型", &existing.llm_model, &next.llm_model);
    show_diff("LLM Base URL", &existing.llm_base_url, &next.llm_base_url);
    if existing.llm_api_key != next.llm_api_key {
        println!("  API Key: \x1b[33m已更新\x1b[0m");
    }
    if existing.search_api_key != next.search_api_key {
        println!("  搜索 Key: \x1b[33m已更新\x1b[0m");
    }
    if existing.search_enabled != next.search_enabled {
        println!(
            "  搜索: {} → \x1b[33m{}\x1b[0m",
            if existing.search_enabled {
                "开"
            } else {
                "关"
            },
            if next.search_enabled { "开" } else { "关" },
        );
    }
    show_diff(
        "金融数据源",
        &existing.finance_provider,
        &next.finance_provider,
    );
    if existing.finance_tushare_token != next.finance_tushare_token {
        println!("  Tushare Token: \x1b[33m已更新\x1b[0m");
    }
    show_diff("飞书 App ID", &existing.feishu_app_id, &next.feishu_app_id);
    if existing.feishu_app_secret != next.feishu_app_secret {
        println!("  飞书 App Secret: \x1b[33m已更新\x1b[0m");
    }
    show_diff(
        "飞书 Bot Open ID",
        &existing.feishu_bot_open_id,
        &next.feishu_bot_open_id,
    );
    show_diff(
        "企微 Corp ID",
        &existing.wechat_corp_id,
        &next.wechat_corp_id,
    );
    if existing.wechat_corp_secret != next.wechat_corp_secret {
        println!("  企微 Corp Secret: \x1b[33m已更新\x1b[0m");
    }
    show_diff(
        "企微 Agent ID",
        &existing.wechat_agent_id,
        &next.wechat_agent_id,
    );
    if existing.litellm_url != next.litellm_url {
        println!(
            "  LiteLLM URL: {} → \x1b[33m{}\x1b[0m",
            if existing.litellm_url.is_empty() {
                "(未设置)"
            } else {
                &existing.litellm_url
            },
            if next.litellm_url.is_empty() {
                "(未设置)"
            } else {
                &next.litellm_url
            },
        );
    }

    println!();
    if Confirm::with_theme(&theme)
        .with_prompt("确认保存以上变更？")
        .default(true)
        .interact()?
    {
        next.save().map_err(|e| anyhow::anyhow!("{e}"))?;
        println!();
        println!("  \x1b[1;32m✓ 配置已保存到 .hermess/settings.json\x1b[0m");
    } else {
        println!();
        println!("  \x1b[2m已取消，配置未修改\x1b[0m");
    }
    println!();
    Ok(())
}

fn show_diff(label: &str, old: &str, new: &str) {
    if old != new {
        let old_display = if old.is_empty() { "(未设置)" } else { old };
        let new_display = if new.is_empty() { "(未设置)" } else { new };
        println!("  {label}: {old_display} → \x1b[33m{new_display}\x1b[0m");
    }
}

// ── Main ─────────────────────────────────────────────────────

fn init_tracing(tui_mode: bool) {
    use std::env;
    let use_json = env::var("LOG_FORMAT").map(|v| v == "json").unwrap_or(false);
    let builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    if tui_mode {
        builder.with_writer(std::io::sink).init();
        return;
    }
    if use_json {
        builder.json().init();
    } else {
        builder.init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // ── 子命令：hermes configure ─────────────────────────
    if let Some(Command::Configure { section }) = &cli.command {
        return run_configure(section);
    }

    init_tracing(cli.tui);

    if cli.gateway {
        return run_gateway(&cli.gateway_config).await;
    }

    // ── Profile override: --profile dev → config/profiles/dev.toml ──
    let config_path = if let Some(ref profile) = cli.profile {
        let path = format!("config/profiles/{profile}.toml");
        tracing::info!(profile = %profile, path = %path, "使用配置预设");
        path
    } else {
        cli.config.clone()
    };

    let mut cfg = Config::from_file(&config_path)?;

    // ── Apply settings.json overrides (above config file, below CLI) ──
    let mut user_settings = tui::UserSettings::load();
    user_settings.apply_env_overrides();
    if !user_settings.llm_provider.is_empty() {
        tracing::info!(
            provider = %user_settings.llm_provider,
            model = %user_settings.llm_model,
            "Applying user settings from .hermess/settings.json"
        );
        cfg.llm.provider = user_settings.llm_provider.clone();
    }
    if !user_settings.llm_model.is_empty() {
        cfg.llm.model = user_settings.llm_model.clone();
    }
    if !user_settings.llm_api_key.is_empty() {
        cfg.llm.api_key = user_settings.llm_api_key.clone();
    }
    if !user_settings.llm_base_url.is_empty() {
        cfg.llm.base_url = user_settings.llm_base_url.clone();
    }
    if user_settings.search_enabled && !user_settings.search_api_key.is_empty() {
        cfg.search.api_key = Some(user_settings.search_api_key.clone());
    }

    // ── 平台适配器配置（必须在 CLI overrides 之前，避免 partial move） ──
    let feishu_cfg = build_feishu_config(&cli, &cfg, &user_settings);
    let wechat_cfg = build_wechat_config(&cli, &cfg, &user_settings);
    if !feishu_cfg.app_id.is_empty() {
        tracing::info!(
            app_id = %feishu_cfg.app_id,
            has_secret = !feishu_cfg.app_secret.is_empty(),
            "飞书平台配置已加载"
        );
    }
    if wechat_cfg.agent_id != 0 {
        tracing::info!(
            corp_id = %wechat_cfg.corp_id,
            agent_id = wechat_cfg.agent_id,
            "企业微信平台配置已加载"
        );
    }

    // ── Apply CLI overrides to config ─────────────────────
    if let Some(v) = cli.api_key {
        cfg.llm.api_key = v;
    }
    if let Some(v) = cli.provider {
        cfg.llm.provider = v;
    }
    if let Some(v) = cli.model {
        cfg.llm.model = v;
    }
    if let Some(v) = cli.max_tokens {
        cfg.llm.max_tokens = v;
    }
    if let Some(v) = cli.base_url {
        cfg.llm.base_url = v;
    }
    if let Some(v) = cli.search_api_key {
        cfg.search.api_key = Some(v);
    }
    if let Some(v) = cli.learning_rate {
        cfg.learning_rate = v;
    }
    if let Some(v) = cli.max_concurrency {
        cfg.max_concurrency = v;
    }
    if let Some(ref v) = cli.danger_mode {
        cfg.guard.danger_mode = v.clone();
    }
    if let Some(v) = cli.max_step_retries {
        cfg.max_step_retries = v;
    }
    if let Some(v) = cli.max_replans {
        cfg.max_replans = v;
    }
    if let Some(v) = cli.compress_threshold {
        cfg.compress_threshold = v;
    }
    if let Some(v) = cli.compress_keep_ratio {
        cfg.compress_keep_ratio = v;
    }

    // ── Assemble dependencies (matching arch.md Section 6) ────
    let memory: Arc<dyn agent_core::MemoryStore> = Arc::new(
        memory::VectorMemory::new(&memory::VectorMemoryConfig {
            url: cfg.qdrant.url.clone(),
            collection: cfg.qdrant.collection.clone(),
            embedding_dim: cfg.qdrant.embedding_dim,
        })
        .await?,
    );

    let llm: Arc<dyn llm::LlmAdapter> = match cfg.llm.provider.as_str() {
        "openai" | "deepseek" | "litellm" => {
            let key = if cfg.llm.api_key.is_empty() {
                let k = std::env::var("OPENAI_API_KEY")
                    .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
                    .unwrap_or_default();
                if k.is_empty() {
                    tracing::warn!(
                        "未配置 LLM API Key！请设置 DEEPSEEK_API_KEY 或 OPENAI_API_KEY 环境变量，\
                         或运行 hermes configure 进行配置"
                    );
                    String::new()
                } else {
                    k
                }
            } else {
                cfg.llm.api_key.clone()
            };
            let base_url = if cfg.llm.base_url.is_empty() {
                if cfg.llm.provider == "deepseek" {
                    "https://api.deepseek.com/v1".to_string()
                } else {
                    "https://api.openai.com/v1".to_string()
                }
            } else {
                cfg.llm.base_url.clone()
            };
            Arc::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: key,
                model: if cfg.llm.model.is_empty() && cfg.llm.provider == "deepseek" {
                    "deepseek-chat".into()
                } else {
                    cfg.llm.model.clone()
                },
                max_tokens: cfg.llm.max_tokens,
                base_url,
            }))
        }
        _ => {
            let key = if cfg.llm.api_key.is_empty() {
                let k = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
                if k.is_empty() {
                    tracing::warn!("No Anthropic API key configured — API calls will fail");
                }
                k
            } else {
                cfg.llm.api_key.clone()
            };
            Arc::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: key,
                model: cfg.llm.model.clone(),
                max_tokens: cfg.llm.max_tokens,
            }))
        }
    };

    let danger_guard = Arc::new(tools::DangerGuard::new(
        tools::ConfirmationPolicy::from_str(&cfg.guard.danger_mode).unwrap(),
        cfg.guard.dangerous_patterns.clone(),
    ));

    let tools = Arc::new(tools::ToolRegistry::default());
    tools.register(Arc::new(tools::ReplyTool));
    tools.register(Arc::new(tools::BashTool::new(Arc::clone(&danger_guard))));
    tools.register(Arc::new(tools::ReadFileTool));
    tools.register(Arc::new(tools::WriteFileTool));
    if cfg.search.api_key.is_some() {
        tools.register(Arc::new(tools::WebSearchTool::new(&tools::SearchConfig {
            api_key: cfg.search.api_key.clone(),
            endpoint: cfg.search.endpoint.clone(),
        })));
    } else {
        tracing::info!("No search API key configured — web_search tool disabled");
    }

    // Discover and register plugins
    for plugin_dir in &cfg.plugin_dirs {
        match tools::discover_plugins(plugin_dir) {
            Ok(plugins) => {
                for (manifest, dir) in plugins {
                    match manifest.plugin_type.as_str() {
                        "shell" => {
                            tools.register(Arc::new(tools::ShellPlugin::new(manifest)));
                        }
                        "script" => {
                            tools.register(Arc::new(tools::ScriptPlugin::new(manifest, dir)));
                        }
                        other => {
                            tracing::warn!(plugin_type = %other, name = %manifest.name, "未知插件类型");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(dir = %plugin_dir, error = %e, "插件目录扫描失败");
            }
        }
    }

    // Register financial data tool with automatic failover.
    // Resolution order: env var > settings.json.
    // When a primary provider fails (network error / unsupported query),
    // the composite automatically falls back to the next available provider.
    let finance_provider_name = std::env::var("HERMESS_FINANCE_PROVIDER")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            let p = user_settings.finance_provider.as_str();
            if p.is_empty() || p == "none" || p == "null" {
                None
            } else {
                Some(p.to_string())
            }
        });
    let finance_provider = hermess_finance::providers::defaults::build_finance_provider(
        hermess_finance::providers::defaults::FinanceProviderOptions {
            provider: finance_provider_name,
            ftshare_url: std::env::var("HERMESS_FINANCE_URL").ok(),
            tushare_token: std::env::var("HERMESS_TUSHARE_TOKEN").ok().or_else(|| {
                (!user_settings.finance_tushare_token.is_empty())
                    .then(|| user_settings.finance_tushare_token.clone())
            }),
            allow_disable: true,
        },
    );
    tools.register(Arc::new(hermess_finance::tool::FinancialTool::new(
        finance_provider,
    )));

    let evolution = Arc::new(
        evolution::EvolutionEngine::load_from_file(
            ".hermes_evolution.json",
            cfg.learning_rate,
            Arc::clone(&memory),
        )
        .unwrap_or_else(|e| {
            let err_str = e.to_string();
            if err_str.contains("No such file") || err_str.contains("entity not found") {
                tracing::info!("No previous evolution state found, starting fresh");
            } else {
                tracing::warn!(error = %e, "Failed to load evolution state, starting fresh");
            }
            evolution::EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory))
        })
        .with_auto_save(".hermes_evolution.json"),
    );

    let mut planner = planner::Planner::new(
        Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        Arc::clone(&evolution),
    )
    .with_streaming(true);
    planner.set_tools(tools.describe_all());

    // ── Create event channel (TUI mode only) ────────────────
    // Must be created before the scheduler so subagent_runner can capture it.
    let (event_tx, event_rx) = if cli.tui {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let subagent_runner = Arc::new(hermess_agent::SubAgentRunnerImpl::new(
        Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        Arc::clone(&evolution),
        Arc::clone(&tools),
        event_tx.clone(),
    ));
    let mut scheduler = scheduler::Scheduler::new(Arc::clone(&tools), cfg.max_concurrency)
        .with_max_retries(cfg.max_step_retries)
        .with_subagent_runner(subagent_runner);

    // Wire event sender to planner and scheduler for TUI progress reporting.
    if let Some(ref tx) = event_tx {
        planner.set_event_sender(tx.clone());
        scheduler.set_event_sender(tx.clone());
    }

    let reflector = if cfg.scorer.success_weight != 0.6 || cfg.scorer.latency_weight != 0.2 {
        reflector::Reflector::with_scorer(
            Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
            evolution::Scorer {
                success_weight: cfg.scorer.success_weight,
                latency_weight: cfg.scorer.latency_weight,
                quality_weight: cfg.scorer.quality_weight,
                latency_target_ms: cfg.scorer.latency_target_ms,
            },
        )
    } else {
        reflector::Reflector::new(Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>)
    };

    let usage_tracker = Arc::new(llm::UsageTracker::new(&cfg.llm.model));
    let evolution_handle = Arc::clone(&evolution);

    // ── Knowledge base preload ─────────────────────────────
    for kb_dir in &cli.knowledge_base {
        tracing::info!(dir = %kb_dir, "预加载知识库...");
        match memory::preload_knowledge_base(
            kb_dir,
            memory.as_ref(),
            llm.as_ref(),
            &|msg| tracing::info!(progress = %msg),
        )
        .await
        {
            Ok(stats) => {
                tracing::info!(
                    files = stats.files_found,
                    chunks = stats.chunks_upserted,
                    skipped = stats.files_skipped,
                    "知识库加载完成"
                );
            }
            Err(e) => {
                tracing::warn!(dir = %kb_dir, error = %e, "知识库加载失败");
            }
        }
    }

    // Shared stop signal for TUI cancel + agent Ctrl-C handler
    let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let flag = Arc::clone(&stop_flag);
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    tracing::info!("received Ctrl-C, signalling stop");
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to register Ctrl-C handler");
                }
            }
        });
    }

    let tui_input = if cli.tui {
        let mut input = tui::TuiInput::new();
        // Wire gateway route mode: share the LLM adapter's Arc so TUI settings
        // changes are reflected in actual API requests.
        input.gateway_mode = llm.route_mode();
        // Share stop flag so TUI can cancel agent operations
        input.stop_flag = Some(Arc::clone(&stop_flag));
        Some(Arc::new(input))
    } else {
        None
    };

    let mut agent = hermess_agent::SmallHermesAgent {
        planner,
        scheduler,
        reflector,
        evolution,
        working_memory: memory::WorkingMemory::new(cfg.working_memory_size),
        llm: Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        turn: 0,
        usage_tracker: Arc::clone(&usage_tracker),
        max_replans: cfg.max_replans,
        compress_threshold: cfg.compress_threshold,
        compress_keep_ratio: cfg.compress_keep_ratio,
        event_tx: event_tx.clone(),
        conversation_history: Vec::new(),
        recent_insights: Vec::new(),
        distiller: hermess_agent::SkillDistiller::new(),
        compressor_llm: None,
        compress_target_tokens: 0,
        mimo: None,
        injection_detector: None,
        tui_input: tui_input.clone(),
    };

    let tui_interactive = cli.tui && cli.task.is_none();
    let effective_interactive = cli.interactive || tui_interactive;

    tracing::info!(
        "Hermes agent starting: provider={}, model={}, config={}, interactive={}, tui={}",
        cfg.llm.provider,
        cfg.llm.model,
        config_path,
        effective_interactive,
        cli.tui,
    );

    // ── Resume session if requested ────────────────────────
    if let Some(ref resume_path) = cli.resume {
        agent.restore_state(resume_path)?;
    }

    // ── Run ──────────────────────────────────────────────────
    let save_path = cli.save.clone();

    if cli.mcp_server {
        // MCP stdio server mode — exposes tools via the Model Context Protocol
        let tools_arc = Arc::clone(&tools);
        let mcp_handler = HermesMcpHandler { tools: tools_arc };
        tracing::info!("Starting MCP stdio server");
        mcp::run_stdio_server(Box::new(mcp_handler)).await?;
        return Ok(());
    }

    if let Some(port) = cli.serve {
        // HTTP server mode — runs until process is killed, then saves state
        tracing::info!("Starting HTTP server on port {}", port);
        run_server(agent, port).await?;
        if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
            tracing::warn!(error = %e, "Failed to save evolution state");
        }
        tracing::info!(usage = %usage_tracker.snapshot(), "Hermes server stopped.");
        return Ok(());
    }

    if let Some(ref cron_expr) = cli.schedule {
        let schedule = scheduler::CronSchedule::parse(cron_expr)
            .map_err(|e| anyhow::anyhow!("无效的 cron 表达式: {e}"))?;
        tracing::info!(cron = %cron_expr, "定时调度模式启动");
        loop {
            let now = chrono::Utc::now();
            let wait_secs = schedule.next_in_secs(&now).unwrap_or(3600).min(86400); // cap at 24h

            tracing::info!(
                next_run = %(now + chrono::Duration::seconds(wait_secs)),
                wait_secs = wait_secs,
                "等待下次执行"
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait_secs as u64)).await;

            let task = cli.task.clone().unwrap_or_else(|| "scheduled run".into());
            let ctx = agent_core::context::Context::new(Some(task));
            agent = agent_core::runner::run_agent(agent, ctx).await?;

            if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
                tracing::warn!(error = %e, "Failed to save evolution state");
            }
        }
    }

    // ── Gateway model discovery for TUI display ──
    // Detect gateway mode: model="auto" OR base_url points to gateway port
    let is_gateway_mode = cfg.llm.model == "auto"
        || cfg.llm.base_url.contains("9090")
        || cfg.llm.base_url.contains("gateway");
    if cli.tui && is_gateway_mode {
        tracing::info!(base_url = %cfg.llm.base_url, model = %cfg.llm.model, "TUI gateway mode detected, querying models");
        let models = query_gateway_models(&cfg.llm.base_url, &cfg.llm.api_key).await;
        tracing::info!(count = models.len(), "Gateway models discovered");
        if let Some(ref tx) = event_tx {
            let _ = tx.send(AgentEvent::GatewayModelsDiscovered {
                models: models.clone(),
                gateway_url: cfg.llm.base_url.clone(),
            });
        }
        // Pre-populate gateway_models so settings shows them immediately
        // (the event will also set them when TUI processes it)
    }

    if cli.tui {
        let ctx = agent_core::context::Context::interactive_with_task(cli.task.clone())
            .with_stop_flag(Arc::clone(&stop_flag));
        agent = tui::run_tui(
            agent,
            ctx,
            event_rx.unwrap(),
            Arc::clone(&evolution_handle),
            "Hermes Agent".into(),
            tui_input.unwrap(),
            Some(Arc::clone(&usage_tracker)),
        )
        .await?;
    } else if cli.task.is_some() || cli.interactive {
        let ctx = if cli.interactive {
            eprintln!(
                "\n\x1b[1;35m🜁 Hermes Agent — 交互模式\x1b[0m\n\
                 \x1b[2m输入任务开始对话，输入 exit 或 Ctrl+C 退出\x1b[0m\n"
            );
            agent_core::context::Context::interactive()
        } else {
            agent_core::context::Context::new(cli.task)
        };
        agent = agent_core::runner::run_agent(agent, ctx).await?;
    } else {
        let ctx = agent_core::context::Context::new(None);
        agent = agent_core::runner::run_agent(agent, ctx).await?;
    }

    // ── Save session if requested ──────────────────────────
    if let Some(ref save_path) = save_path {
        if let Err(e) = agent.save_state(save_path) {
            tracing::warn!(error = %e, "Failed to save session");
        }
    }

    // Save evolution state on exit
    if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
        tracing::warn!(error = %e, "Failed to save evolution state");
        anyhow::bail!("Failed to save evolution state: {e}");
    }

    tracing::info!(usage = %usage_tracker.snapshot(), "Hermes Agent stopped.");
    Ok(())
}

// ── HTTP Server mode ─────────────────────────────────────────────

#[derive(Deserialize)]
struct RunRequest {
    task: String,
}

async fn run_server(agent: hermess_agent::SmallHermesAgent, port: u16) -> anyhow::Result<()> {
    use std::net::SocketAddr;
    use tower_http::{
        cors::CorsLayer,
        limit::RequestBodyLimitLayer,
        request_id::{MakeRequestId, RequestId, SetRequestIdLayer},
    };
    use uuid::Uuid;

    #[derive(Clone, Default)]
    struct MakeRequestUuid;
    impl MakeRequestId for MakeRequestUuid {
        fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
            let id = Uuid::new_v4().to_string().parse().ok()?;
            Some(RequestId::new(id))
        }
    }

    let agent_arc = Arc::new(Mutex::new(agent));

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/agent/run", post(run_agent_handler))
        .layer((
            SetRequestIdLayer::x_request_id(MakeRequestUuid),
            RequestBodyLimitLayer::new(4 * 1024 * 1024),
            CorsLayer::permissive(),
        ))
        .with_state(agent_arc);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "HTTP server starting");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received, draining...");
}

async fn health_handler() -> &'static str {
    "ok"
}

fn agent_error_json(message: String) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "message": message,
            "type": "agent_error"
        }
    })
}

async fn run_agent_handler(
    AxumState(agent_arc): AxumState<Arc<Mutex<hermess_agent::SmallHermesAgent>>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let ctx = agent_core::context::Context::new(Some(req.task));
    let agent_guard = agent_arc.lock().await;
    let agent_clone = Arc::clone(&agent_arc);
    drop(agent_guard);

    let handle = tokio::spawn(async move {
        let mut ag = agent_clone.lock().await;
        ag.run_loop(ctx).await
    });

    match handle.await {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "summary": "任务已完成。",
                "turn": 0,
                "success": true,
            })),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(agent_error_json(format!("执行错误: {:#}", e))),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(agent_error_json(format!("任务异常: {:#}", e))),
        )
            .into_response(),
    }
}

// ── Gateway ────────────────────────────────────────────────────────

async fn run_gateway(config_path: &str) -> anyhow::Result<()> {
    use std::net::SocketAddr;
    use tokio::net::TcpSocket;

    let cfg = hermess_gateway::config::GatewayConfig::from_file(config_path)?;
    let listen_addr = cfg.gateway.listen.clone();
    let gateway = hermess_gateway::gateway::Gateway::new(cfg, "embedded", false).await;

    tracing::info!(addr = %listen_addr, instance = "embedded", "Hermess Gateway starting");

    let feedback = std::sync::Arc::clone(&gateway.feedback);
    let app = hermess_gateway::server::build_router(gateway);

    let parsed: SocketAddr = listen_addr.parse()?;
    let socket = if parsed.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(parsed)?;
    let listener = socket.listen(4096)?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    let feedback_file = ".hermes_feedback_embedded.json";
    if let Err(e) = feedback.save_to_file(feedback_file) {
        tracing::error!(error = %e, "Failed to save feedback state");
    }
    Ok(())
}

// ── Gateway model discovery ────────────────────────────────────────

async fn query_gateway_models(base_url: &str, api_key: &str) -> Vec<String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    tracing::info!(%url, "Querying gateway models");
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                tracing::warn!(%url, %status, "Gateway models endpoint returned error");
                return Vec::new();
            }
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    if let Some(models) = body["data"].as_array() {
                        let ids: Vec<String> = models
                            .iter()
                            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                            .collect();
                        tracing::info!(%url, count = ids.len(), "Gateway models discovered");
                        return ids;
                    }
                    tracing::warn!(%url, "Gateway models response missing 'data' array");
                }
                Err(e) => {
                    tracing::warn!(%url, error = %e, "Failed to parse gateway models response");
                }
            }
            Vec::new()
        }
        Err(e) => {
            tracing::warn!(%url, error = %e, "Failed to query gateway models");
            Vec::new()
        }
    }
}

// ── MCP Handler ──────────────────────────────────────────────────

struct HermesMcpHandler {
    tools: Arc<tools::ToolRegistry>,
}

#[async_trait::async_trait]
impl mcp::McpHandler for HermesMcpHandler {
    async fn list_tools(&self) -> anyhow::Result<Vec<mcp::ToolDef>> {
        let tools = self.tools.describe_all();
        let defs: Vec<mcp::ToolDef> = tools
            .into_iter()
            .map(|t| mcp::ToolDef {
                name: t
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                description: t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                input_schema: t.get("schema").cloned().unwrap_or(serde_json::json!({})),
            })
            .collect();
        Ok(defs)
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Option<serde_json::Value>,
    ) -> anyhow::Result<mcp::ToolCallResult> {
        let args = args.unwrap_or(serde_json::json!({}));
        let output = self.tools.call(name, args).await?;
        Ok(mcp::ToolCallResult {
            content: vec![mcp::ToolCallContent {
                content_type: "text".into(),
                text: output.content,
            }],
            is_error: Some(!output.success),
        })
    }
}
