//! D-Bus message parsing
//!
//! D-Bus wire protocol message format:
//! - byte 0: endian marker ('l' = little, 'B' = big)
//! - byte 1: message type (1=method_call, 2=method_return, 3=error, 4=signal)
//! - byte 2: flags
//! - byte 3: protocol version (always 1)
//! - bytes 4-7: body length (u32)
//! - bytes 8-11: serial (u32)
//! - bytes 12+: header fields array (length + fields)

use anyhow::{bail, Result};
use tokio::io::{AsyncRead, AsyncReadExt};
use zvariant::{serialized::Context, Endian as ZEndian, Value};

/// Maximum D-Bus message size (128 MB per spec, but we use 64 MB limit).
const MAX_MESSAGE_SIZE: u32 = 64 * 1024 * 1024;

/// Fixed header size (before variable header fields array).
const FIXED_HEADER_SIZE: usize = 12;

/// Minimum header size (fixed header + 4 bytes for array length).
const MIN_HEADER_SIZE: usize = 16;

/// D-Bus message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Invalid = 0,
    MethodCall = 1,
    MethodReturn = 2,
    Error = 3,
    Signal = 4,
}

impl From<u8> for MessageType {
    fn from(v: u8) -> Self {
        match v {
            1 => MessageType::MethodCall,
            2 => MessageType::MethodReturn,
            3 => MessageType::Error,
            4 => MessageType::Signal,
            _ => MessageType::Invalid,
        }
    }
}

/// Byte order for D-Bus message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    /// Read a u32 from a buffer with the appropriate endianness.
    pub fn read_u32(&self, buf: &[u8]) -> u32 {
        let arr: [u8; 4] = buf[..4].try_into().unwrap();
        match self {
            Endian::Little => u32::from_le_bytes(arr),
            Endian::Big => u32::from_be_bytes(arr),
        }
    }
}

/// Parsed D-Bus message header.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields are part of public API for library users
pub struct MessageHeader {
    pub endian: Endian,
    pub msg_type: MessageType,
    pub flags: u8,
    pub serial: u32,
    pub body_len: u32,
    pub destination: Option<String>,
    pub reply_serial: Option<u32>,
    pub sender: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub path: Option<String>,
    pub signature: Option<String>,
}

/// A complete D-Bus message (header + body as raw bytes).
#[derive(Debug, Clone)]
pub struct Message {
    pub header: MessageHeader,
    /// Raw message bytes including header and body
    pub raw: Vec<u8>,
}

impl Message {
    /// Check if this message is a RequestName call to the D-Bus daemon.
    pub fn is_request_name(&self) -> bool {
        self.header.destination.as_deref() == Some("org.freedesktop.DBus")
            && self.header.interface.as_deref() == Some("org.freedesktop.DBus")
            && self.header.member.as_deref() == Some("RequestName")
    }

    /// Get the byte offset where the message body starts.
    ///
    /// The body starts after the header fields array, aligned to an 8-byte boundary.
    pub fn body_start(&self) -> usize {
        let array_len = self.header.endian.read_u32(&self.raw[FIXED_HEADER_SIZE..]) as usize;
        let header_end = MIN_HEADER_SIZE + array_len;
        let padding = (8 - (header_end % 8)) % 8;
        header_end + padding
    }

    /// Extract a simple string from the message body.
    /// This works for methods where the first argument is a string (AddMatch, etc.)
    pub fn extract_string_from_body(&self) -> Option<String> {
        self.extract_name_from_body()
    }

    /// Extract the service name from RequestName body.
    /// The body format is: STRING (name) + UINT32 (flags)
    pub fn extract_name_from_body(&self) -> Option<String> {
        let body_start = self.body_start();

        if body_start + 4 > self.raw.len() {
            return None;
        }

        // Parse STRING: u32 length + bytes + null terminator
        let str_len = self.header.endian.read_u32(&self.raw[body_start..]) as usize;
        let str_start = body_start + 4;

        if str_start + str_len > self.raw.len() {
            return None;
        }

        String::from_utf8(self.raw[str_start..str_start + str_len].to_vec()).ok()
    }
}

