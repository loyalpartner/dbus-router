//! Client session handling with dual upstream connections and routing

use crate::auth;
use crate::config::Config;
use crate::dbus_daemon::{
    build_list_names_response, merge_list_names, needs_request_rewrite, needs_response_rewrite,
    parse_string_array, rewrite_match_rule_body, rewrite_name_owner_changed,
    rewrite_single_name_response, rewrite_string_array_response, rewrite_unique_name_request,
    signal_needs_rewrite,
};
use crate::fake_name::get_bus_from_fake_name;
use crate::message::{self, read_message, Message, MessageType};
use crate::message_rewrite::{parse_match_rule, rewrite_message_header, RewriteDirection};
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

/// Log a message with all relevant fields for debugging.
fn log_message(msg: &Message, direction: &str, target: Option<Bus>) {
    let target_str = match target {
        Some(Bus::Host) => " -> Host",
        Some(Bus::Sandbox) => " -> Sandbox",
        None => "",
    };

    tracing::trace!(
        direction = direction,
        target = target_str,
        msg_type = ?msg.header.msg_type,
        serial = msg.header.serial,
        reply_serial = ?msg.header.reply_serial,
        sender = ?msg.header.sender,
        destination = ?msg.header.destination,
        interface = ?msg.header.interface,
        member = ?msg.header.member,
        body_len = msg.header.body_len,
        "{}",
        format_message_summary(msg)
    );
}

/// Format a one-line summary of the message for logging.
fn format_message_summary(msg: &Message) -> String {
    let type_str = match msg.header.msg_type {
        MessageType::MethodCall => "CALL",
        MessageType::MethodReturn => "REPLY",
        MessageType::Error => "ERROR",
        MessageType::Signal => "SIGNAL",
        MessageType::Invalid => "INVALID",
    };

    let interface = msg.header.interface.as_deref().unwrap_or("-");
    let member = msg.header.member.as_deref().unwrap_or("-");
    let dest = msg.header.destination.as_deref().unwrap_or("-");
    let sender = msg.header.sender.as_deref().unwrap_or("-");

    format!(
        "{} {}.{} [{}->{}] serial={}",
        type_str, interface, member, sender, dest, msg.header.serial
    )
}

/// Prepare a message from an upstream bus for forwarding to the client.
/// Rewrites sender header and body as needed.
fn prepare_message_for_client(
    msg: &Message,
    source_bus: Bus,
    pending_calls: &HashMap<u32, PendingCallInfo>,
) -> Vec<u8> {
    let mut msg_for_client = msg.clone();

    // Log message context for debugging header rewriting issues
    tracing::trace!(
        msg_type = ?msg.header.msg_type,
        sender = ?msg.header.sender,
        destination = ?msg.header.destination,
        interface = ?msg.header.interface,
        member = ?msg.header.member,
        source_bus = ?source_bus,
        "Preparing message for client"
    );

    // Rewrite sender header to add bus prefix
    if let Err(e) =
        rewrite_message_header(&mut msg_for_client, RewriteDirection::ToClient, source_bus)
    {
        tracing::warn!(
            error = %e,
            msg_type = ?msg.header.msg_type,
            sender = ?msg.header.sender,
            destination = ?msg.header.destination,
            interface = ?msg.header.interface,
            member = ?msg.header.member,
            "Failed to rewrite sender header"
        );
    }

    // Handle body rewriting based on message type
    match msg_for_client.header.msg_type {
        MessageType::MethodReturn => {
            rewrite_method_return_body(&msg_for_client, source_bus, pending_calls)
        }
        MessageType::Signal => rewrite_signal_body(&msg_for_client, source_bus),
        _ => msg_for_client.raw,
    }
}

