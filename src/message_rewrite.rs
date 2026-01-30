//! D-Bus message rewriting for fake unique name transformation
//!
//! This module handles rewriting D-Bus messages to transform unique names
//! between their real form and fake form with bus prefixes.

#![allow(dead_code)] // Some functions are prepared for future use

use crate::fake_name::{from_fake_name, to_fake_name};
use crate::message::{Endian, Message};
use crate::session::Bus;
use anyhow::{bail, Result};

/// Direction of message flow for rewriting
#[derive(Debug, Clone, Copy)]
pub enum RewriteDirection {
    /// Message going from upstream bus to client (add fake prefix)
    ToClient,
    /// Message going from client to upstream bus (remove fake prefix)
    ToUpstream,
}

/// Rewrite a message's header fields (sender, destination) for fake name transformation.
///
/// This modifies the raw bytes of the message in place.
pub fn rewrite_message_header(msg: &mut Message, direction: RewriteDirection, source_bus: Bus) -> Result<()> {
    // For ToClient: transform sender from real to fake
    // For ToUpstream: transform destination from fake to real

    match direction {
        RewriteDirection::ToClient => {
            // Add prefix to sender
            if let Some(ref sender) = msg.header.sender {
                let fake_sender = to_fake_name(sender, source_bus);
                if fake_sender != *sender {
                    // Log message context for debugging header parsing issues
                    tracing::debug!(
                        msg_type = ?msg.header.msg_type,
                        interface = ?msg.header.interface,
                        member = ?msg.header.member,
                        destination = ?msg.header.destination,
                        sender = %sender,
                        new_sender = %fake_sender,
                        "Rewriting SENDER header field"
                    );
                    rewrite_header_field(&mut msg.raw, msg.header.endian, 7, &fake_sender)?;
                    msg.header.sender = Some(fake_sender);
                }
            }
        }
        RewriteDirection::ToUpstream => {
            // Remove prefix from destination
            if let Some(ref dest) = msg.header.destination {
                if let Some((real_dest, _bus)) = from_fake_name(dest) {
                    tracing::debug!(
                        msg_type = ?msg.header.msg_type,
                        interface = ?msg.header.interface,
                        member = ?msg.header.member,
                        destination = %dest,
                        new_destination = %real_dest,
                        "Rewriting DESTINATION header field"
                    );
                    rewrite_header_field(&mut msg.raw, msg.header.endian, 6, &real_dest)?;
                    msg.header.destination = Some(real_dest);
                }
            }
        }
    }

    Ok(())
}

