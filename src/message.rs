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
    fn read_u32(&self, buf: &[u8]) -> u32 {
        let arr: [u8; 4] = buf[..4].try_into().unwrap();
        match self {
            Endian::Little => u32::from_le_bytes(arr),
            Endian::Big => u32::from_be_bytes(arr),
        }
    }
}

/// D-Bus header field codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HeaderField {
    Invalid = 0,
    Path = 1,
    Interface = 2,
    Member = 3,
    ErrorName = 4,
    ReplySerial = 5,
    Destination = 6,
    Sender = 7,
    Signature = 8,
    UnixFds = 9,
}

impl From<u8> for HeaderField {
    fn from(v: u8) -> Self {
        match v {
            1 => HeaderField::Path,
            2 => HeaderField::Interface,
            3 => HeaderField::Member,
            4 => HeaderField::ErrorName,
            5 => HeaderField::ReplySerial,
            6 => HeaderField::Destination,
            7 => HeaderField::Sender,
            8 => HeaderField::Signature,
            9 => HeaderField::UnixFds,
            _ => HeaderField::Invalid,
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
}

/// A complete D-Bus message (header + body as raw bytes).
#[derive(Debug)]
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

    /// Extract the service name from RequestName body.
    /// The body format is: STRING (name) + UINT32 (flags)
    pub fn extract_name_from_body(&self) -> Option<String> {
        // Body starts after header (aligned to 8 bytes)
        // Find body start position in raw message
        let fixed_header_size = 12;
        let array_len = self.header.endian.read_u32(&self.raw[fixed_header_size..]);
        let header_end = 16 + array_len as usize;
        let padding = (8 - (header_end % 8)) % 8;
        let body_start = header_end + padding;

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
}

/// Parse header fields to extract destination, reply_serial, sender, interface, and member.
fn parse_header_fields(buf: &[u8], endian: Endian) -> Result<ParsedHeaderFields> {
    let mut fields = ParsedHeaderFields::default();

    let mut pos = 0;
    while pos < buf.len() {
        // Align to 8-byte boundary for struct
        pos = align_to(pos, 8);
        if pos >= buf.len() {
            break;
        }

        // Field code (1 byte)
        let field_code = HeaderField::from(buf[pos]);
        pos += 1;

        if pos >= buf.len() {
            break;
        }

        // Signature (1 byte length + signature string + null)
        let sig_len = buf[pos] as usize;
        pos += 1;

        if pos + sig_len >= buf.len() {
            break;
        }

        let signature = &buf[pos..pos + sig_len];
        pos += sig_len + 1; // +1 for null terminator

        // Align to value type alignment
        match (field_code, signature) {
            (HeaderField::Destination, b"s")
            | (HeaderField::Sender, b"s")
            | (HeaderField::Interface, b"s")
            | (HeaderField::Member, b"s") => {
                pos = align_to(pos, 4);
                if pos + 4 > buf.len() {
                    break;
                }
                let str_len = endian.read_u32(&buf[pos..pos + 4]) as usize;
                pos += 4;
                if pos + str_len > buf.len() {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[pos..pos + str_len]).to_string();
                pos += str_len + 1; // +1 for null terminator

                match field_code {
                    HeaderField::Destination => fields.destination = Some(s),
                    HeaderField::Sender => fields.sender = Some(s),
                    HeaderField::Interface => fields.interface = Some(s),
                    HeaderField::Member => fields.member = Some(s),
                    _ => unreachable!(),
                }
            }
            (HeaderField::ReplySerial, b"u") => {
                pos = align_to(pos, 4);
                if pos + 4 > buf.len() {
                    break;
                }
                fields.reply_serial = Some(endian.read_u32(&buf[pos..pos + 4]));
                pos += 4;
            }
            _ => {
                // Skip other fields - we need to parse based on signature
                // For simplicity, skip unknown fields by finding next 8-byte boundary
                pos = align_to(pos, 8);
            }
        }
    }

    Ok(fields)
}

/// Align position to the given boundary.
fn align_to(pos: usize, alignment: usize) -> usize {
    (pos + alignment - 1) & !(alignment - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_to() {
        assert_eq!(align_to(0, 8), 0);
        assert_eq!(align_to(1, 8), 8);
        assert_eq!(align_to(7, 8), 8);
        assert_eq!(align_to(8, 8), 8);
        assert_eq!(align_to(9, 8), 16);

        assert_eq!(align_to(0, 4), 0);
        assert_eq!(align_to(1, 4), 4);
        assert_eq!(align_to(3, 4), 4);
        assert_eq!(align_to(4, 4), 4);
    }

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
            },
            raw: vec![],
        };
        assert!(!msg3.is_request_name());
    }
}
