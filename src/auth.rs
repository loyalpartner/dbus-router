//! D-Bus authentication passthrough
//!
//! D-Bus authentication protocol:
//! 1. Client sends \0 byte (carries credentials via SCM_CREDENTIALS)
//! 2. Text-based protocol with \r\n line endings
//! 3. Client may send NEGOTIATE_UNIX_FD to request FD passing capability
//! 4. Client sends "BEGIN\r\n" to signal auth completion

use anyhow::{bail, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const AUTH_BUFFER_SIZE: usize = 4096;

/// Passthrough D-Bus authentication between client and bus.
///
/// Forwards authentication messages until BEGIN is seen from the client,
/// then returns control for message forwarding phase.
///
/// NOTE: We track commands that expect responses (NEGOTIATE_UNIX_FD) and wait
/// for those responses before completing. This prevents the response from being
/// misinterpreted as message data.
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
    tracing::trace!("Forwarded null byte to bus");

    // Step 2: Forward authentication lines until BEGIN
    let mut client_buf = vec![0u8; AUTH_BUFFER_SIZE];
    let mut bus_buf = vec![0u8; AUTH_BUFFER_SIZE];

    // Track pending commands that expect responses
    let mut pending_responses: u32 = 0;
    let mut begin_received = false;

    loop {
        // If we've received BEGIN and all responses are done, we're finished
        if begin_received && pending_responses == 0 {
            tracing::debug!("Auth phase complete");
            return Ok(());
        }

        tokio::select! {
            // Read from client (only if we haven't seen BEGIN yet)
            result = read_auth_line(client, &mut client_buf), if !begin_received => {
                let line = result?;
                if line.is_empty() {
                    bail!("Client disconnected during auth");
                }

                tracing::trace!(line = %String::from_utf8_lossy(&line), "Client -> Bus");

                // Track commands that expect responses
                if is_negotiate_unix_fd(&line) {
                    pending_responses += 1;
                    tracing::trace!(pending = pending_responses, "NEGOTIATE_UNIX_FD sent");
                }

                bus.write_all(&line).await?;

                // Check if this is BEGIN (end of auth phase from client side)
                if is_begin_line(&line) {
                    begin_received = true;
                    tracing::trace!(pending = pending_responses, "BEGIN received");
                    // Don't return yet if we have pending responses
                }
            }

            // Read from bus
            result = read_auth_line(bus, &mut bus_buf) => {
                let line = result?;
                if line.is_empty() {
                    bail!("Bus disconnected during auth");
                }

                tracing::trace!(line = %String::from_utf8_lossy(&line), "Bus -> Client");

                // Track responses
                if is_agree_unix_fd(&line) || is_error_line(&line) {
                    pending_responses = pending_responses.saturating_sub(1);
                    tracing::trace!(pending = pending_responses, "Response received");
                }

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

/// Check if a line is NEGOTIATE_UNIX_FD command.
fn is_negotiate_unix_fd(line: &[u8]) -> bool {
    line.starts_with(b"NEGOTIATE_UNIX_FD")
}

/// Check if a line is AGREE_UNIX_FD response.
fn is_agree_unix_fd(line: &[u8]) -> bool {
    line.starts_with(b"AGREE_UNIX_FD")
}

/// Check if a line is an ERROR response.
fn is_error_line(line: &[u8]) -> bool {
    line.starts_with(b"ERROR")
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

    #[test]
    fn test_is_negotiate_unix_fd() {
        assert!(is_negotiate_unix_fd(b"NEGOTIATE_UNIX_FD\r\n"));
        assert!(!is_negotiate_unix_fd(b"BEGIN\r\n"));
    }

    #[test]
    fn test_is_agree_unix_fd() {
        assert!(is_agree_unix_fd(b"AGREE_UNIX_FD\r\n"));
        assert!(!is_agree_unix_fd(b"ERROR\r\n"));
    }

    #[test]
    fn test_is_error_line() {
        assert!(is_error_line(b"ERROR\r\n"));
        assert!(is_error_line(b"ERROR something\r\n"));
        assert!(!is_error_line(b"OK\r\n"));
    }
}
