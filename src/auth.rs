//! D-Bus authentication passthrough
//!
//! D-Bus authentication protocol:
//! 1. Client sends \0 byte (carries credentials via SCM_CREDENTIALS)
//! 2. Text-based protocol with \r\n line endings
//! 3. Client sends "BEGIN\r\n" to signal auth completion

use anyhow::{bail, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const AUTH_BUFFER_SIZE: usize = 4096;

/// Passthrough D-Bus authentication between client and bus.
///
/// Forwards authentication messages until BEGIN is seen from the client,
/// then returns control for message forwarding phase.
pub async fn auth_passthrough(client: &mut UnixStream, bus: &mut UnixStream) -> Result<()> {
    // Step 1: Forward the initial null byte from client to bus
    let mut null_byte = [0u8; 1];
    let n = client.read(&mut null_byte).await?;
    if n == 0 {
        bail!("Client disconnected before sending null byte");
    }
    if null_byte[0] != 0 {
        bail!("Expected null byte, got: {}", null_byte[0]);
    }
    bus.write_all(&null_byte).await?;
    tracing::debug!("Forwarded null byte to bus");

    // Step 2: Forward authentication lines until BEGIN
    let mut client_buf = vec![0u8; AUTH_BUFFER_SIZE];
    let mut bus_buf = vec![0u8; AUTH_BUFFER_SIZE];

    loop {
        tokio::select! {
            // Read from client
            result = read_auth_line(client, &mut client_buf) => {
                let line = result?;
                if line.is_empty() {
                    bail!("Client disconnected during auth");
                }

                tracing::debug!(line = %String::from_utf8_lossy(&line), "Client -> Bus");
                bus.write_all(&line).await?;

                // Check if this is BEGIN (end of auth phase)
                if is_begin_line(&line) {
                    tracing::debug!("Auth phase complete");
                    return Ok(());
                }
            }

            // Read from bus
            result = read_auth_line(bus, &mut bus_buf) => {
                let line = result?;
                if line.is_empty() {
                    bail!("Bus disconnected during auth");
                }

                tracing::debug!(line = %String::from_utf8_lossy(&line), "Bus -> Client");
                client.write_all(&line).await?;
            }
        }
    }
}

/// Read a line ending with \r\n from the stream.
/// Returns the complete line including \r\n.
async fn read_auth_line(stream: &mut UnixStream, buf: &mut [u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= buf.len() {
            bail!("Auth line too long");
        }

        let n = stream.read(&mut buf[pos..pos + 1]).await?;
        if n == 0 {
            return Ok(Vec::new()); // EOF
        }

        result.push(buf[pos]);
        pos += 1;

        // Check for \r\n ending
        if result.len() >= 2
            && result[result.len() - 2] == b'\r'
            && result[result.len() - 1] == b'\n'
        {
            return Ok(result);
        }
    }
}

/// Check if a line is the BEGIN command.
fn is_begin_line(line: &[u8]) -> bool {
    line.starts_with(b"BEGIN")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_begin_line() {
        assert!(is_begin_line(b"BEGIN\r\n"));
        assert!(is_begin_line(b"BEGIN"));
        assert!(!is_begin_line(b"AUTH EXTERNAL\r\n"));
        assert!(!is_begin_line(b"OK\r\n"));
    }
}