/// Rewrite method return body if it contains unique names that need prefixing.
fn rewrite_method_return_body(
    msg: &Message,
    source_bus: Bus,
    pending_calls: &HashMap<u32, PendingCallInfo>,
) -> Vec<u8> {
    let Some(reply_serial) = msg.header.reply_serial else {
        return msg.raw.clone();
    };

    let Some(call_info) = pending_calls.get(&reply_serial) else {
        return msg.raw.clone();
    };

    if call_info.bus != source_bus {
        return msg.raw.clone();
    }

    let Some(ref member) = call_info.member else {
        return msg.raw.clone();
    };

    if !needs_response_rewrite(member) {
        return msg.raw.clone();
    }

    // ListQueuedOwners returns an array of unique names
    let result = if member == "ListQueuedOwners" {
        rewrite_string_array_response(msg, source_bus)
    } else {
        rewrite_single_name_response(msg, source_bus)
    };

    match result {
        Ok(rewritten) => {
            tracing::trace!(member = member, bus = ?source_bus, "Rewrote response body");
            rewritten
        }
        Err(e) => {
            tracing::warn!(member = member, error = %e, "Failed to rewrite response body");
            msg.raw.clone()
        }
    }
}

/// Rewrite signal body if it contains unique names that need prefixing.
fn rewrite_signal_body(msg: &Message, source_bus: Bus) -> Vec<u8> {
    let Some(ref member) = msg.header.member else {
        return msg.raw.clone();
    };

    if !signal_needs_rewrite(member) {
        return msg.raw.clone();
    }

    match rewrite_name_owner_changed(msg, source_bus) {
        Ok(rewritten) => {
            tracing::trace!(member = member, bus = ?source_bus, "Rewrote signal body");
            rewritten
        }
        Err(e) => {
            tracing::warn!(member = member, error = %e, "Failed to rewrite signal body");
            msg.raw.clone()
        }
    }
}

/// Result of processing a merge response.
enum MergeResult {
    /// First response stored, waiting for second
    Stored,
    /// Second response received, returns merged message bytes
    Complete(Vec<u8>),
}

/// Process a ListNames/ListActivatableNames merge response.
/// Returns `Some(MergeResult)` if the message was a merge response, `None` otherwise.
fn process_merge_response(
    msg: &Message,
    source_bus: Bus,
    pending_merges: &mut HashMap<u32, PendingMerge>,
) -> Option<MergeResult> {
    let reply_serial = msg.header.reply_serial?;
    let pending = pending_merges.get_mut(&reply_serial)?;

    let names = parse_string_array(msg).unwrap_or_default();
    tracing::trace!(
        serial = reply_serial,
        count = names.len(),
        bus = ?source_bus,
        "Received ListNames response"
    );

    let Some((first_bus, first_names)) = pending.first_response.take() else {
        // First response - store it
        pending.first_response = Some((source_bus, names));
        return Some(MergeResult::Stored);
    };

    // Second response - merge and build final message
    let (host_names, sandbox_names) = if first_bus == Bus::Host {
        (first_names, names)
    } else {
        (names, first_names)
    };

    let merged = merge_list_names(host_names, sandbox_names);
    tracing::trace!(
        serial = reply_serial,
        count = merged.len(),
        "Merged ListNames response"
    );

    let pending = pending_merges.remove(&reply_serial).unwrap();
    let response = match build_list_names_response(&pending.original_request, merged) {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build merged response");
            msg.raw.clone()
        }
    };

    Some(MergeResult::Complete(response))
}

/// Pending merge state for ListNames/ListActivatableNames
#[derive(Debug)]
struct PendingMerge {
    /// The original request message (for building response)
    original_request: Message,
    /// First response received (None if waiting for first)
    first_response: Option<(Bus, Vec<String>)>,
}

/// Info about a pending call (for response rewriting)
#[derive(Debug, Clone)]
struct PendingCallInfo {
    /// Which bus the call was sent to
    bus: Bus,
    /// The member (method name) of the call (for response rewriting)
    member: Option<String>,
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
    pending_calls: HashMap<u32, PendingCallInfo>,
    /// Client process executable path (for sandbox export permission check)
    client_exe_path: Option<PathBuf>,
    /// Services exported by this client to the host bus
    exported_services: HashSet<String>,
    /// Services registered by this client on the sandbox bus
    sandbox_services: HashSet<String>,
    /// Track incoming calls from upstream buses (serial -> source bus)
    incoming_calls: HashMap<u32, Bus>,
    /// Pending ListNames/ListActivatableNames merges (serial -> state)
    pending_merges: HashMap<u32, PendingMerge>,
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
            sandbox_services: HashSet::new(),
            incoming_calls: HashMap::new(),
            pending_merges: HashMap::new(),
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
        // Check if this is a hostpass client (they only use host bus)
        let is_hostpass = self
            .client_exe_path
            .as_ref()
            .map(|p| self.config.has_hostpass(p))
            .unwrap_or(false);

