//! Special handling for org.freedesktop.DBus daemon methods
//!
//! This module handles methods that require special processing:
//! - ListNames: merge results from both buses
//! - GetNameOwner: rewrite unique name in response
//! - NameOwnerChanged: rewrite unique names in signal body

use crate::fake_name::{from_fake_name, is_unique_name, to_fake_name};
use crate::message::{Endian, Message};
use crate::message_rewrite::{parse_match_rule_sender, rewrite_match_rule_sender};
use crate::session::Bus;
use anyhow::{bail, Result};
use zvariant::{serialized::{Context, Data}, to_bytes, Endian as ZEndian, LE, BE};

/// Methods that need response rewriting (unique name in return value)
pub const METHODS_NEED_RESPONSE_REWRITE: &[&str] = &[
    "Hello",
    "GetNameOwner",
    "ListQueuedOwners",
];

/// Methods that need request rewriting (unique name in argument)
pub const METHODS_NEED_REQUEST_REWRITE: &[&str] = &[
    "GetConnectionCredentials",
    "GetConnectionUnixUser",
    "GetConnectionUnixProcessID",
    "GetConnectionSELinuxSecurityContext",
    "GetAdtAuditSessionData",
    "NameHasOwner",
];

/// Methods that need result merging from both buses
pub const METHODS_NEED_MERGE: &[&str] = &[
    "ListNames",
    "ListActivatableNames",
];

/// Signals that need body rewriting
pub const SIGNALS_NEED_REWRITE: &[&str] = &[
    "NameOwnerChanged",
];

/// Check if a method needs response rewriting
pub fn needs_response_rewrite(member: &str) -> bool {
    METHODS_NEED_RESPONSE_REWRITE.contains(&member)
}

/// Check if a method needs request rewriting
pub fn needs_request_rewrite(member: &str) -> bool {
    METHODS_NEED_REQUEST_REWRITE.contains(&member)
}

/// Check if a method needs result merging
pub fn needs_merge(member: &str) -> bool {
    METHODS_NEED_MERGE.contains(&member)
}

/// Check if a signal needs body rewriting
pub fn signal_needs_rewrite(member: &str) -> bool {
    SIGNALS_NEED_REWRITE.contains(&member)
}

/// Rewrite AddMatch/RemoveMatch body to remove fake prefix from sender.
/// Returns the rewritten message bytes if sender was rewritten, or None if no rewrite needed.
pub fn rewrite_match_rule_body(msg: &Message) -> Result<Option<Vec<u8>>> {
    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        return Ok(None);
    }

    // Parse the string from body
    let str_len = msg.header.endian.read_u32(&msg.raw[body_start..]) as usize;
    let str_start = body_start + 4;
    let str_end = str_start + str_len;

    if str_end > msg.raw.len() {
        return Ok(None);
    }

    let rule = String::from_utf8_lossy(&msg.raw[str_start..str_end]).to_string();

    // Check if sender in match rule is a fake unique name
    if let Some(sender) = parse_match_rule_sender(&rule) {
        if let Some((real_sender, _bus)) = from_fake_name(&sender) {
            // Rewrite the match rule with the real sender
            let new_rule = rewrite_match_rule_sender(&rule, &sender, &real_sender);
            tracing::trace!(
                old_sender = %sender,
                new_sender = %real_sender,
                "Rewrote match rule sender"
            );
            return rebuild_message_with_string(&msg.raw, msg.header.endian, body_start, &new_rule)
                .map(Some);
        }
    }

    Ok(None)
}

/// Rewrite a single unique name in response body (for GetNameOwner, Hello)
pub fn rewrite_single_name_response(msg: &Message, source: Bus) -> Result<Vec<u8>> {
    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        return Ok(msg.raw.clone());
    }

    // Parse the string from body
    let str_len = msg.header.endian.read_u32(&msg.raw[body_start..]) as usize;
    let str_start = body_start + 4;
    let str_end = str_start + str_len;

    if str_end > msg.raw.len() {
        return Ok(msg.raw.clone());
    }

    let name = String::from_utf8_lossy(&msg.raw[str_start..str_end]).to_string();

    // Only rewrite if it's a unique name
    if !is_unique_name(&name) {
        return Ok(msg.raw.clone());
    }

    let fake_name = to_fake_name(&name, source);

    // Build new message with rewritten name
    rebuild_message_with_string(&msg.raw, msg.header.endian, body_start, &fake_name)
}