/// Rewrite a specific header field in the raw message bytes.
///
/// Header field codes:
/// - 1: PATH (object path)
/// - 2: INTERFACE
/// - 3: MEMBER
/// - 4: ERROR_NAME
/// - 5: REPLY_SERIAL
/// - 6: DESTINATION
/// - 7: SENDER
/// - 8: SIGNATURE
/// - 9: UNIX_FDS
fn rewrite_header_field(raw: &mut Vec<u8>, endian: Endian, field_code: u8, new_value: &str) -> Result<()> {
    // Parse the header to find the field location
    let array_len = endian.read_u32(&raw[12..]) as usize;
    let fields_start = 16;
    let fields_end = fields_start + array_len;

    tracing::trace!(
        field_code = field_code,
        new_value = %new_value,
        array_len = array_len,
        raw_len = raw.len(),
        header_bytes = ?&raw[0..16.min(raw.len())],
        "rewrite_header_field called"
    );

    // Scan through header fields to find the target field
    let mut pos = fields_start;
    let mut field_index = 0;
    while pos < fields_end {
        // Align to 8 bytes (struct alignment)
        let aligned_pos = (pos + 7) & !7;
        if aligned_pos >= fields_end {
            break;
        }
        pos = aligned_pos;

        if pos >= fields_end {
            break;
        }

        let code = raw[pos];
        pos += 1;

        // Read signature (1 byte length + signature bytes)
        if pos >= fields_end {
            break;
        }
        let sig_len = raw[pos] as usize;
        pos += 1 + sig_len + 1; // length byte + signature + null terminator

        // Align to the variant value alignment based on type
        // Read the first byte of the signature (for type determination)
        let sig_byte = if sig_len > 0 { raw[pos - sig_len - 1] } else { 0 };

        tracing::trace!(
            field_index = field_index,
            code = code,
            sig_len = sig_len,
            sig_byte = sig_byte,
            sig_char = %if sig_byte.is_ascii_graphic() { sig_byte as char } else { '?' },
            pos = pos,
            "Parsing header field"
        );
        field_index += 1;
        let value_align = match sig_byte {
            b's' | b'o' | b'u' | b'i' | b'b' | b'h' => 4, // strings, uint32, int32, boolean, fd: 4-byte alignment
            b'n' | b'q' => 2,        // int16, uint16: 2-byte alignment
            b'x' | b't' | b'd' => 8, // int64, uint64, double: 8-byte alignment
            b'g' | b'y' => 1,        // signature, byte: 1-byte alignment
            b'a' => 4,               // array: 4-byte alignment (for length prefix)
            b'v' => 1,               // variant: 1-byte alignment (for signature length)
            b'(' | b'{' => 8,        // struct, dict_entry: 8-byte alignment
            _ => 1,
        };
        pos = (pos + value_align - 1) & !(value_align - 1);

        if code == field_code {
            // Found the field - this is a string value
            // Read current string length
            if pos + 4 > raw.len() {
                bail!("Invalid header field: truncated string length");
            }
            let old_len = endian.read_u32(&raw[pos..]) as usize;
            let old_str_end = pos + 4 + old_len + 1; // length + string + null

            // Create new string data
            let new_len = new_value.len() as u32;
            let new_len_bytes = match endian {
                Endian::Little => new_len.to_le_bytes(),
                Endian::Big => new_len.to_be_bytes(),
            };

            let mut new_str_data = Vec::with_capacity(4 + new_value.len() + 1);
            new_str_data.extend_from_slice(&new_len_bytes);
            new_str_data.extend_from_slice(new_value.as_bytes());
            new_str_data.push(0); // null terminator

            // Calculate internal padding (alignment for next struct in array)
            // Each header field is a struct and must start at 8-byte boundary
            let old_internal_padding = (8 - (old_str_end % 8)) % 8;
            let old_next_field_start = old_str_end + old_internal_padding;
            let old_header_end = fields_end;

            // Check if there are more fields after this one
            let has_more_fields = old_next_field_start < old_header_end;

            // Calculate where the new string will end in the rebuilt message
            let new_str_end_in_new_msg = 16 + (pos - fields_start) + new_str_data.len();

            // Only add internal padding if there are more fields after this one
            let new_internal_padding = if has_more_fields {
                (8 - (new_str_end_in_new_msg % 8)) % 8
            } else {
                0
            };

            // Calculate size difference including internal padding change
            let old_field_total = if has_more_fields {
                (old_str_end - pos) + old_internal_padding
            } else {
                old_str_end - pos
            };
            let new_field_total = new_str_data.len() + new_internal_padding;
            let size_diff = new_field_total as isize - old_field_total as isize;

            // Calculate new array length
            let new_array_len = (array_len as isize + size_diff) as usize;
            let new_fields_end = fields_start + new_array_len;

            // Calculate header-to-body padding
            let old_final_padding = (8 - (old_header_end % 8)) % 8;
            let old_body_start = old_header_end + old_final_padding;

            let new_final_padding = (8 - (new_fields_end % 8)) % 8;

            // Replace the string and reconstruct the message with correct padding
            let mut new_raw = Vec::with_capacity(raw.len());

            // Fixed header (12 bytes)
            new_raw.extend_from_slice(&raw[..12]);

            // New array length (4 bytes)
            let array_len_bytes = match endian {
                Endian::Little => (new_array_len as u32).to_le_bytes(),
                Endian::Big => (new_array_len as u32).to_be_bytes(),
            };
            new_raw.extend_from_slice(&array_len_bytes);

            // Header fields: everything before the string we're replacing
            new_raw.extend_from_slice(&raw[fields_start..pos]);

            // New string data
            new_raw.extend_from_slice(&new_str_data);

            // Internal padding for struct alignment (only if there are more fields)
            new_raw.extend(std::iter::repeat_n(0u8, new_internal_padding));

            // Rest of header fields (after the old string and its internal padding)
            if has_more_fields {
                new_raw.extend_from_slice(&raw[old_next_field_start..old_header_end]);
            }

            // Final header-to-body padding
            new_raw.resize(new_raw.len() + new_final_padding, 0);

            // Body (everything after old padding)
            new_raw.extend_from_slice(&raw[old_body_start..]);

            *raw = new_raw;
            return Ok(());
        }

        // Skip the value based on the signature type
        // Read the first byte of the signature (for type determination)
        let sig_byte = if sig_len > 0 { raw[pos - sig_len - 1] } else { 0 };

        match sig_byte {
            b's' | b'o' => {
                // String/object path: 4 bytes length + string + null
                if pos + 4 > fields_end {
                    break;
                }
                let value_len = endian.read_u32(&raw[pos..]) as usize;
                pos += 4 + value_len + 1;
            }
            b'g' => {
                // Signature: 1 byte length + signature + null
                if pos >= fields_end {
                    break;
                }
                let value_len = raw[pos] as usize;
                pos += 1 + value_len + 1;
            }
            b'u' | b'i' | b'b' | b'h' => {
                // uint32, int32, boolean, unix fd: 4 bytes
                pos += 4;
            }
            b'n' | b'q' => {
                // int16, uint16: 2 bytes
                pos += 2;
            }
            b'x' | b't' | b'd' => {
                // int64, uint64, double: 8 bytes
                pos += 8;
            }
            b'y' => {
                // byte: 1 byte
                pos += 1;
            }
            b'a' => {
                // Array: 4 bytes length + array content
                if pos + 4 > fields_end {
                    break;
                }
                let array_len = endian.read_u32(&raw[pos..]) as usize;
                pos += 4 + array_len;
            }
            b'v' => {
                // Variant: signature + value
                // Read embedded signature length
                if pos >= fields_end {
                    break;
                }
                let vsig_len = raw[pos] as usize;
                if pos + 1 + vsig_len + 1 > fields_end {
                    break;
                }
                let vsig_byte = if vsig_len > 0 { raw[pos + 1] } else { 0 };
                pos += 1 + vsig_len + 1; // skip signature
                // Align and skip the variant value based on its type
                let v_align = match vsig_byte {
                    b's' | b'o' | b'u' | b'i' | b'b' | b'h' | b'a' => 4,
                    b'n' | b'q' => 2,
                    b'x' | b't' | b'd' => 8,
                    b'(' | b'{' => 8,
                    _ => 1,
                };
                pos = (pos + v_align - 1) & !(v_align - 1);
                // Skip value (simplified: only handle basic types inside variant)
                match vsig_byte {
                    b's' | b'o' => {
                        if pos + 4 > fields_end { break; }
                        let vlen = endian.read_u32(&raw[pos..]) as usize;
                        pos += 4 + vlen + 1;
                    }
                    b'g' => {
                        if pos >= fields_end { break; }
                        let vlen = raw[pos] as usize;
                        pos += 1 + vlen + 1;
                    }
                    b'u' | b'i' | b'b' | b'h' => pos += 4,
                    b'n' | b'q' => pos += 2,
                    b'x' | b't' | b'd' => pos += 8,
                    b'y' => pos += 1,
                    _ => {
                        // Nested complex type in variant, can't skip
                        tracing::trace!(vsig_byte = vsig_byte, "Cannot skip nested complex type in variant");
                        break;
                    }
                }
            }
            _ => {
                // Complex type (struct, dict_entry) or parsing error
                // Dump raw bytes around position for debugging
                let context_start = pos.saturating_sub(16);
                let context_end = (pos + 16).min(raw.len());
                let context_bytes: Vec<u8> = raw[context_start..context_end].to_vec();
                tracing::warn!(
                    field_index = field_index,
                    field_code = code,
                    sig_len = sig_len,
                    sig_byte = sig_byte,
                    sig_byte_char = %if sig_byte.is_ascii_graphic() { sig_byte as char } else { '?' },
                    pos = pos,
                    fields_end = fields_end,
                    raw_context = ?context_bytes,
                    "Unknown header field type, cannot parse"
                );
                break;
            }
        }
    }

    // Field not found - this is OK, not all messages have all fields
    Ok(())
}

