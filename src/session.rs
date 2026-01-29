//! Client session handling with dual upstream connections and routing

use crate::auth;
use crate::config::Config;
use crate::message::{read_message, Message};
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// Target bus for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Host,
    Sandbox,
}

/// A client session with connections to both upstream buses.
pub struct Session {
    /// Connection from sandbox app
    client: UnixStream,
    /// Connection to host session bus
    host_bus: UnixStream,
    /// Connection to sandbox session bus (default target)
    sandbox_bus: UnixStream,
    /// Routing configuration
    config: Arc<Config>,
    /// Track which bus each outgoing serial was sent to (for routing replies)
    pending_calls: HashMap<u32, Bus>,
}

impl Session {
    /// Create a new session with connections to both upstream buses.
    pub async fn new(
        client: UnixStream,
        host_addr: &str,
        sandbox_addr: &str,
        config: Arc<Config>,
    ) -> Result<Self> {
        let host_bus = connect_dbus(host_addr).await?;
        let sandbox_bus = connect_dbus(sandbox_addr).await?;

        Ok(Self {
            client,
            host_bus,
            sandbox_bus,
            config,
            pending_calls: HashMap::new(),
        })
    }

    /// Run the session: authenticate with both buses, then forward messages.
    pub async fn run(mut self) -> Result<()> {
        // Phase 1: Auth passthrough with sandbox bus
        tracing::debug!("Starting auth phase with sandbox bus");
        auth::auth_passthrough(&mut self.client, &mut self.sandbox_bus).await?;
        tracing::info!("Auth with sandbox bus completed");

        // Phase 1b: Also authenticate with host bus (using same credentials)
        // We send a minimal auth sequence to the host bus
        tracing::debug!("Starting auth phase with host bus");
        self.auth_host_bus().await?;
        tracing::info!("Auth with host bus completed, starting message forwarding");

        // Phase 2: Message forwarding with routing
        self.forward_loop().await
    }

    /// Authenticate with the host bus.
    /// The host bus needs its own auth handshake.
    async fn auth_host_bus(&mut self) -> Result<()> {
        // Send null byte
        self.host_bus.write_all(&[0]).await?;

        // Send EXTERNAL auth with our UID
        // UID must be encoded as hex of ASCII digits (e.g., 1000 -> "31303030")
        let uid = unsafe { libc::getuid() };
        let uid_hex = uid
            .to_string()
            .as_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        let auth_cmd = format!("AUTH EXTERNAL {}\r\n", uid_hex);
        self.host_bus.write_all(auth_cmd.as_bytes()).await?;

        // Read response until we get OK
        let mut buf = vec![0u8; 256];
        let mut response = Vec::new();
        loop {
            let n = tokio::io::AsyncReadExt::read(&mut self.host_bus, &mut buf).await?;
            if n == 0 {
                bail!("Host bus disconnected during auth");
            }
            response.extend_from_slice(&buf[..n]);
            if response.windows(2).any(|w| w == b"\r\n") {
                break;
            }
        }

        let response_str = String::from_utf8_lossy(&response);
        if !response_str.starts_with("OK") {
            bail!("Host bus auth failed: {}", response_str.trim());
        }

        // Send BEGIN to finish auth
        self.host_bus.write_all(b"BEGIN\r\n").await?;

        Ok(())
    }

