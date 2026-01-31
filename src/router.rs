//! Router core: listen for connections and spawn sessions

use crate::config::Config;
use crate::error::Result;
use crate::session::Session;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixListener;

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

                    let host_addr = self.host_addr.clone();
                    let sandbox_addr = self.sandbox_addr.clone();
                    let config = Arc::clone(&self.config);

                    tokio::spawn(async move {
                        match Session::new(stream, &host_addr, &sandbox_addr, config).await {
                            Ok(session) => {
                                if let Err(e) = session.run().await {
                                    tracing::error!(error = %e, "Session error");
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to create session");
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to accept connection");
                }
            }
        }
    }
}
