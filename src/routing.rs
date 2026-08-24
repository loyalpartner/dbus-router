//! Routing decisions for client requests.

use crate::bus::Bus;
use crate::config::Config;
use crate::dbus::daemon::route_dbus_method;
use crate::dbus::message::Message;
use crate::fake_name::get_bus_from_fake_name;
use std::path::Path;

/// Routing decision result.
#[derive(Debug, Clone)]
pub enum RouteDecision {
    /// Route to a single bus.
    Single(Bus),
    /// Route to both buses (for AddMatch without sender).
    Both,
    /// Route to both buses and merge results (for ListNames).
    Merge,
}

/// Determine which bus to route a request to based on destination.
pub fn route_request(config: &Config, msg: &Message, client_exe: Option<&Path>) -> RouteDecision {
    // Hostpass: route ALL messages from hostpass processes to host bus.
    // This is required because D-Bus requires Hello() before any other operations,
    // and the host bus won't accept messages from connections that haven't called Hello().
    if let Some(exe) = client_exe {
        if config.has_hostpass(exe) {
            tracing::trace!(exe = %exe.display(), "Routing to host (hostpass)");
            return RouteDecision::Single(Bus::Host);
        }
    }

    // Check if destination is a fake unique name (e.g., :h.1.45).
    if let Some(ref dest) = msg.header.destination {
        if let Some(bus) = get_bus_from_fake_name(dest) {
            tracing::trace!(dest = %dest, bus = ?bus, "Routing by fake unique name");
            return RouteDecision::Single(bus);
        }
    }

    // Special handling for org.freedesktop.DBus methods.
    if msg.header.destination.as_deref() == Some("org.freedesktop.DBus")
        && msg.header.interface.as_deref() == Some("org.freedesktop.DBus")
    {
        return route_dbus_method(config, msg);
    }

    // Method calls and signals with a destination are routed based on config.
    if let Some(ref dest) = msg.header.destination {
        if config.should_route_to_host(dest) {
            return RouteDecision::Single(Bus::Host);
        }
    }

    // Default: route to sandbox bus.
    RouteDecision::Single(Bus::Sandbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RouteRule;
    use crate::dbus::message::{Endian, Message, MessageHeader, MessageType};

    fn base_message() -> Message {
        Message {
            header: MessageHeader {
                endian: Endian::Little,
                msg_type: MessageType::MethodCall,
                flags: 0,
                serial: 1,
                body_len: 0,
                destination: Some("org.freedesktop.DBus".to_string()),
                reply_serial: None,
                sender: None,
                interface: Some("org.freedesktop.DBus".to_string()),
                member: Some("ListNames".to_string()),
                path: None,
                signature: None,
                unix_fds: None,
            },
            raw: vec![],
            fds: Vec::new(),
        }
    }

    #[test]
    fn test_route_list_names_merge() {
        let config = Config::default();
        let msg = base_message();
        assert!(matches!(
            route_request(&config, &msg, None),
            RouteDecision::Merge
        ));
    }

    #[test]
    fn test_route_regular_destination() {
        let config = Config {
            host_routes: vec![RouteRule {
                destination: "org.freedesktop.portal.*".to_string(),
            }],
            ..Default::default()
        };
        let mut msg = base_message();
        msg.header.destination = Some("org.freedesktop.portal.Desktop".to_string());
        msg.header.interface = None;
        msg.header.member = Some("Method".to_string());

        assert!(matches!(
            route_request(&config, &msg, None),
            RouteDecision::Single(Bus::Host)
        ));
    }
}