/// Rewrite unique names in string array response (for ListQueuedOwners)
pub fn rewrite_string_array_response(msg: &Message, source: Bus) -> Result<Vec<u8>> {
    let names = parse_string_array(msg)?;

    if names.is_empty() {
        return Ok(msg.raw.clone());
    }

    // Rewrite each unique name to add prefix
    let rewritten: Vec<String> = names
        .into_iter()
        .map(|name| {
            if is_unique_name(&name) {
                to_fake_name(&name, source)
            } else {
                name
            }
        })
        .collect();

    // Serialize the new array body
    let z_endian = match msg.header.endian {
        Endian::Little => LE,
        Endian::Big => BE,
    };
    let new_body = to_bytes(Context::new_dbus(z_endian, 0), &rewritten)?;

    // Rebuild message with new body, preserving header
    rebuild_message_with_body(&msg.raw, msg.header.endian, &new_body)
}

/// Rewrite unique name in request body (for GetConnectionCredentials etc.)
/// Removes the fake prefix before sending to upstream
pub fn rewrite_unique_name_request(msg: &Message) -> Result<(Vec<u8>, Bus)> {
    use crate::fake_name::from_fake_name;

    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        bail!("No body in message");
    }

    // Parse the string from body
    let str_len = msg.header.endian.read_u32(&msg.raw[body_start..]) as usize;
    let str_start = body_start + 4;
    let str_end = str_start + str_len;

    if str_end > msg.raw.len() {
        bail!("Invalid string in body");
    }

    let name = String::from_utf8_lossy(&msg.raw[str_start..str_end]).to_string();

    // Check if it's a fake unique name
    if let Some((real_name, bus)) = from_fake_name(&name) {
        let new_raw = rebuild_message_with_string(&msg.raw, msg.header.endian, body_start, &real_name)?;
        Ok((new_raw, bus))
    } else {
        // Not a fake name, return original
        Ok((msg.raw.clone(), Bus::Sandbox))
    }
}

/// Rewrite ListNames/ListActivatableNames response - add fake prefix to all unique names
pub fn rewrite_list_names_response(msg: &Message, source: Bus) -> Result<Vec<u8>> {
    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        return Ok(msg.raw.clone());
    }

    let z_endian = match msg.header.endian {
        Endian::Little => ZEndian::Little,
        Endian::Big => ZEndian::Big,
    };

    // Parse the string array from body
    let ctxt = Context::new_dbus(z_endian, body_start);
    let body_data = &msg.raw[body_start..];
    let data = Data::new(body_data, ctxt);

    let names: Vec<String> = match data.deserialize::<Vec<String>>() {
        Ok((names, _)) => names,
        Err(e) => {
            tracing::warn!("Failed to parse ListNames response: {}", e);
            return Ok(msg.raw.clone());
        }
    };

    // Rewrite unique names
    let rewritten_names: Vec<String> = names
        .into_iter()
        .map(|name| {
            if is_unique_name(&name) {
                to_fake_name(&name, source)
            } else {
                name
            }
        })
        .collect();

    // Serialize the new array
    let new_body = match msg.header.endian {
        Endian::Little => to_bytes(Context::new_dbus(LE, 0), &rewritten_names)?,
        Endian::Big => to_bytes(Context::new_dbus(BE, 0), &rewritten_names)?,
    };

    // Rebuild message with new body
    rebuild_message_with_body(&msg.raw, msg.header.endian, &new_body)
}

/// Merge ListNames results from both buses
pub fn merge_list_names(host_names: Vec<String>, sandbox_names: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;

    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();

    // Add sandbox names first (with :s. prefix for unique names)
    for name in sandbox_names {
        let key = if is_unique_name(&name) {
            to_fake_name(&name, Bus::Sandbox)
        } else {
            name.clone()
        };
        if seen.insert(key.clone()) {
            result.push(key);
        }
    }

    // Add host names (with :h. prefix for unique names)
    for name in host_names {
        let key = if is_unique_name(&name) {
            to_fake_name(&name, Bus::Host)
        } else {
            name.clone()
        };
        if seen.insert(key.clone()) {
            result.push(key);
        }
    }

    result
}

