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
