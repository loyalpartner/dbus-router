//! Router core: listen for connections and spawn sessions

use crate::config::Config;
use crate::error::Result;
use crate::session::Session;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};

/// The main router that listens for client connections.
pub struct Router {
    /// Path to the Unix socket to listen on
    listen_path: PathBuf,
    /// Host session bus address
    host_addr: String,
    /// Sandbox session bus address (default target)
    sandbox_addr: String,
    /// Routing configuration
    config: Arc<Config>,
}

impl Router {
    /// Create a new router.
    pub fn new(
        listen_path: PathBuf,
        host_addr: String,
        sandbox_addr: String,
        config: Config,
    ) -> Self {
        Self {
            listen_path,
            host_addr,
            sandbox_addr,
            config: Arc::new(config),
        }
    }

    /// Run the router, accepting connections and spawning sessions.
    pub async fn run(&self) -> Result<()> {
        // Remove existing socket if present
        if self.listen_path.exists() {
            std::fs::remove_file(&self.listen_path)?;
        }

        let listener = UnixListener::bind(&self.listen_path)?;
        tracing::info!(path = %self.listen_path.display(), "Listening for connections");

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tracing::info!("New client connection");
                    tokio::spawn(serve_client(
                        stream,
                        self.host_addr.clone(),
                        self.sandbox_addr.clone(),
                        Arc::clone(&self.config),
                    ));
                }
                // One client failing to connect must not take the router
                // down; keep accepting.
                Err(e) => tracing::error!(error = %e, "Failed to accept connection"),
            }
        }
    }
}

/// Serve one accepted client until it disconnects. Errors are logged and
/// end this task only - the accept loop above keeps running.
async fn serve_client(
    stream: UnixStream,
    host_addr: String,
    sandbox_addr: String,
    config: Arc<Config>,
) {
    let session = match Session::new(stream, &host_addr, &sandbox_addr, config).await {
        Ok(session) => session,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create session");
            return;
        }
    };

    if let Err(e) = session.run().await {
        tracing::error!(
            error = %e,
            host = %host_addr,
            sandbox = %sandbox_addr,
            "Session error"
        );
    }
}