/// Read a complete D-Bus message from the stream.
pub async fn read_message<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Option<Message>> {
    // Read fixed header (12 bytes)
    let mut fixed_header = [0u8; FIXED_HEADER_SIZE];
    match stream.read_exact(&mut fixed_header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    // Parse endian
    let endian = match fixed_header[0] {
        b'l' => Endian::Little,
        b'B' => Endian::Big,
        other => bail!("Invalid endian marker: {}", other),
    };

    let msg_type = MessageType::from(fixed_header[1]);
    let flags = fixed_header[2];
    let protocol_version = fixed_header[3];

    if protocol_version != 1 {
        bail!("Unsupported D-Bus protocol version: {}", protocol_version);
    }

    let body_len = endian.read_u32(&fixed_header[4..8]);
    let serial = endian.read_u32(&fixed_header[8..12]);

    if body_len > MAX_MESSAGE_SIZE {
        bail!("Message body too large: {} bytes", body_len);
    }

    // Read header fields array length (4 bytes)
    let mut array_len_buf = [0u8; 4];
    stream.read_exact(&mut array_len_buf).await?;
    let array_len = endian.read_u32(&array_len_buf);

    if array_len > MAX_MESSAGE_SIZE {
        bail!("Header fields array too large: {} bytes", array_len);
    }

    // Read header fields array
    let mut fields_buf = vec![0u8; array_len as usize];
    stream.read_exact(&mut fields_buf).await?;

    // Calculate padding to 8-byte boundary after header
    let header_end = MIN_HEADER_SIZE + array_len as usize;
    let padding = (8 - (header_end % 8)) % 8;
    let mut padding_buf = vec![0u8; padding];
    if padding > 0 {
        stream.read_exact(&mut padding_buf).await?;
    }

    // Read body
    let mut body = vec![0u8; body_len as usize];
    if body_len > 0 {
        stream.read_exact(&mut body).await?;
    }

    // Parse header fields
    let fields = parse_header_fields(&fields_buf, endian)?;

    // Reconstruct raw message
    let total_len = FIXED_HEADER_SIZE + 4 + array_len as usize + padding + body_len as usize;
    let mut raw = Vec::with_capacity(total_len);
    raw.extend_from_slice(&fixed_header);
    raw.extend_from_slice(&array_len_buf);
    raw.extend_from_slice(&fields_buf);
    raw.extend_from_slice(&padding_buf);
    raw.extend_from_slice(&body);

    Ok(Some(Message {
        header: MessageHeader {
            endian,
            msg_type,
            flags,
            serial,
            body_len,
            destination: fields.destination,
            reply_serial: fields.reply_serial,
            sender: fields.sender,
            interface: fields.interface,
            member: fields.member,
            path: fields.path,
            signature: fields.signature,
        },
        raw,
    }))
}

/// Parsed header fields.
#[derive(Debug, Default)]
struct ParsedHeaderFields {
    destination: Option<String>,
    reply_serial: Option<u32>,
    sender: Option<String>,
    interface: Option<String>,
    member: Option<String>,
    path: Option<String>,
    signature: Option<String>,
}

/// D-Bus header field as (code, value) tuple - signature a(yv)
type HeaderFieldTuple<'a> = (u8, Value<'a>);

