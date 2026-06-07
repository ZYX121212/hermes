use std::net::SocketAddr;

use clap::Parser;

mod classifier;
mod config;
mod decision;
mod decomposer;
mod discovery;

mod feedback;
mod gateway;
mod merger;
mod metrics;
mod models;
mod registry;
mod server;
mod shg;
mod skills;
mod strategy;

#[derive(Parser)]
#[command(name = "hermes-gateway", about = "Hermess LLM Routing Gateway")]
struct Cli {
    #[arg(short, long, default_value = "config/gateway.toml")]
    config: String,
    /// Hermess instance name — feedback state is scoped per instance.
    #[arg(long, default_value = "default")]
    name: String,
    /// Start with clean memory, ignoring any saved feedback from previous runs.
    #[arg(long, default_value_t = false)]
    fresh: bool,
}

fn init_tracing() {
    use std::env;
    let use_json = env::var("LOG_FORMAT").map(|v| v == "json").unwrap_or(false);
    let builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    if use_json {
        builder.json().init();
    } else {
        builder.init();
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    tracing::info!("Received shutdown signal, draining connections...");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let cfg = config::GatewayConfig::from_file(&cli.config)?;

    let listen_addr = cfg.gateway.listen.clone();
    let instance_name = cli.name;
    let gateway = gateway::Gateway::new(cfg, &instance_name, cli.fresh).await;

    tracing::info!(addr = %listen_addr, instance = %instance_name, "Hermess Gateway starting");
    tracing::info!(models = gateway.list_models().len(), "Registered models");

    let feedback_file = feedback_file_path(&instance_name);
    let feedback = std::sync::Arc::clone(&gateway.feedback);
    let app = server::build_router(gateway);
    let listener = bind_with_reuse(&listen_addr).await?;
    tracing::info!("Gateway listening on {listen_addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    if let Err(e) = feedback.save_to_file(&feedback_file) {
        tracing::error!(error = %e, path = %feedback_file, "Failed to save feedback state");
    } else {
        tracing::info!(path = %feedback_file, "Feedback state saved");
    }
    tracing::info!("Gateway shut down cleanly");
    Ok(())
}

/// Bind with SO_REUSEADDR so restarts don't fail on TIME_WAIT.
async fn bind_with_reuse(addr: &str) -> anyhow::Result<tokio::net::TcpListener> {
    use tokio::net::TcpSocket;
    let parsed: SocketAddr = addr.parse()?;
    let socket = if parsed.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(parsed)?;
    Ok(socket.listen(4096)?)
}

fn feedback_file_path(instance_name: &str) -> String {
    format!(".hermes_feedback_{instance_name}.json")
}
