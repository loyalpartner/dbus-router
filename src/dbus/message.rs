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

use super::socket::{self, DbusReader};
use crate::error::{Error, Result};
use std::os::fd::OwnedFd;
use std::sync::Arc;
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
    /// Number of unix fds the message declares (header field 9).
    pub unix_fds: Option<u32>,
}

/// A complete D-Bus message (header + body as raw bytes).
#[derive(Debug, Clone)]
pub struct Message {
    pub header: MessageHeader,
    /// Raw message bytes including header and body
    pub raw: Vec<u8>,
    /// File descriptors attached to this message via SCM_RIGHTS, in the order
    /// the sender attached them. Shared via `Arc` so cloned/rewritten
    /// messages can forward the same fds without duplicating them.
    pub fds: Vec<Arc<OwnedFd>>,
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
/// The fixed 16-byte prologue every D-Bus message starts with: the 12-byte
/// fixed header plus the length of the header-fields array. Parsed in one
/// place so the two readers below cannot drift apart on protocol details.
struct Prologue {
    endian: Endian,
    msg_type: MessageType,
    flags: u8,
    serial: u32,
    body_len: u32,
    array_len: usize,
}

impl Prologue {
    /// `head` must hold at least `MIN_HEADER_SIZE` bytes.
    fn parse(head: &[u8]) -> Result<Self> {
        let endian = match head[0] {
            b'l' => Endian::Little,
            b'B' => Endian::Big,
            other => return Err(Error::Protocol(format!("Invalid endian marker: {}", other))),
        };

        let protocol_version = head[3];
        if protocol_version != 1 {
            return Err(Error::Protocol(format!(
                "Unsupported D-Bus protocol version: {}",
                protocol_version
            )));
        }

        let body_len = endian.read_u32(&head[4..8]);
        if body_len > MAX_MESSAGE_SIZE {
            return Err(Error::Protocol(format!(
                "Message body too large: {} bytes",
                body_len
            )));
        }

        let array_len = endian.read_u32(&head[12..16]);
        if array_len > MAX_MESSAGE_SIZE {
            return Err(Error::Protocol(format!(
                "Header fields array too large: {} bytes",
                array_len
            )));
        }

        Ok(Self {
            endian,
            msg_type: MessageType::from(head[1]),
            flags: head[2],
            serial: endian.read_u32(&head[8..12]),
            body_len,
            array_len: array_len as usize,
        })
    }

    /// Offset just past the header-fields array.
    fn header_end(&self) -> usize {
        MIN_HEADER_SIZE + self.array_len
    }

    /// Padding that aligns the body to an 8-byte boundary.
    fn padding(&self) -> usize {
        (8 - (self.header_end() % 8)) % 8
    }

    /// Total wire size of the message.
    fn total_len(&self) -> usize {
        self.header_end() + self.padding() + self.body_len as usize
    }

    fn into_header(self, fields: ParsedHeaderFields) -> MessageHeader {
        MessageHeader {
            endian: self.endian,
            msg_type: self.msg_type,
            flags: self.flags,
            serial: self.serial,
            body_len: self.body_len,
            destination: fields.destination,
            reply_serial: fields.reply_serial,
            sender: fields.sender,
            interface: fields.interface,
            member: fields.member,
            path: fields.path,
            signature: fields.signature,
            unix_fds: fields.unix_fds,
        }
    }
}

/// Read one message from a plain byte stream. Used where no descriptors can
/// arrive (the auth/Hello handshake); `read_message_from` is the fd-aware
/// counterpart used for the forwarding path.
pub async fn read_message<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Option<Message>> {
    let mut head = [0u8; MIN_HEADER_SIZE];
    // EOF exactly here means the peer closed between messages - not an error.
    match stream.read_exact(&mut head[..FIXED_HEADER_SIZE]).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    stream.read_exact(&mut head[FIXED_HEADER_SIZE..]).await?;

    let prologue = Prologue::parse(&head)?;

    let mut fields_buf = vec![0u8; prologue.array_len];
    stream.read_exact(&mut fields_buf).await?;

    let mut padding_buf = vec![0u8; prologue.padding()];
    if !padding_buf.is_empty() {
        stream.read_exact(&mut padding_buf).await?;
    }

    let mut body = vec![0u8; prologue.body_len as usize];
    if !body.is_empty() {
        stream.read_exact(&mut body).await?;
    }

    let fields = parse_header_fields(&fields_buf, prologue.endian)?;

    let mut raw = Vec::with_capacity(prologue.total_len());
    raw.extend_from_slice(&head);
    raw.extend_from_slice(&fields_buf);
    raw.extend_from_slice(&padding_buf);
    raw.extend_from_slice(&body);

    Ok(Some(Message {
        header: prologue.into_header(fields),
        raw,
        fds: Vec::new(),
    }))
}

/// Adopt exactly the descriptors the header declared. The count is capped:
/// a peer claiming more than the daemon limit is refused rather than trusted.
fn take_declared_fds(reader: &mut DbusReader, declared: Option<u32>) -> Result<Vec<Arc<OwnedFd>>> {
    let declared = declared.unwrap_or(0) as usize;
    if declared > socket::MAX_FDS_PER_MESSAGE {
        return Err(Error::Protocol(format!(
            "Message declares {} fds, max {}",
            declared,
            socket::MAX_FDS_PER_MESSAGE
        )));
    }
    if declared == 0 {
        return Ok(Vec::new());
    }
    reader
        .take_fds(declared)
        .map_err(|e| Error::Protocol(e.to_string()))
}

/// Read one message together with any SCM_RIGHTS descriptors it carries.
pub async fn read_message_from(reader: &mut DbusReader) -> Result<Option<Message>> {
    if !reader.fill(MIN_HEADER_SIZE).await? {
        return Ok(None);
    }

    let prologue = Prologue::parse(reader.head(MIN_HEADER_SIZE))?;
    let total = prologue.total_len();

    if !reader.fill(total).await? {
        return Err(Error::Protocol(
            "Connection closed in the middle of a message".to_string(),
        ));
    }

    let raw = reader.head(total).to_vec();
    reader.consume(total);

    let fields = parse_header_fields(
        &raw[MIN_HEADER_SIZE..prologue.header_end()],
        prologue.endian,
    )?;
    let fds = take_declared_fds(reader, fields.unix_fds)?;

    Ok(Some(Message {
        header: prologue.into_header(fields),
        raw,
        fds,
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
    unix_fds: Option<u32>,
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
            9 => result.unix_fds = u32::try_from(&value).ok(),
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
                unix_fds: None,
            },
            raw: vec![],
            fds: Vec::new(),
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
                unix_fds: None,
            },
            raw: vec![],
            fds: Vec::new(),
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
                unix_fds: None,
            },
            raw: vec![],
            fds: Vec::new(),
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