    /// Forward messages between client and upstream buses with routing.
    async fn forward_loop(mut self) -> Result<()> {
        let (client_read, mut client_write) = self.client.split();
        let (host_read, mut host_write) = self.host_bus.split();
        let (sandbox_read, mut sandbox_write) = self.sandbox_bus.split();

        let mut client_read = tokio::io::BufReader::new(client_read);
        let mut host_read = tokio::io::BufReader::new(host_read);
        let mut sandbox_read = tokio::io::BufReader::new(sandbox_read);

        loop {
            tokio::select! {
                // Read from client and route to appropriate bus
                result = read_message(&mut client_read) => {
                    match result {
                        Ok(Some(msg)) => {
                            let target = route_request(&self.config, &msg);
                            self.pending_calls.insert(msg.header.serial, target);

                            tracing::debug!(
                                serial = msg.header.serial,
                                destination = ?msg.header.destination,
                                target = ?target,
                                msg_type = ?msg.header.msg_type,
                                "Routing message"
                            );

                            match target {
                                Bus::Host => host_write.write_all(&msg.raw).await?,
                                Bus::Sandbox => sandbox_write.write_all(&msg.raw).await?,
                            }
                        }
                        Ok(None) => {
                            tracing::debug!("Client disconnected");
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Error reading from client");
                            return Ok(());
                        }
                    }
                }

                // Read from host bus and forward to client
                result = read_message(&mut host_read) => {
                    match result {
                        Ok(Some(msg)) => {
                            tracing::debug!(
                                serial = msg.header.serial,
                                reply_serial = ?msg.header.reply_serial,
                                "Host -> Client"
                            );
                            client_write.write_all(&msg.raw).await?;
                        }
                        Ok(None) => {
                            tracing::debug!("Host bus disconnected");
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Error reading from host bus");
                            return Ok(());
                        }
                    }
                }

                // Read from sandbox bus and forward to client
                result = read_message(&mut sandbox_read) => {
                    match result {
                        Ok(Some(msg)) => {
                            tracing::debug!(
                                serial = msg.header.serial,
                                reply_serial = ?msg.header.reply_serial,
                                "Sandbox -> Client"
                            );
                            client_write.write_all(&msg.raw).await?;
                        }
                        Ok(None) => {
                            tracing::debug!("Sandbox bus disconnected");
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Error reading from sandbox bus");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

/// Determine which bus to route a request to based on destination.
#[allow(dead_code)] // pending_calls is tracked but not used for response routing yet
fn route_request(config: &Config, msg: &Message) -> Bus {
    // Method calls and signals with a destination are routed based on config
    if let Some(ref dest) = msg.header.destination {
        if config.should_route_to_host(dest) {
            return Bus::Host;
        }
    }

    // Default: route to sandbox bus
    Bus::Sandbox
}

/// Parsed Unix socket address.
#[derive(Debug, Clone)]
enum UnixAddress {
    /// Filesystem path socket
    Path(std::path::PathBuf),
    /// Abstract socket (Linux only)
    Abstract(String),
}

/// Connect to a D-Bus address.
/// Supports "unix:path=/path/to/socket" and "unix:abstract=name" formats.
async fn connect_dbus(addr: &str) -> Result<UnixStream> {
    let unix_addr = parse_unix_address(addr)?;
    match unix_addr {
        UnixAddress::Path(path) => {
            tracing::debug!(path = %path.display(), "Connecting to D-Bus (path)");
            let stream = UnixStream::connect(&path).await?;
            Ok(stream)
        }
        UnixAddress::Abstract(name) => {
            tracing::debug!(name = %name, "Connecting to D-Bus (abstract)");
            let stream = connect_abstract(&name).await?;
            Ok(stream)
        }
    }
}

/// Connect to an abstract Unix socket (Linux only).
#[cfg(target_os = "linux")]
async fn connect_abstract(name: &str) -> Result<UnixStream> {
    use std::os::linux::net::SocketAddrExt;

    let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
    let std_stream = std::os::unix::net::UnixStream::connect_addr(&addr)?;
    std_stream.set_nonblocking(true)?;
    let stream = UnixStream::from_std(std_stream)?;
    Ok(stream)
}

#[cfg(not(target_os = "linux"))]
async fn connect_abstract(_name: &str) -> Result<UnixStream> {
    bail!("Abstract sockets are only supported on Linux");
}

/// Parse a D-Bus address string to extract the Unix socket address.
/// Supports formats:
/// - unix:path=/path/to/socket
/// - unix:abstract=name
fn parse_unix_address(addr: &str) -> Result<UnixAddress> {
    if !addr.starts_with("unix:") {
        bail!("Only unix: addresses are supported, got: {}", addr);
    }

    let parts = &addr[5..]; // Skip "unix:"

    for part in parts.split(',') {
        if let Some(path) = part.strip_prefix("path=") {
            return Ok(UnixAddress::Path(Path::new(path).to_path_buf()));
        }
        if let Some(name) = part.strip_prefix("abstract=") {
            return Ok(UnixAddress::Abstract(name.to_string()));
        }
    }

    bail!("No path= or abstract= found in address: {}", addr);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unix_address_path() {
        let addr = parse_unix_address("unix:path=/run/user/1000/bus").unwrap();
        assert!(matches!(addr, UnixAddress::Path(p) if p == Path::new("/run/user/1000/bus")));

        let addr = parse_unix_address("unix:path=/tmp/test.sock,guid=abc123").unwrap();
        assert!(matches!(addr, UnixAddress::Path(p) if p == Path::new("/tmp/test.sock")));
    }

    #[test]
    fn test_parse_unix_address_abstract() {
        let addr = parse_unix_address("unix:abstract=/tmp/dbus-test").unwrap();
        assert!(matches!(addr, UnixAddress::Abstract(n) if n == "/tmp/dbus-test"));

        let addr = parse_unix_address("unix:abstract=dbus-session,guid=abc").unwrap();
        assert!(matches!(addr, UnixAddress::Abstract(n) if n == "dbus-session"));
    }

    #[test]
    fn test_parse_invalid_address() {
        assert!(parse_unix_address("tcp:host=localhost").is_err());
        assert!(parse_unix_address("unix:guid=abc123").is_err()); // no path or abstract
    }
}