/// Information about how to rewrite body content for specific D-Bus methods
#[derive(Debug)]
pub struct BodyRewriteInfo {
    /// Positions in the body that contain unique names (as strings)
    pub unique_name_positions: Vec<BodyFieldPosition>,
    /// Whether the first body argument is a match rule string that needs parsing
    pub has_match_rule: bool,
}

#[derive(Debug)]
pub enum BodyFieldPosition {
    /// Simple string at given offset
    StringAt(usize),
    /// Array of strings starting at given offset
    StringArray(usize),
}

/// Get rewrite information for org.freedesktop.DBus methods
pub fn get_dbus_method_rewrite_info(member: &str) -> Option<BodyRewriteInfo> {
    match member {
        // Hello() -> s (unique name) - return value needs rewrite
        "Hello" => Some(BodyRewriteInfo {
            unique_name_positions: vec![BodyFieldPosition::StringAt(0)],
            has_match_rule: false,
        }),
        // GetNameOwner(s) -> s (unique name) - return value needs rewrite
        "GetNameOwner" => Some(BodyRewriteInfo {
            unique_name_positions: vec![BodyFieldPosition::StringAt(0)],
            has_match_rule: false,
        }),
        // ListNames() -> as (array of names) - return value needs rewrite
        "ListNames" | "ListActivatableNames" | "ListQueuedOwners" => Some(BodyRewriteInfo {
            unique_name_positions: vec![BodyFieldPosition::StringArray(0)],
            has_match_rule: false,
        }),
        // AddMatch(s) / RemoveMatch(s) - match rule string needs parsing
        "AddMatch" | "RemoveMatch" => Some(BodyRewriteInfo {
            unique_name_positions: vec![],
            has_match_rule: true,
        }),
        // GetConnectionCredentials(s), GetConnectionUnixUser(s), etc.
        // The argument is a unique name that needs rewriting
        "GetConnectionCredentials" | "GetConnectionUnixUser" | "GetConnectionUnixProcessID" => {
            Some(BodyRewriteInfo {
                unique_name_positions: vec![BodyFieldPosition::StringAt(0)],
                has_match_rule: false,
            })
        }
        _ => None,
    }
}

