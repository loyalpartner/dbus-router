//! Test binary for dbus-router library

use clap::Parser;
use dbus_router::{Config, Result, Router};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "dbus-router")]
#[command(about = "A dual-upstream D-Bus router (test binary)")]
struct Args {
    /// Path to the Unix socket to listen on
    #[arg(long)]
    listen: PathBuf,

    /// Host session bus address (e.g., unix:path=/run/user/1000/bus)
    #[arg(long)]
    host: String,

    /// Sandbox session bus address (default target)
    #[arg(long)]
    sandbox: String,

    /// Path to the configuration file (TOML format)
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Load configuration if provided, otherwise use empty config
    let cfg = match &args.config {
        Some(path) => {
            tracing::info!(path = %path.display(), "Loading configuration");
            Config::load(path)?
        }
        None => {
            tracing::info!("No config file specified, all traffic routes to sandbox bus");
            Config::default()
        }
    };

    tracing::info!(
        listen = %args.listen.display(),
        host = %args.host,
        sandbox = %args.sandbox,
        host_routes = cfg.host_routes.len(),
        "Starting dbus-router"
    );

    let router = Router::new(args.listen, args.host, args.sandbox, cfg);
    router.run().await
}
