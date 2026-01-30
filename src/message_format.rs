//! D-Bus message formatting for dbus-monitor style output
//!
//! This module provides functions to format D-Bus messages in a style similar
//! to `dbus-monitor`, making it easier to debug and understand message flow.

use crate::message::{Endian, Message, MessageType};
use crate::session::Bus;
use zvariant::{
    serialized::{Context, Data},
    Endian as ZEndian, Value,
};

/// Format a D-Bus message in dbus-monitor style.
///
/// Example output:
/// ```text
/// method_call sender=:1.45 -> dest=org.fcitx.Fcitx5 serial=42 path=/org/fcitx/Fcitx5 iface=org.fcitx.Fcitx5.Controller1 member=SetCurrentInputMethod
///    string "pinyin"
/// ```
pub fn format_message(msg: &Message, direction: &str, bus: Option<Bus>) -> String {
    let mut output = String::new();

    // Format message type
    let type_str = match msg.header.msg_type {
        MessageType::MethodCall => "method_call",
        MessageType::MethodReturn => "method_return",
        MessageType::Error => "error",
        MessageType::Signal => "signal",
        MessageType::Invalid => "invalid",
    };

    // Build the header line
    output.push_str(type_str);

    // Add sender
    let sender = msg.header.sender.as_deref().unwrap_or("(unknown)");
    output.push_str(&format!(" sender={}", sender));

    // Add direction and destination
    let dest = msg.header.destination.as_deref().unwrap_or("(broadcast)");
    output.push_str(&format!(" -> dest={}", dest));

    // Add bus info if provided
    if let Some(b) = bus {
        let bus_str = match b {
            Bus::Host => "[H]",
            Bus::Sandbox => "[S]",
        };
        output.push_str(&format!(" {}", bus_str));
    }

    // Add direction indicator
    output.push_str(&format!(" ({})", direction));

    // Add serial
    output.push_str(&format!(" serial={}", msg.header.serial));

    // Add reply_serial for method_return and error
    if let Some(reply_serial) = msg.header.reply_serial {
        output.push_str(&format!(" reply_to={}", reply_serial));
    }

    // Add path if present
    if let Some(ref path) = msg.header.path {
        output.push_str(&format!(" path={}", path));
    }

    // Add interface if present
    if let Some(ref iface) = msg.header.interface {
        output.push_str(&format!(" iface={}", iface));
    }

    // Add member if present
    if let Some(ref member) = msg.header.member {
        output.push_str(&format!(" member={}", member));
    }

    // Parse and format body if signature is present and body is non-empty
    if msg.header.body_len > 0 {
        if let Some(ref sig) = msg.header.signature {
            if !sig.is_empty() {
                if let Some(body_str) = format_body(msg, sig) {
                    output.push('\n');
                    output.push_str(&body_str);
                }
            }
        }
    }

    output
}

/// Format the message body based on its signature.
fn format_body(msg: &Message, signature: &str) -> Option<String> {
    let body_start = msg.body_start();

    if body_start >= msg.raw.len() {
        return None;
    }

    let body_bytes = &msg.raw[body_start..];
    let z_endian = match msg.header.endian {
        Endian::Little => ZEndian::Little,
        Endian::Big => ZEndian::Big,
    };

    // Parse body using zvariant
    // Body starts at position 0 within its own context (already aligned)
    let ctxt = Context::new_dbus(z_endian, 0);
    let data = Data::new(body_bytes, ctxt);

    // Try to parse as a single Value - this handles most cases
    match data.deserialize::<Value>() {
        Ok((value, _)) => Some(format_value(&value, 3)),
        Err(e) => {
            tracing::trace!(
                signature = signature,
                error = %e,
                "Failed to parse body as Value"
            );
            // Try parsing based on specific common signatures
            format_body_by_signature(body_bytes, signature, z_endian)
        }
    }
}