/// Parse a D-Bus match rule string and extract the sender field
pub fn parse_match_rule_sender(rule: &str) -> Option<String> {
    for part in rule.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("sender='") {
            if let Some(value) = value.strip_suffix('\'') {
                return Some(value.to_string());
            }
        }
        if let Some(value) = part.strip_prefix("sender=") {
            // Handle unquoted values (less common)
            return Some(value.to_string());
        }
    }
    None
}

/// Rewrite the sender field in a match rule string
pub fn rewrite_match_rule_sender(rule: &str, old_sender: &str, new_sender: &str) -> String {
    let old_pattern = format!("sender='{}'", old_sender);
    let new_pattern = format!("sender='{}'", new_sender);
    rule.replace(&old_pattern, &new_pattern)
}

/// Parse a D-Bus match rule string and extract the interface field
pub fn parse_match_rule_interface(rule: &str) -> Option<String> {
    for part in rule.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("interface='") {
            if let Some(value) = value.strip_suffix('\'') {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Information extracted from a match rule for routing decisions
#[derive(Debug, Default)]
pub struct MatchRuleInfo {
    pub sender: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub path: Option<String>,
    pub path_namespace: Option<String>,
    pub destination: Option<String>,
    pub msg_type: Option<String>,
}

/// Parse a complete match rule string
pub fn parse_match_rule(rule: &str) -> MatchRuleInfo {
    let mut info = MatchRuleInfo::default();

    for part in rule.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let value = value.trim_matches('\'');
            match key {
                "sender" => info.sender = Some(value.to_string()),
                "interface" => info.interface = Some(value.to_string()),
                "member" => info.member = Some(value.to_string()),
                "path" => info.path = Some(value.to_string()),
                "path_namespace" => info.path_namespace = Some(value.to_string()),
                "destination" => info.destination = Some(value.to_string()),
                "type" => info.msg_type = Some(value.to_string()),
                _ => {}
            }
        }
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_match_rule_sender() {
        assert_eq!(
            parse_match_rule_sender("type='signal',sender='org.fcitx.Fcitx5'"),
            Some("org.fcitx.Fcitx5".to_string())
        );
        assert_eq!(
            parse_match_rule_sender("sender=':1.45',type='signal'"),
            Some(":1.45".to_string())
        );
        assert_eq!(
            parse_match_rule_sender("type='signal',interface='org.freedesktop.DBus'"),
            None
        );
    }

    #[test]
    fn test_rewrite_match_rule_sender() {
        let rule = "type='signal',sender=':1.45',interface='org.test'";
        let rewritten = rewrite_match_rule_sender(rule, ":1.45", ":h.1.45");
        assert_eq!(rewritten, "type='signal',sender=':h.1.45',interface='org.test'");
    }

    #[test]
    fn test_parse_match_rule() {
        let rule = "type='signal',sender='org.fcitx.Fcitx5',interface='org.fcitx.Fcitx5.Controller1',member='CurrentInputMethodChanged',path='/controller'";
        let info = parse_match_rule(rule);

        assert_eq!(info.msg_type, Some("signal".to_string()));
        assert_eq!(info.sender, Some("org.fcitx.Fcitx5".to_string()));
        assert_eq!(info.interface, Some("org.fcitx.Fcitx5.Controller1".to_string()));
        assert_eq!(info.member, Some("CurrentInputMethodChanged".to_string()));
        assert_eq!(info.path, Some("/controller".to_string()));
    }

    #[test]
    fn test_get_dbus_method_rewrite_info() {
        assert!(get_dbus_method_rewrite_info("Hello").is_some());
        assert!(get_dbus_method_rewrite_info("GetNameOwner").is_some());
        assert!(get_dbus_method_rewrite_info("AddMatch").is_some());
        assert!(get_dbus_method_rewrite_info("ListNames").is_some());
        assert!(get_dbus_method_rewrite_info("SomeOtherMethod").is_none());
    }

    #[test]
    fn test_rewrite_match_rule_with_fake_sender() {
        // Test rewriting match rule with fake unique name
        let rule = "type='signal',sender=':h.1.45',interface='org.test'";
        let rewritten = rewrite_match_rule_sender(rule, ":h.1.45", ":1.45");
        assert_eq!(rewritten, "type='signal',sender=':1.45',interface='org.test'");

        // Test with sandbox prefix
        let rule = "type='signal',sender=':s.1.23',member='Test'";
        let rewritten = rewrite_match_rule_sender(rule, ":s.1.23", ":1.23");
        assert_eq!(rewritten, "type='signal',sender=':1.23',member='Test'");
    }

    #[test]
    fn test_parse_match_rule_with_fake_sender() {
        let rule = "type='signal',sender=':h.1.45',interface='org.test'";
        let sender = parse_match_rule_sender(rule);
        assert_eq!(sender, Some(":h.1.45".to_string()));

        let rule = "type='signal',sender=':s.1.23'";
        let sender = parse_match_rule_sender(rule);
        assert_eq!(sender, Some(":s.1.23".to_string()));
    }

    #[test]
    fn test_rewrite_header_field_sender() {
        use crate::message::{MessageHeader, MessageType};

        // Build a minimal D-Bus message with sender ":1.45"
        // Format: fixed header (12 bytes) + array length (4 bytes) + header fields + padding + body
        let mut raw = vec![
            b'l',  // Little endian
            1,     // METHOD_CALL
            0,     // flags
            1,     // protocol version
            0, 0, 0, 0, // body length (0)
            1, 0, 0, 0, // serial (1)
        ];

        // Header fields array - we'll build this manually
        // Field 7 (SENDER) with value ":1.45"
        let sender_value = b":1.45";
        // field code (7=SENDER), sig_len (1), signature ('s'), null terminator
        let mut fields = vec![7u8, 1, b's', 0];

        // Align to 4 bytes for string value (we're at position 4, already aligned)
        let sender_len = sender_value.len() as u32;
        fields.extend_from_slice(&sender_len.to_le_bytes());
        fields.extend_from_slice(sender_value);
        fields.push(0); // null terminator

        // Add array length to raw
        let array_len = fields.len() as u32;
        raw.extend_from_slice(&array_len.to_le_bytes());
        raw.extend_from_slice(&fields);

        // Add padding to 8-byte boundary
        let header_end = 16 + fields.len();
        let padding = (8 - (header_end % 8)) % 8;
        raw.resize(raw.len() + padding, 0);

        let header = MessageHeader {
            endian: Endian::Little,
            msg_type: MessageType::MethodCall,
            flags: 0,
            serial: 1,
            body_len: 0,
            destination: None,
            reply_serial: None,
            sender: Some(":1.45".to_string()),
            interface: None,
            member: None,
        };

        let mut msg = Message { header, raw };

        // Rewrite sender to add :h. prefix
        let result = rewrite_message_header(&mut msg, RewriteDirection::ToClient, Bus::Host);
        assert!(result.is_ok());

        // Check that the sender was updated
        assert_eq!(msg.header.sender, Some(":h.1.45".to_string()));

        // Verify the raw bytes contain the new sender
        let raw_str = String::from_utf8_lossy(&msg.raw);
        assert!(raw_str.contains(":h.1.45"));
    }

    #[test]
    fn test_rewrite_header_field_destination() {
        use crate::message::{MessageHeader, MessageType};

        // Build a minimal D-Bus message with destination ":h.1.45"
        let mut raw = vec![
            b'l',  // Little endian
            1,     // METHOD_CALL
            0,     // flags
            1,     // protocol version
            0, 0, 0, 0, // body length (0)
            1, 0, 0, 0, // serial (1)
        ];

        // Header fields array
        // Field 6 (DESTINATION) with value ":h.1.45"
        let dest_value = b":h.1.45";
        // field code (6=DESTINATION), sig_len (1), signature ('s'), null terminator
        let mut fields = vec![6u8, 1, b's', 0];

        let dest_len = dest_value.len() as u32;
        fields.extend_from_slice(&dest_len.to_le_bytes());
        fields.extend_from_slice(dest_value);
        fields.push(0); // null terminator

        // Add array length to raw
        let array_len = fields.len() as u32;
        raw.extend_from_slice(&array_len.to_le_bytes());
        raw.extend_from_slice(&fields);

        // Add padding to 8-byte boundary
        let header_end = 16 + fields.len();
        let padding = (8 - (header_end % 8)) % 8;
        raw.resize(raw.len() + padding, 0);

        let header = MessageHeader {
            endian: Endian::Little,
            msg_type: MessageType::MethodCall,
            flags: 0,
            serial: 1,
            body_len: 0,
            destination: Some(":h.1.45".to_string()),
            reply_serial: None,
            sender: None,
            interface: None,
            member: None,
        };

        let mut msg = Message { header, raw };

        // Rewrite destination to remove :h. prefix
        let result = rewrite_message_header(&mut msg, RewriteDirection::ToUpstream, Bus::Host);
        assert!(result.is_ok());

        // Check that the destination was updated
        assert_eq!(msg.header.destination, Some(":1.45".to_string()));

        // Verify the raw bytes contain the new destination
        let raw_str = String::from_utf8_lossy(&msg.raw);
        assert!(raw_str.contains(":1.45"));
        assert!(!raw_str.contains(":h.1.45"));
    }
}
