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
                    rewrite_header_field(&mut msg.raw, msg.header.endian, 7, &fake_sender)?;
                    msg.header.sender = Some(fake_sender);
                }
            }
        }
        RewriteDirection::ToUpstream => {
            // Remove prefix from destination
            if let Some(ref dest) = msg.header.destination {
                if let Some((real_dest, _bus)) = from_fake_name(dest) {
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

    // Scan through header fields to find the target field
    let mut pos = fields_start;
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

        // Align to the variant value alignment
        // For strings (signature 's'), alignment is 4
        let value_align = if sig_len == 1 && pos > 2 && raw[pos - sig_len - 1] == b's' {
            4
        } else {
            1
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

            // Calculate size difference
            let old_field_size = old_str_end - pos;
            let new_field_size = new_str_data.len();
            let size_diff = new_field_size as isize - old_field_size as isize;

            // Replace the string in the raw buffer
            let mut new_raw = Vec::with_capacity((raw.len() as isize + size_diff) as usize);
            new_raw.extend_from_slice(&raw[..pos]);
            new_raw.extend_from_slice(&new_str_data);
            new_raw.extend_from_slice(&raw[old_str_end..]);

            // Update array length in header
            let new_array_len = (array_len as isize + size_diff) as u32;
            let array_len_bytes = match endian {
                Endian::Little => new_array_len.to_le_bytes(),
                Endian::Big => new_array_len.to_be_bytes(),
            };
            new_raw[12..16].copy_from_slice(&array_len_bytes);

            *raw = new_raw;
            return Ok(());
        }

        // Skip the value
        // For strings: 4 bytes length + string + null
        if pos + 4 > fields_end {
            break;
        }
        let value_len = endian.read_u32(&raw[pos..]) as usize;
        pos += 4 + value_len + 1;
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
}
