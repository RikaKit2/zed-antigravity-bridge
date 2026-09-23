mod adapter;
mod auth;
mod server;
mod types;

use auth::TokenManager;
use clap::Parser;
use server::{create_router, AppState};
use std::net::SocketAddr;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "zed-antigravity-bridge")]
#[command(about = "High-performance OpenAI-to-Antigravity FIM completions bridge for Zed IDE")]
struct Cli {
    #[arg(short, long, env = "BRIDGE_PORT", default_value_t = 8080)]
    port: u16,

    #[arg(long, env = "BRIDGE_HOST", default_value = "127.0.0.1")]
    host: String,

    #[arg(short, long, env = "DEFAULT_MODEL", default_value = "tab_flash_lite_preview")]
    model: String,

    #[arg(
        long,
        env = "ANTIGRAVITY_BASE_URL",
        default_value = "https://daily-cloudcode-pa.googleapis.com"
    )]
    upstream: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zed_antigravity_bridge=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    info!(
        "Starting Zed Antigravity Bridge on {}:{} with model {} (upstream: {})",
        cli.host, cli.port, cli.model, cli.upstream
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .build()?;

    let token_manager = TokenManager::new();

    let state = AppState {
        client,
        token_manager,
        default_model: cli.model,
        upstream_base_url: cli.upstream,
    };

    let app = create_router(state);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    info!("Bridge listening on http://{}", addr);
    info!("Zed endpoint: http://{}/v1/completions", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Zed Antigravity Bridge shut down cleanly");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
