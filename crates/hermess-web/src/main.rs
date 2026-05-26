// hermess-webd — 企业微信接入的 Hermes Agent 守护进程
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use hermess_web::session::SessionManager;
use hermess_web::wechat::client::WeChatClient;

/// Hermes Web Daemon — 通过企业微信控制 Hermes Agent
#[derive(Parser)]
#[command(name = "hermess-webd")]
struct Cli {
    #[arg(short, long, default_value = "config/wechat.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg = hermess_web::WebAppConfig::from_file(&cli.config)?;

    tracing::info!(
        "hermess-webd starting: provider={}, model={}, server={}:{}",
        cfg.llm.provider,
        cfg.llm.model,
        cfg.server.host,
        cfg.server.port,
    );

    // ── 共享资源 ──────────────────────────────────────────
    let memory: Arc<dyn agent_core::MemoryStore> = Arc::new(
        memory::VectorMemory::new(&memory::VectorMemoryConfig {
            url: cfg.qdrant.url.clone(),
            collection: cfg.qdrant.collection.clone(),
            embedding_dim: cfg.qdrant.embedding_dim,
        })
        .await?,
    );

    let llm: Arc<dyn llm::LlmAdapter> = match cfg.llm.provider.as_str() {
        "openai" | "deepseek" => {
            let key = if cfg.llm.api_key.is_empty() {
                std::env::var("OPENAI_API_KEY")
                    .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
                    .unwrap_or_default()
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
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
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

    let tools = Arc::new(tools::ToolRegistry::default());
    let danger_guard = Arc::new(tools::DangerGuard::new(
        tools::ConfirmationPolicy::from_str("ask").unwrap(),
        vec![],
    ));
    tools.register(Arc::new(tools::ReplyTool));
    tools.register(Arc::new(tools::BashTool::new(Arc::clone(&danger_guard))));
    tools.register(Arc::new(tools::ReadFileTool));
    tools.register(Arc::new(tools::WriteFileTool));
    tools.register(Arc::new(tools::WebSearchTool::new(&tools::SearchConfig {
        api_key: cfg.search.api_key.clone(),
        endpoint: cfg.search.endpoint.clone(),
    })));

    let evolution = Arc::new(
        evolution::EvolutionEngine::load_from_file(
            ".hermes_web_evolution.json",
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
        }),
    );

    let evolution_handle = Arc::clone(&evolution);

    // ── 微信 API 客户端 ────────────────────────────────────
    let wx_client = WeChatClient::new(
        cfg.wechat.corp_id.clone(),
        cfg.wechat.agent_id.clone(),
        cfg.wechat.secret.clone(),
    );

    // ── 会话管理器 ─────────────────────────────────────────
    let sessions = Arc::new(SessionManager::new(
        Arc::clone(&evolution),
        Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        Arc::clone(&tools),
        cfg.max_concurrency,
        cfg.working_memory_size,
    ));
    sessions.clone().start_cleanup();

    // ── HTTP 服务器 ────────────────────────────────────────
    let state = Arc::new(hermess_web::server::AppState {
        wx_config: cfg.wechat.clone(),
        wx_client,
        sessions,
    });

    let router = hermess_web::server::build_router(state);
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("hermess-webd listening on http://{}", addr);

    // 优雅关闭
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down...");
        })
        .await?;

    // ── 保存进化状态 ──────────────────────────────────────
    if let Err(e) = evolution_handle.save_to_file(".hermes_web_evolution.json") {
        tracing::warn!(error = %e, "Failed to save evolution state");
    }

    Ok(())
}