/// Rewrite NameOwnerChanged signal body
/// Body format: (name: s, old_owner: s, new_owner: s)
pub fn rewrite_name_owner_changed(msg: &Message, source: Bus) -> Result<Vec<u8>> {
    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        return Ok(msg.raw.clone());
    }

    let z_endian = match msg.header.endian {
        Endian::Little => ZEndian::Little,
        Endian::Big => ZEndian::Big,
    };

    // Parse the three strings from body
    let ctxt = Context::new_dbus(z_endian, body_start);
    let body_data = &msg.raw[body_start..];
    let data = Data::new(body_data, ctxt);

    let (name, old_owner, new_owner): (String, String, String) = match data.deserialize::<(String, String, String)>() {
        Ok((tuple, _)) => tuple,
        Err(e) => {
            tracing::warn!("Failed to parse NameOwnerChanged: {}", e);
            return Ok(msg.raw.clone());
        }
    };

    // Rewrite unique names in old_owner and new_owner
    let new_old_owner = if is_unique_name(&old_owner) && !old_owner.is_empty() {
        to_fake_name(&old_owner, source)
    } else {
        old_owner
    };

    let new_new_owner = if is_unique_name(&new_owner) && !new_owner.is_empty() {
        to_fake_name(&new_owner, source)
    } else {
        new_owner
    };

    // Serialize the new body
    let new_body = match msg.header.endian {
        Endian::Little => to_bytes(Context::new_dbus(LE, 0), &(name, new_old_owner, new_new_owner))?,
        Endian::Big => to_bytes(Context::new_dbus(BE, 0), &(name, new_old_owner, new_new_owner))?,
    };

    rebuild_message_with_body(&msg.raw, msg.header.endian, &new_body)
}

/// Get the body start position in a D-Bus message
fn get_body_start(raw: &[u8], endian: Endian) -> usize {
    let fixed_header_size = 12;
    let array_len = endian.read_u32(&raw[fixed_header_size..]) as usize;
    let header_end = 16 + array_len;
    let padding = (8 - (header_end % 8)) % 8;
    header_end + padding
}

/// Rebuild a message with a new string in the body
fn rebuild_message_with_string(raw: &[u8], endian: Endian, _body_start: usize, new_str: &str) -> Result<Vec<u8>> {
    let new_len = new_str.len() as u32;
    let new_len_bytes = match endian {
        Endian::Little => new_len.to_le_bytes(),
        Endian::Big => new_len.to_be_bytes(),
    };

    // Build new body: length + string + null terminator
    let mut new_body = Vec::with_capacity(4 + new_str.len() + 1);
    new_body.extend_from_slice(&new_len_bytes);
    new_body.extend_from_slice(new_str.as_bytes());
    new_body.push(0);

    rebuild_message_with_body(raw, endian, &new_body)
}

/// Rebuild a message with a new body
fn rebuild_message_with_body(raw: &[u8], endian: Endian, new_body: &[u8]) -> Result<Vec<u8>> {
    let body_start = get_body_start(raw, endian);

    // Update body length in header (bytes 4-7)
    let new_body_len = new_body.len() as u32;
    let body_len_bytes = match endian {
        Endian::Little => new_body_len.to_le_bytes(),
        Endian::Big => new_body_len.to_be_bytes(),
    };

    let mut new_raw = Vec::with_capacity(body_start + new_body.len());
    new_raw.extend_from_slice(&raw[..4]); // endian, type, flags, version
    new_raw.extend_from_slice(&body_len_bytes); // new body length
    new_raw.extend_from_slice(&raw[8..body_start]); // serial + header fields + padding
    new_raw.extend_from_slice(new_body);

    Ok(new_raw)
}

/// Parse a string array from message body (for ListNames response)
pub fn parse_string_array(msg: &Message) -> Result<Vec<String>> {
    let body_start = get_body_start(&msg.raw, msg.header.endian);

    if body_start >= msg.raw.len() {
        return Ok(vec![]);
    }

    let z_endian = match msg.header.endian {
        Endian::Little => ZEndian::Little,
        Endian::Big => ZEndian::Big,
    };

    let ctxt = Context::new_dbus(z_endian, body_start);
    let body_data = &msg.raw[body_start..];
    let data = Data::new(body_data, ctxt);

    let names: Vec<String> = data.deserialize::<Vec<String>>()
        .map(|(names, _)| names)
        .unwrap_or_default();

    Ok(names)
}