/// Try to format body based on specific signature patterns.
fn format_body_by_signature(body_bytes: &[u8], signature: &str, endian: ZEndian) -> Option<String> {
    let ctxt = Context::new_dbus(endian, 0);
    let data = Data::new(body_bytes, ctxt);

    match signature {
        "s" => {
            // Single string
            let value: String = data.deserialize().ok()?.0;
            Some(format!("   string \"{}\"", value))
        }
        "u" => {
            // Single uint32
            let value: u32 = data.deserialize().ok()?.0;
            Some(format!("   uint32 {}", value))
        }
        "su" | "us" => {
            // String + uint32 or uint32 + string
            let (v1, v2): (String, u32) = data.deserialize().ok()?.0;
            Some(format!("   string \"{}\"\n   uint32 {}", v1, v2))
        }
        "ss" => {
            // Two strings
            let (v1, v2): (String, String) = data.deserialize().ok()?.0;
            Some(format!("   string \"{}\"\n   string \"{}\"", v1, v2))
        }
        "sss" => {
            // Three strings (common for NameOwnerChanged)
            let (v1, v2, v3): (String, String, String) = data.deserialize().ok()?.0;
            Some(format!(
                "   string \"{}\"\n   string \"{}\"\n   string \"{}\"",
                v1, v2, v3
            ))
        }
        "as" => {
            // Array of strings
            let values: Vec<String> = data.deserialize().ok()?.0;
            if values.is_empty() {
                Some("   array []".to_string())
            } else {
                let mut output = "   array [".to_string();
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    output.push_str(&format!("      string \"{}\"", v));
                }
                output.push_str("\n   ]");
                Some(output)
            }
        }
        _ => {
            // Unknown signature, just show it
            tracing::trace!(
                signature = signature,
                "Unhandled signature for body formatting"
            );
            None
        }
    }
}

