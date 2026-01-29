//! Client session handling with dual upstream connections and routing

use crate::auth;
use crate::config::Config;
use crate::message::{self, read_message, Message, MessageType};
use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    /// Client process executable path (for sandbox export permission check)
    client_exe_path: Option<PathBuf>,
    /// Services exported by this client to the host bus
    exported_services: HashSet<String>,
    /// Track incoming calls from host bus (serial -> source bus)
    incoming_calls: HashMap<u32, Bus>,
}

/// Get the executable path of a peer process from a Unix socket.
#[cfg(target_os = "linux")]
fn get_peer_exe_path(stream: &UnixStream) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;

    let fd = stream.as_raw_fd();

    // Get peer credentials using SO_PEERCRED
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };

    if ret != 0 {
        tracing::debug!(
            "Failed to get peer credentials: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }

    let pid = ucred.pid;
    if pid <= 0 {
        return None;
    }

    // Read /proc/{pid}/exe symlink
    let exe_path = format!("/proc/{}/exe", pid);
    match std::fs::read_link(&exe_path) {
        Ok(path) => {
            tracing::debug!(pid = pid, exe = %path.display(), "Got peer exe path");
            Some(path)
        }
        Err(e) => {
            tracing::debug!(pid = pid, error = %e, "Failed to read exe path");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn get_peer_exe_path(_stream: &UnixStream) -> Option<PathBuf> {
    None
}

impl Session {
    /// Create a new session with connections to both upstream buses.
    pub async fn new(
        client: UnixStream,
        host_addr: &str,
        sandbox_addr: &str,
        config: Arc<Config>,
    ) -> Result<Self> {
        let client_exe_path = get_peer_exe_path(&client);
        let host_bus = connect_dbus(host_addr).await?;
        let sandbox_bus = connect_dbus(sandbox_addr).await?;

        Ok(Self {
            client,
            host_bus,
            sandbox_bus,
            config,
            pending_calls: HashMap::new(),
            client_exe_path,
            exported_services: HashSet::new(),
            incoming_calls: HashMap::new(),
        })
    }

    /// Run the session: authenticate with both buses, then forward messages.
    pub async fn run(mut self) -> Result<()> {
        // Check if this is a hostpass client
        let is_hostpass = self
            .client_exe_path
            .as_ref()
            .map(|p| self.config.has_hostpass(p))
            .unwrap_or(false);

        // Phase 1: Auth passthrough with sandbox bus
        tracing::debug!("Starting auth phase with sandbox bus");
        auth::auth_passthrough(&mut self.client, &mut self.sandbox_bus).await?;
        tracing::info!("Auth with sandbox bus completed");

        // Phase 1b: Also authenticate with host bus (using same credentials)
        // For hostpass clients, skip Hello() - their Hello() will be forwarded to host bus
        // For non-hostpass clients, send Hello() so host_routes messages can be routed
        tracing::debug!("Starting auth phase with host bus");
        self.auth_host_bus(is_hostpass).await?;
        tracing::info!("Auth with host bus completed, starting message forwarding");

        // Phase 2: Message forwarding with routing
        self.forward_loop().await
    }

    /// Authenticate with the host bus.
    /// The host bus needs its own auth handshake.
    /// If `skip_hello` is true, skip the Hello() call (for hostpass clients whose
    /// Hello() will be forwarded to the host bus).
    async fn auth_host_bus(&mut self, skip_hello: bool) -> Result<()> {
        // Send null byte and EXTERNAL auth with hex-encoded UID
        self.host_bus.write_all(&[0]).await?;
        let uid = unsafe { libc::getuid() };
        let uid_hex: String = uid
            .to_string()
            .bytes()
            .map(|b| format!("{:02x}", b))
            .collect();
        self.host_bus
            .write_all(format!("AUTH EXTERNAL {}\r\n", uid_hex).as_bytes())
            .await?;

        // Read auth response
        let response = read_auth_line(&mut self.host_bus).await?;
        if !response.starts_with("OK") {
            bail!("Host bus auth failed: {}", response.trim());
        }

        // Negotiate UNIX FD passing
        self.host_bus.write_all(b"NEGOTIATE_UNIX_FD\r\n").await?;
        let response = read_auth_line(&mut self.host_bus).await?;
        tracing::debug!(response = %response.trim(), "Host bus NEGOTIATE_UNIX_FD response");

        // Send BEGIN to complete SASL auth
        self.host_bus.write_all(b"BEGIN\r\n").await?;

        if skip_hello {
            tracing::debug!("Skipping Hello() for hostpass client");
            Ok(())
        } else {
            self.send_host_hello().await
        }
    }

    /// Send Hello() method call to host bus and read the response.
    /// This registers the router's connection with the host bus daemon.
    async fn send_host_hello(&mut self) -> Result<()> {
        use zvariant::{serialized::Context, to_bytes, ObjectPath, Value, LE};

        // Build header fields array for Hello() call
        let path = ObjectPath::try_from("/org/freedesktop/DBus").unwrap();
        let fields: Vec<(u8, Value)> = vec![
            (1, Value::ObjectPath(path)),                   // PATH
            (2, Value::Str("org.freedesktop.DBus".into())), // INTERFACE
            (3, Value::Str("Hello".into())),                // MEMBER
            (6, Value::Str("org.freedesktop.DBus".into())), // DESTINATION
        ];

        let ctxt = Context::new_dbus(LE, 12);
        let fields_encoded = to_bytes(ctxt, &fields)?;
        let array_len = fields_encoded.len() - 4; // Exclude 4-byte length prefix

        // Calculate padding to 8-byte boundary
        let header_end = 16 + array_len;
        let padding = (8 - (header_end % 8)) % 8;

        // Build D-Bus message: fixed header + fields + padding
        let mut msg = Vec::with_capacity(16 + array_len + padding);
        msg.extend_from_slice(&[b'l', 1, 0, 1]); // endian, method_call, flags, version
        msg.extend_from_slice(&0u32.to_le_bytes()); // body length
        msg.extend_from_slice(&1u32.to_le_bytes()); // serial
        msg.extend_from_slice(&fields_encoded);
        msg.resize(msg.len() + padding, 0);

        self.host_bus.write_all(&msg).await?;

        // Read and validate response.
        // After Hello(), the bus daemon sends:
        // 1. MethodReturn with our unique name
        // 2. NameAcquired signal for that name
        // We must consume both to prevent the signal from being forwarded to client,
        // which would cause "Unexpected message Signal" errors in strict clients like zbus.
        //
        // First, read the MethodReturn
        match message::read_message(&mut self.host_bus).await? {
            Some(resp) if resp.header.msg_type == MessageType::MethodReturn => {
                tracing::debug!("Host bus Hello() MethodReturn received");
            }
            Some(resp) if resp.header.msg_type == MessageType::Error => {
                bail!("Host bus Hello() failed with error")
            }
            Some(resp) => bail!("Unexpected response to Hello(): {:?}", resp.header.msg_type),
            None => bail!("Host bus disconnected after Hello()"),
        }

        // Then, read the NameAcquired signal
        match message::read_message(&mut self.host_bus).await? {
            Some(resp) if resp.header.msg_type == MessageType::Signal => {
                tracing::debug!(
                    interface = ?resp.header.interface,
                    member = ?resp.header.member,
                    "Consumed NameAcquired signal after Hello()"
                );
            }
            Some(resp) => {
                tracing::warn!(
                    msg_type = ?resp.header.msg_type,
                    "Unexpected second message after Hello(), expected Signal"
                );
            }
            None => bail!("Host bus disconnected after Hello()"),
        }

        tracing::debug!("Host bus Hello() completed");
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
                            // Check if this is a reply to an incoming host call
                            if matches!(msg.header.msg_type, MessageType::MethodReturn | MessageType::Error) {
                                if let Some(reply_serial) = msg.header.reply_serial {
                                    if let Some(Bus::Host) = self.incoming_calls.remove(&reply_serial) {
                                        tracing::debug!(
                                            reply_serial = reply_serial,
                                            "Routing reply to host bus"
                                        );
                                        host_write.write_all(&msg.raw).await?;
                                        continue;
                                    }
                                }
                            }

                            let target = route_request(&self.config, &msg, self.client_exe_path.as_deref());

                            // Track RequestName calls to know which services this client exports
                            if msg.is_request_name() && target == Bus::Host {
                                if let Some(name) = msg.extract_name_from_body() {
                                    tracing::info!(service = %name, "Client exporting service to host bus");
                                    self.exported_services.insert(name);
                                }
                            }

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
                            // Check if this is a call to an exported service
                            if msg.header.msg_type == MessageType::MethodCall {
                                if let Some(ref dest) = msg.header.destination {
                                    if self.exported_services.contains(dest) {
                                        tracing::debug!(
                                            serial = msg.header.serial,
                                            destination = %dest,
                                            "Routing host call to client (exported service)"
                                        );
                                        self.incoming_calls.insert(msg.header.serial, Bus::Host);
                                        client_write.write_all(&msg.raw).await?;
                                        continue;
                                    }
                                }
                            }

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
fn route_request(config: &Config, msg: &Message, client_exe: Option<&Path>) -> Bus {
    // Hostpass: route ALL messages from hostpass processes to host bus
    // This is required because D-Bus requires Hello() before any other operations,
    // and the host bus won't accept messages from connections that haven't called Hello()
    if let Some(exe) = client_exe {
        if config.has_hostpass(exe) {
            tracing::debug!(exe = %exe.display(), "Routing to host (hostpass)");
            return Bus::Host;
        }
    }

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

/// Read a single line from a D-Bus auth handshake (terminated by CRLF).
async fn read_auth_line(stream: &mut UnixStream) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut buf = [0u8; 256];
    let mut response = Vec::new();
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            bail!("Connection closed during auth");
        }
        response.extend_from_slice(&buf[..n]);
        if response.windows(2).any(|w| w == b"\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&response).into_owned())
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