        let (client_read, mut client_write) = self.client.split();
        let (host_read, mut host_write) = self.host_bus.split();
        let (sandbox_read, mut sandbox_write) = self.sandbox_bus.split();

        let mut client_read = tokio::io::BufReader::new(client_read);
        let mut host_read = tokio::io::BufReader::new(host_read);
        let mut sandbox_read = tokio::io::BufReader::new(sandbox_read);

        // Track if sandbox bus is still active (for hostpass clients that survive sandbox disconnect)
        let mut sandbox_active = true;

        loop {
            tokio::select! {
                biased;
                // Read from client and route to appropriate bus
                result = read_message(&mut client_read) => {
                    match result {
                        Ok(Some(msg)) => {
                            // Check if this is a reply to an incoming call from upstream bus
                            if matches!(msg.header.msg_type, MessageType::MethodReturn | MessageType::Error) {
                                if let Some(reply_serial) = msg.header.reply_serial {
                                    if let Some(source_bus) = self.incoming_calls.remove(&reply_serial) {
                                        // Rewrite destination from fake name to real name
                                        let mut msg_for_bus = msg.clone();
                                        if let Err(e) = rewrite_message_header(
                                            &mut msg_for_bus,
                                            RewriteDirection::ToUpstream,
                                            source_bus,
                                        ) {
                                            tracing::warn!(error = %e, "Failed to rewrite destination for reply");
                                        }

                                        match source_bus {
                                            Bus::Host => {
                                                log_message(&msg, "Client->Host(reply)", Some(Bus::Host));
                                                host_write.write_all(&msg_for_bus.raw).await?;
                                            }
                                            Bus::Sandbox => {
                                                log_message(&msg, "Client->Sandbox(reply)", Some(Bus::Sandbox));
                                                sandbox_write.write_all(&msg_for_bus.raw).await?;
                                            }
                                        }
                                        continue;
                                    }
                                }
                            }

                            let decision = route_request(&self.config, &msg, self.client_exe_path.as_deref());

                            // Track RequestName calls to know which services this client exports
                            if msg.is_request_name() {
                                if let Some(name) = msg.extract_name_from_body() {
                                    match decision {
                                        RouteDecision::Single(Bus::Host) => {
                                            tracing::info!(service = %name, "Client exporting service to host bus");
                                            self.exported_services.insert(name);
                                        }
                                        RouteDecision::Single(Bus::Sandbox) => {
                                            tracing::info!(service = %name, "Client registering service on sandbox bus");
                                            self.sandbox_services.insert(name);
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            match decision {
                                RouteDecision::Single(target) => {
                                    // Prepare message for sending to upstream
                                    let mut msg_for_upstream = msg.clone();

                                    // 1. Rewrite destination header if it's a fake unique name
                                    if let Err(e) = rewrite_message_header(
                                        &mut msg_for_upstream,
                                        RewriteDirection::ToUpstream,
                                        target, // source_bus not used for ToUpstream
                                    ) {
                                        tracing::warn!(error = %e, "Failed to rewrite destination header");
                                    }

                                    // 2. Rewrite body for org.freedesktop.DBus methods
                                    let msg_to_send = if msg_for_upstream.header.destination.as_deref() == Some("org.freedesktop.DBus") {
                                        if let Some(member) = msg_for_upstream.header.member.as_deref() {
                                            if needs_request_rewrite(member) {
                                                // Rewrite unique name in body (GetConnectionCredentials etc.)
                                                match rewrite_unique_name_request(&msg_for_upstream) {
                                                    Ok((rewritten, _bus)) => {
                                                        tracing::trace!(
                                                            member = member,
                                                            "Rewrote request body to remove fake prefix"
                                                        );
                                                        rewritten
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            member = member,
                                                            error = %e,
                                                            "Failed to rewrite request body"
                                                        );
                                                        msg_for_upstream.raw.clone()
                                                    }
                                                }
                                            } else if member == "AddMatch" || member == "RemoveMatch" {
                                                // Rewrite sender in match rule
                                                match rewrite_match_rule_body(&msg_for_upstream) {
                                                    Ok(Some(rewritten)) => {
                                                        tracing::trace!(
                                                            member = member,
                                                            "Rewrote match rule sender"
                                                        );
                                                        rewritten
                                                    }
                                                    Ok(None) => msg_for_upstream.raw.clone(),
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            member = member,
                                                            error = %e,
                                                            "Failed to rewrite match rule"
                                                        );
                                                        msg_for_upstream.raw.clone()
                                                    }
                                                }
                                            } else {
                                                msg_for_upstream.raw.clone()
                                            }
                                        } else {
                                            msg_for_upstream.raw.clone()
                                        }
                                    } else {
                                        msg_for_upstream.raw.clone()
                                    };

                                    self.pending_calls.insert(msg.header.serial, PendingCallInfo {
                                        bus: target,
                                        member: msg.header.member.clone(),
                                    });
                                    log_message(&msg, "Client", Some(target));

                                    match target {
                                        Bus::Host => host_write.write_all(&msg_to_send).await?,
                                        Bus::Sandbox => sandbox_write.write_all(&msg_to_send).await?,
                                    }
                                }
                                RouteDecision::Both => {
                                    // Send to both buses (e.g., AddMatch without sender)
                                    // Rewrite match rule body if needed
                                    let msg_to_send = if msg.header.member.as_deref() == Some("AddMatch")
                                        || msg.header.member.as_deref() == Some("RemoveMatch")
                                    {
                                        match rewrite_match_rule_body(&msg) {
                                            Ok(Some(rewritten)) => rewritten,
                                            Ok(None) => msg.raw.clone(),
                                            Err(_) => msg.raw.clone(),
                                        }
                                    } else {
                                        msg.raw.clone()
                                    };

                                    self.pending_calls.insert(msg.header.serial, PendingCallInfo {
                                        bus: Bus::Sandbox,
                                        member: msg.header.member.clone(),
                                    });
                                    log_message(&msg, "Client->Both", None);

                                    host_write.write_all(&msg_to_send).await?;
                                    sandbox_write.write_all(&msg_to_send).await?;
                                }
                                RouteDecision::Merge => {
                                    // ListNames/ListActivatableNames: send to both and merge results
                                    log_message(&msg, "Client->Merge", None);

                                    self.pending_merges.insert(msg.header.serial, PendingMerge {
                                        original_request: msg.clone(),
                                        first_response: None,
                                    });

                                    host_write.write_all(&msg.raw).await?;
                                    sandbox_write.write_all(&msg.raw).await?;
                                }
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
                                        log_message(&msg, "Host->Client(exported)", None);
                                        self.incoming_calls.insert(msg.header.serial, Bus::Host);
                                        client_write.write_all(&msg.raw).await?;
                                        continue;
                                    }
                                }
                            }

                            // Check if this is a response to a pending merge request
                            if let Some(result) = process_merge_response(&msg, Bus::Host, &mut self.pending_merges) {
                                if let MergeResult::Complete(response) = result {
                                    client_write.write_all(&response).await?;
                                }
                                continue;
                            }

                            let msg_to_send = prepare_message_for_client(
                                &msg,
                                Bus::Host,
                                &self.pending_calls,
                            );
                            log_message(&msg, "Host->Client", None);
                            client_write.write_all(&msg_to_send).await?;
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
                result = read_message(&mut sandbox_read), if sandbox_active => {
                    match result {
                        Ok(Some(msg)) => {
                            // Check if this is a response to a pending merge request
                            if let Some(result) = process_merge_response(&msg, Bus::Sandbox, &mut self.pending_merges) {
                                if let MergeResult::Complete(response) = result {
                                    client_write.write_all(&response).await?;
                                }
                                continue;
                            }

                            // Check if this is a call to a sandbox service registered by this client
                            if msg.header.msg_type == MessageType::MethodCall {
                                if let Some(ref dest) = msg.header.destination {
                                    if self.sandbox_services.contains(dest) {
                                        // Rewrite sender to add :s. prefix before forwarding to client
                                        let mut msg_for_client = msg.clone();
                                        if let Err(e) = rewrite_message_header(
                                            &mut msg_for_client,
                                            RewriteDirection::ToClient,
                                            Bus::Sandbox,
                                        ) {
                                            tracing::warn!(error = %e, "Failed to rewrite sender header for sandbox service call");
                                        }

                                        log_message(&msg, "Sandbox->Client(service)", None);
                                        self.incoming_calls.insert(msg.header.serial, Bus::Sandbox);
                                        client_write.write_all(&msg_for_client.raw).await?;
                                        continue;
                                    }
                                }
                            }

                            let msg_to_send = prepare_message_for_client(
                                &msg,
                                Bus::Sandbox,
                                &self.pending_calls,
                            );
                            log_message(&msg, "Sandbox->Client", None);
                            client_write.write_all(&msg_to_send).await?;
                        }
                        Ok(None) => {
                            tracing::debug!("Sandbox bus disconnected");
                            if is_hostpass {
                                // Hostpass clients only use host bus, so they can continue
                                tracing::info!("Hostpass client continues after sandbox disconnect");
                                sandbox_active = false;
                                continue;
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Error reading from sandbox bus");
                            if is_hostpass {
                                tracing::info!("Hostpass client continues after sandbox read error");
                                sandbox_active = false;
                                continue;
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

/// Routing decision result
#[derive(Debug, Clone)]
pub enum RouteDecision {
    /// Route to a single bus
    Single(Bus),
    /// Route to both buses (for AddMatch without sender)
    Both,
    /// Route to both buses and merge results (for ListNames)
    Merge,
}

/// Determine which bus to route a request to based on destination.
fn route_request(config: &Config, msg: &Message, client_exe: Option<&Path>) -> RouteDecision {
    // Hostpass: route ALL messages from hostpass processes to host bus
    // This is required because D-Bus requires Hello() before any other operations,
    // and the host bus won't accept messages from connections that haven't called Hello()
    if let Some(exe) = client_exe {
        if config.has_hostpass(exe) {
            tracing::trace!(exe = %exe.display(), "Routing to host (hostpass)");
            return RouteDecision::Single(Bus::Host);
        }
    }

    // Check if destination is a fake unique name (e.g., :h.1.45)
    if let Some(ref dest) = msg.header.destination {
        if let Some(bus) = get_bus_from_fake_name(dest) {
            tracing::trace!(dest = %dest, bus = ?bus, "Routing by fake unique name");
            return RouteDecision::Single(bus);
        }
    }

    // Special handling for org.freedesktop.DBus methods
    if msg.header.destination.as_deref() == Some("org.freedesktop.DBus")
        && msg.header.interface.as_deref() == Some("org.freedesktop.DBus")
    {
        return route_dbus_daemon_call(config, msg);
    }

    // Method calls and signals with a destination are routed based on config
    if let Some(ref dest) = msg.header.destination {
        if config.should_route_to_host(dest) {
            return RouteDecision::Single(Bus::Host);
        }
    }

    // Default: route to sandbox bus
    RouteDecision::Single(Bus::Sandbox)
}

/// Route calls to org.freedesktop.DBus based on method and arguments
fn route_dbus_daemon_call(config: &Config, msg: &Message) -> RouteDecision {
    let member = msg.header.member.as_deref().unwrap_or("");

    match member {
        // AddMatch/RemoveMatch: route based on sender in the match rule
        "AddMatch" | "RemoveMatch" => {
            if let Some(rule) = msg.extract_string_from_body() {
                let info = parse_match_rule(&rule);

                // If sender is specified, route based on sender
                if let Some(ref sender) = info.sender {
                    // Check if sender is a fake unique name
                    if let Some(bus) = get_bus_from_fake_name(sender) {
                        tracing::trace!(
                            member = member,
                            sender = %sender,
                            bus = ?bus,
                            "Routing {} by fake sender name",
                            member
                        );
                        return RouteDecision::Single(bus);
                    }

                    // Check if sender matches host_routes
                    if config.should_route_to_host(sender) {
                        tracing::trace!(
                            member = member,
                            sender = %sender,
                            "Routing {} to host (sender in host_routes)",
                            member
                        );
                        return RouteDecision::Single(Bus::Host);
                    }
                }

                // If interface is specified but no sender, try to route by interface
                if let Some(ref interface) = info.interface {
                    // Extract service name from interface (e.g., org.fcitx.Fcitx5.Controller1 -> org.fcitx.Fcitx5)
                    if let Some(service) = extract_service_from_interface(interface) {
                        if config.should_route_to_host(&service) {
                            tracing::trace!(
                                member = member,
                                interface = %interface,
                                service = %service,
                                "Routing {} to host (interface matches host_routes)",
                                member
                            );
                            return RouteDecision::Single(Bus::Host);
                        }
                    }
                }

                // No sender or interface to determine routing - send to both buses
                tracing::trace!(
                    member = member,
                    rule = %rule,
                    "Routing {} to both buses (no sender specified)",
                    member
                );
                return RouteDecision::Both;
            }

            // Can't parse rule, default to sandbox
            RouteDecision::Single(Bus::Sandbox)
        }

        // RequestName/ReleaseName: route based on the service name being registered
        "RequestName" | "ReleaseName" => {
            if let Some(name) = msg.extract_name_from_body() {
                if config.should_route_to_host(&name) {
                    tracing::trace!(
                        member = member,
                        name = %name,
                        "Routing {} to host (name in host_routes)",
                        member
                    );
                    return RouteDecision::Single(Bus::Host);
                }
            }
            RouteDecision::Single(Bus::Sandbox)
        }

        // GetNameOwner/NameHasOwner/StartServiceByName: route based on queried name
        "GetNameOwner" | "NameHasOwner" | "StartServiceByName" => {
            if let Some(name) = msg.extract_name_from_body() {
                // Check if name is a fake unique name
                if let Some(bus) = get_bus_from_fake_name(&name) {
                    return RouteDecision::Single(bus);
                }

                if config.should_route_to_host(&name) {
                    tracing::trace!(
                        member = member,
                        name = %name,
                        "Routing {} to host (name in host_routes)",
                        member
                    );
                    return RouteDecision::Single(Bus::Host);
                }
            }
            RouteDecision::Single(Bus::Sandbox)
        }

        // GetConnectionCredentials etc: route based on unique name argument
        "GetConnectionCredentials" | "GetConnectionUnixUser" | "GetConnectionUnixProcessID" => {
            if let Some(name) = msg.extract_name_from_body() {
                if let Some(bus) = get_bus_from_fake_name(&name) {
                    tracing::trace!(
                        member = member,
                        name = %name,
                        bus = ?bus,
                        "Routing {} by fake unique name",
                        member
                    );
                    return RouteDecision::Single(bus);
                }
            }
            // Unknown unique name, default to sandbox
            RouteDecision::Single(Bus::Sandbox)
        }

        // ListNames/ListActivatableNames: merge results from both buses
        "ListNames" | "ListActivatableNames" => {
            tracing::trace!(
                member = member,
                "Routing {} to both buses for merge",
                member
            );
            RouteDecision::Merge
        }

        // Other methods: default to sandbox
        _ => RouteDecision::Single(Bus::Sandbox),
    }
}

/// Extract service name from an interface string.
/// e.g., "org.fcitx.Fcitx5.Controller1" -> "org.fcitx.Fcitx5"
fn extract_service_from_interface(interface: &str) -> Option<String> {
    // Typically service name is the first 3 parts of the interface
    // But this is heuristic - we try different lengths
    let parts: Vec<&str> = interface.split('.').collect();
    if parts.len() >= 3 {
        // Try org.foo.Bar first (3 parts)
        Some(parts[..3].join("."))
    } else {
        None
    }
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