/// Format a zvariant Value in dbus-monitor style with indentation.
fn format_value(value: &Value, indent: usize) -> String {
    let indent_str = " ".repeat(indent);

    match value {
        Value::U8(v) => format!("{}byte {}", indent_str, v),
        Value::Bool(v) => format!("{}boolean {}", indent_str, v),
        Value::I16(v) => format!("{}int16 {}", indent_str, v),
        Value::U16(v) => format!("{}uint16 {}", indent_str, v),
        Value::I32(v) => format!("{}int32 {}", indent_str, v),
        Value::U32(v) => format!("{}uint32 {}", indent_str, v),
        Value::I64(v) => format!("{}int64 {}", indent_str, v),
        Value::U64(v) => format!("{}uint64 {}", indent_str, v),
        Value::F64(v) => format!("{}double {}", indent_str, v),
        Value::Str(s) => format!("{}string \"{}\"", indent_str, s),
        Value::ObjectPath(p) => format!("{}object_path \"{}\"", indent_str, p),
        Value::Signature(s) => format!("{}signature \"{}\"", indent_str, s),
        Value::Value(v) => {
            let inner = format_value(v, 0);
            format!("{}variant {}", indent_str, inner.trim_start())
        }
        Value::Array(arr) => {
            let items: Vec<_> = arr.iter().collect();
            if items.is_empty() {
                format!("{}array []", indent_str)
            } else {
                let mut output = format!("{}array [", indent_str);
                for (i, elem) in items.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    output.push_str(&format_value(elem, indent + 3));
                }
                output.push('\n');
                output.push_str(&format!("{}]", indent_str));
                output
            }
        }
        Value::Dict(dict) => {
            let entries: Vec<_> = dict.iter().collect();
            if entries.is_empty() {
                format!("{}dict {{}}", indent_str)
            } else {
                let mut output = format!("{}dict {{", indent_str);
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    let key_str = format_value(k, 0);
                    let val_str = format_value(v, 0);
                    output.push_str(&format!(
                        "{}   {} => {}",
                        indent_str,
                        key_str.trim(),
                        val_str.trim()
                    ));
                }
                output.push('\n');
                output.push_str(&format!("{}}}", indent_str));
                output
            }
        }
        Value::Structure(s) => {
            let fields = s.fields();
            if fields.is_empty() {
                format!("{}struct {{}}", indent_str)
            } else {
                let mut output = format!("{}struct {{", indent_str);
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    output.push_str(&format_value(field, indent + 3));
                }
                output.push('\n');
                output.push_str(&format!("{}}}", indent_str));
                output
            }
        }
        Value::Fd(fd) => format!("{}unix_fd {:?}", indent_str, fd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageHeader;

    #[test]
    fn test_format_value_basic_types() {
        assert_eq!(format_value(&Value::U8(42), 0), "byte 42");
        assert_eq!(format_value(&Value::Bool(true), 0), "boolean true");
        assert_eq!(format_value(&Value::I32(-123), 0), "int32 -123");
        assert_eq!(format_value(&Value::U32(456), 0), "uint32 456");
        assert_eq!(format_value(&Value::F64(3.14), 0), "double 3.14");
        assert_eq!(
            format_value(&Value::Str("hello".into()), 0),
            "string \"hello\""
        );
    }

    #[test]
    fn test_format_value_with_indent() {
        assert_eq!(format_value(&Value::U32(123), 3), "   uint32 123");
        assert_eq!(
            format_value(&Value::Str("test".into()), 3),
            "   string \"test\""
        );
    }

    #[test]
    fn test_format_message_method_call() {
        let msg = Message {
            header: MessageHeader {
                endian: Endian::Little,
                msg_type: MessageType::MethodCall,
                flags: 0,
                serial: 42,
                body_len: 0,
                destination: Some("org.freedesktop.DBus".to_string()),
                reply_serial: None,
                sender: Some(":1.45".to_string()),
                interface: Some("org.freedesktop.DBus".to_string()),
                member: Some("Hello".to_string()),
                path: Some("/org/freedesktop/DBus".to_string()),
                signature: None,
            },
            raw: vec![],
        };

        let formatted = format_message(&msg, "->", Some(Bus::Host));
        assert!(formatted.contains("method_call"));
        assert!(formatted.contains("sender=:1.45"));
        assert!(formatted.contains("dest=org.freedesktop.DBus"));
        assert!(formatted.contains("serial=42"));
        assert!(formatted.contains("path=/org/freedesktop/DBus"));
        assert!(formatted.contains("member=Hello"));
        assert!(formatted.contains("[H]"));
    }

    #[test]
    fn test_format_message_method_return() {
        let msg = Message {
            header: MessageHeader {
                endian: Endian::Little,
                msg_type: MessageType::MethodReturn,
                flags: 0,
                serial: 100,
                body_len: 0,
                destination: Some(":1.45".to_string()),
                reply_serial: Some(42),
                sender: Some(":1.1".to_string()),
                interface: None,
                member: None,
                path: None,
                signature: None,
            },
            raw: vec![],
        };

        let formatted = format_message(&msg, "<-", None);
        assert!(formatted.contains("method_return"));
        assert!(formatted.contains("reply_to=42"));
        assert!(formatted.contains("serial=100"));
    }

    #[test]
    fn test_format_message_signal() {
        let msg = Message {
            header: MessageHeader {
                endian: Endian::Little,
                msg_type: MessageType::Signal,
                flags: 0,
                serial: 50,
                body_len: 0,
                destination: None,
                reply_serial: None,
                sender: Some(":1.3".to_string()),
                interface: Some("org.freedesktop.DBus".to_string()),
                member: Some("NameOwnerChanged".to_string()),
                path: Some("/org/freedesktop/DBus".to_string()),
                signature: None,
            },
            raw: vec![],
        };

        let formatted = format_message(&msg, "->", Some(Bus::Sandbox));
        assert!(formatted.contains("signal"));
        assert!(formatted.contains("dest=(broadcast)"));
        assert!(formatted.contains("[S]"));
        assert!(formatted.contains("member=NameOwnerChanged"));
    }
}