/// Parse header fields using zvariant to extract destination, reply_serial, sender, interface, and member.
fn parse_header_fields(buf: &[u8], endian: Endian) -> Result<ParsedHeaderFields> {
    let z_endian = match endian {
        Endian::Little => ZEndian::Little,
        Endian::Big => ZEndian::Big,
    };

    // Header fields array signature is a(yv) - array of (byte, variant)
    // The buf already contains just the array data (after the 4-byte length prefix)
    // We need to prepend the array length back for zvariant to parse correctly
    let array_len = buf.len() as u32;
    let mut full_buf = Vec::with_capacity(4 + buf.len());
    match endian {
        Endian::Little => full_buf.extend_from_slice(&array_len.to_le_bytes()),
        Endian::Big => full_buf.extend_from_slice(&array_len.to_be_bytes()),
    }
    full_buf.extend_from_slice(buf);

    // Position 12: header fields array starts at byte 12 in D-Bus header
    // After 4-byte array length (at pos 12), we're at pos 16 which is 8-byte aligned
    // So no padding is needed between length and array content
    let ctxt = Context::new_dbus(z_endian, 12);
    let data = zvariant::serialized::Data::new(&full_buf, ctxt);

    let fields: Vec<HeaderFieldTuple> = match data.deserialize::<Vec<HeaderFieldTuple>>() {
        Ok((fields, _)) => fields,
        Err(e) => {
            tracing::warn!("Failed to parse header fields with zvariant: {}", e);
            return Ok(ParsedHeaderFields::default());
        }
    };

    let mut result = ParsedHeaderFields::default();
    for (code, value) in fields {
        match code {
            1 => {
                // PATH - ObjectPath, convert to String
                if let Value::ObjectPath(p) = &value {
                    result.path = Some(p.to_string());
                }
            }
            2 => result.interface = String::try_from(&value).ok(),
            3 => result.member = String::try_from(&value).ok(),
            5 => result.reply_serial = u32::try_from(&value).ok(),
            6 => result.destination = String::try_from(&value).ok(),
            7 => result.sender = String::try_from(&value).ok(),
            8 => {
                // SIGNATURE
                if let Value::Signature(s) = &value {
                    result.signature = Some(s.to_string());
                }
            }
            _ => { /* skip unknown fields */ }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endian_read_u32() {
        let le = Endian::Little;
        let be = Endian::Big;

        // 0x12345678 in little-endian
        let le_bytes = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(le.read_u32(&le_bytes), 0x12345678);

        // 0x12345678 in big-endian
        let be_bytes = [0x12, 0x34, 0x56, 0x78];
        assert_eq!(be.read_u32(&be_bytes), 0x12345678);
    }

    #[test]
    fn test_message_type() {
        assert_eq!(MessageType::from(1), MessageType::MethodCall);
        assert_eq!(MessageType::from(2), MessageType::MethodReturn);
        assert_eq!(MessageType::from(3), MessageType::Error);
        assert_eq!(MessageType::from(4), MessageType::Signal);
        assert_eq!(MessageType::from(0), MessageType::Invalid);
        assert_eq!(MessageType::from(255), MessageType::Invalid);
    }

    #[test]
    fn test_is_request_name() {
        // Create a message header for RequestName
        let msg = Message {
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
                member: Some("RequestName".to_string()),
                path: None,
                signature: None,
            },
            raw: vec![],
        };
        assert!(msg.is_request_name());

        // Not a RequestName - different member
        let msg2 = Message {
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
                member: Some("Hello".to_string()),
                path: None,
                signature: None,
            },
            raw: vec![],
        };
        assert!(!msg2.is_request_name());

        // Not a RequestName - different destination
        let msg3 = Message {
            header: MessageHeader {
                endian: Endian::Little,
                msg_type: MessageType::MethodCall,
                flags: 0,
                serial: 1,
                body_len: 0,
                destination: Some("org.example.Service".to_string()),
                reply_serial: None,
                sender: None,
                interface: Some("org.freedesktop.DBus".to_string()),
                member: Some("RequestName".to_string()),
                path: None,
                signature: None,
            },
            raw: vec![],
        };
        assert!(!msg3.is_request_name());
    }

    #[test]
    fn test_parse_header_fields_with_path() {
        use zvariant::{serialized::Context, to_bytes, ObjectPath, Value, LE};

        // Create header fields using zvariant serialization
        // Header fields are a(yv) - array of (byte field_code, variant value)
        let path = ObjectPath::try_from("/org/freedesktop/DBus").unwrap();
        let fields: Vec<(u8, Value)> = vec![
            (1, Value::ObjectPath(path)),                               // PATH
            (6, Value::Str("org.freedesktop.DBus".to_string().into())), // DESTINATION
        ];

        // Use position 12 to match actual D-Bus header layout
        // Header fields array starts at byte 12, after 4-byte length we're at 16 (8-aligned)
        let ctxt = Context::new_dbus(LE, 12);
        let encoded = to_bytes(ctxt, &fields).unwrap();

        // The encoded data structure from to_bytes at position 12:
        // - 4 bytes: array length
        // - N bytes: array content (no padding since 16 is 8-aligned)
        // parse_header_fields expects just the array content (no length prefix)
        let buf = &encoded[4..];

        let parsed = parse_header_fields(buf, Endian::Little).unwrap();
        assert_eq!(parsed.destination, Some("org.freedesktop.DBus".to_string()));
    }
}