/// Build a ListNames response message
pub fn build_list_names_response(
    original_request: &Message,
    names: Vec<String>,
) -> Result<Vec<u8>> {
    let endian = original_request.header.endian;

    // Serialize the names array
    let body = match endian {
        Endian::Little => to_bytes(Context::new_dbus(LE, 0), &names)?,
        Endian::Big => to_bytes(Context::new_dbus(BE, 0), &names)?,
    };

    // Build response header with signature "as" for array of strings
    build_method_return(original_request, &body, Some("as"))
}

/// Build a MethodReturn message
fn build_method_return(request: &Message, body: &[u8], signature: Option<&str>) -> Result<Vec<u8>> {
    use zvariant::{Signature, Value};

    let endian = request.header.endian;
    let z_endian = match endian {
        Endian::Little => LE,
        Endian::Big => BE,
    };

    // Build header fields for MethodReturn
    let mut fields: Vec<(u8, Value)> = vec![
        (5, Value::U32(request.header.serial)), // REPLY_SERIAL
    ];

    // Add signature field if provided
    if let Some(sig) = signature {
        fields.push((8, Value::Signature(Signature::from_str_unchecked(sig)))); // SIGNATURE
    }

    let ctxt = Context::new_dbus(z_endian, 12);
    let fields_encoded = to_bytes(ctxt, &fields)?;
    let array_len = (fields_encoded.len() - 4) as u32;

    // Calculate padding
    let header_end = 16 + array_len as usize;
    let padding = (8 - (header_end % 8)) % 8;

    // Build message
    let mut msg = Vec::with_capacity(16 + array_len as usize + padding + body.len());

    // Fixed header
    msg.push(if endian == Endian::Little { b'l' } else { b'B' });
    msg.push(2); // MethodReturn
    msg.push(1); // NO_REPLY_EXPECTED flag
    msg.push(1); // protocol version

    // Body length
    let body_len_bytes = match endian {
        Endian::Little => (body.len() as u32).to_le_bytes(),
        Endian::Big => (body.len() as u32).to_be_bytes(),
    };
    msg.extend_from_slice(&body_len_bytes);

    // Serial (use request serial + 1000000 to avoid collision)
    let serial = request.header.serial.wrapping_add(1000000);
    let serial_bytes = match endian {
        Endian::Little => serial.to_le_bytes(),
        Endian::Big => serial.to_be_bytes(),
    };
    msg.extend_from_slice(&serial_bytes);

    // Header fields
    msg.extend_from_slice(&fields_encoded);

    // Padding
    msg.resize(msg.len() + padding, 0);

    // Body
    msg.extend_from_slice(body);

    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_list_names() {
        let host = vec![
            "org.fcitx.Fcitx5".to_string(),
            ":1.45".to_string(),
            "org.freedesktop.DBus".to_string(),
        ];
        let sandbox = vec![
            "org.example.App".to_string(),
            ":1.23".to_string(),
            "org.freedesktop.DBus".to_string(), // duplicate
        ];

        let merged = merge_list_names(host, sandbox);

        // Should have: org.example.App, :s.1.23, org.freedesktop.DBus, org.fcitx.Fcitx5, :h.1.45
        assert!(merged.contains(&"org.example.App".to_string()));
        assert!(merged.contains(&":s.1.23".to_string()));
        assert!(merged.contains(&"org.freedesktop.DBus".to_string()));
        assert!(merged.contains(&"org.fcitx.Fcitx5".to_string()));
        assert!(merged.contains(&":h.1.45".to_string()));

        // org.freedesktop.DBus should only appear once
        assert_eq!(merged.iter().filter(|n| *n == "org.freedesktop.DBus").count(), 1);
    }

    #[test]
    fn test_needs_checks() {
        assert!(needs_response_rewrite("GetNameOwner"));
        assert!(needs_response_rewrite("Hello"));
        assert!(!needs_response_rewrite("RequestName"));

        assert!(needs_request_rewrite("GetConnectionCredentials"));
        assert!(!needs_request_rewrite("GetNameOwner"));

        assert!(needs_merge("ListNames"));
        assert!(!needs_merge("GetNameOwner"));

        assert!(signal_needs_rewrite("NameOwnerChanged"));
        assert!(!signal_needs_rewrite("NameAcquired"));
    }
}
