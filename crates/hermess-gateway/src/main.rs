use clap::Parser;

mod classifier;
mod config;
mod decision;
mod decomposer;
mod discovery;
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
    let gateway = gateway::Gateway::new(cfg).await;

    tracing::info!(addr = %listen_addr, "Hermess Gateway starting");
    tracing::info!(models = gateway.list_models().len(), "Registered models");

    let app = server::build_router(gateway);
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
