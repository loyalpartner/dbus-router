//! Connection helpers for D-Bus Unix sockets.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

/// Parsed Unix socket address.
#[derive(Debug, Clone)]
enum UnixAddress {
    /// Filesystem path socket.
    Path(PathBuf),
    /// Abstract socket (Linux only).
    Abstract(String),
}

/// Connect to a D-Bus address.
/// Supports "unix:path=/path/to/socket" and "unix:abstract=name" formats.
pub async fn connect_dbus(addr: &str) -> Result<UnixStream> {
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
    Err(Error::Protocol(
        "Abstract sockets are only supported on Linux".to_string(),
    ))
}

/// Parse a D-Bus address string to extract the Unix socket address.
/// Supports formats:
/// - unix:path=/path/to/socket
/// - unix:abstract=name
fn parse_unix_address(addr: &str) -> Result<UnixAddress> {
    if !addr.starts_with("unix:") {
        return Err(Error::Protocol(format!(
            "Only unix: addresses are supported, got: {}",
            addr
        )));
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

    Err(Error::Protocol(format!(
        "No path= or abstract= found in address: {}",
        addr
    )))
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
