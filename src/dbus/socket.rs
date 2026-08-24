//! Unix socket IO with SCM_RIGHTS file-descriptor passing for D-Bus messages.
//!
//! D-Bus carries file descriptors via SCM_RIGHTS ancillary data attached to
//! the byte stream. The kernel attaches the fds of a `sendmsg` call to the
//! socket buffer holding those bytes, so on the receiving side fds and message
//! bytes arrive in the order the sender wrote them. We model this with:
//!
//! - `DbusReader`: a byte buffer plus a FIFO of received fds. Callers read
//!   complete messages from the byte side and then take as many fds from the
//!   FIFO as the message header declares (UNIX_FDS field).
//! - `DbusWriter`: sends one message per `sendmsg` with its fds attached.
//!   On a partial write the fds are already delivered to the kernel, so the
//!   remaining bytes are flushed without fds (same strategy as libdbus).

#![allow(dead_code)] // Prepared for future diagnostic use

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

/// Maximum fds accepted in a single message.
///
/// Matches the common D-Bus daemon limit (DBUS_MAXIMUM_MESSAGE_UNIX_FDS).
pub const MAX_FDS_PER_MESSAGE: usize = 64;

/// Size in bytes of the control-message buffer for one recvmsg call.
fn cmsg_buf_space() -> usize {
    cmsg_space_for(MAX_FDS_PER_MESSAGE * std::mem::size_of::<RawFd>())
}

/// CMSG_SPACE for `data_len` bytes of cmsg payload.
fn cmsg_space_for(data_len: usize) -> usize {
    // SAFETY: CMSG_SPACE is a pure layout computation.
    unsafe { libc::CMSG_SPACE(data_len as u32) as usize }
}

/// Read side of a D-Bus unix socket connection with fd support.
pub struct DbusReader {
    fd: AsyncFd<OwnedFd>,
    /// Cached raw fd for syscalls while the AsyncFd guard borrows `self.fd`.
    raw_fd: RawFd,
    /// Bytes received but not yet consumed by message parsing.
    buf: Vec<u8>,
    /// Start of unconsumed data in `buf`.
    start: usize,
    /// File descriptors received but not yet bound to a message.
    fdq: VecDeque<OwnedFd>,
    /// Scratch buffer for recvmsg, kept allocated across calls.
    rbuf: Box<[u8]>,
    /// Scratch control buffer for recvmsg.
    cmsg_buf: Box<[u8]>,
}

impl DbusReader {
    /// Create a reader over a socket fd (must be non-blocking).
    pub fn new(fd: OwnedFd) -> io::Result<Self> {
        let raw_fd = fd.as_raw_fd();
        Ok(Self {
            fd: AsyncFd::new(fd)?,
            raw_fd,
            buf: Vec::with_capacity(16 * 1024),
            start: 0,
            fdq: VecDeque::new(),
            rbuf: vec![0u8; 64 * 1024].into_boxed_slice(),
            cmsg_buf: vec![0u8; cmsg_buf_space()].into_boxed_slice(),
        })
    }

    /// Ensure at least `n` unconsumed bytes are buffered.
    /// Returns `false` on EOF before `n` bytes could be read.
    pub async fn fill(&mut self, n: usize) -> io::Result<bool> {
        while self.avail() < n {
            let got = self.recv_some().await?;
            if got == 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Borrow the first `n` unconsumed bytes. Only valid after a successful
    /// `fill(n)` and before the next `fill`/`consume`.
    pub fn head(&self, n: usize) -> &[u8] {
        &self.buf[self.start..self.start + n]
    }

    /// Consume `n` buffered bytes.
    pub fn consume(&mut self, n: usize) {
        self.start += n;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > 64 * 1024 {
            // Compact occasionally so the buffer does not grow unboundedly.
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }

    /// Number of buffered, unconsumed bytes.
    pub fn avail(&self) -> usize {
        self.buf.len() - self.start
    }

    /// Number of fds currently queued.
    pub fn fds_queued(&self) -> usize {
        self.fdq.len()
    }

    /// Take exactly `n` fds for the message being parsed.
    ///
    /// The kernel delivers fds in the same order messages are written, so a
    /// complete message always finds its fds at the front of the queue. A
    /// short queue means the peer violated the protocol.
    pub fn take_fds(&mut self, n: usize) -> io::Result<Vec<Arc<OwnedFd>>> {
        if self.fdq.len() < n {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "message declares {} fds but only {} received",
                    n,
                    self.fdq.len()
                ),
            ));
        }
        Ok((0..n)
            .map(|_| Arc::new(self.fdq.pop_front().unwrap()))
            .collect())
    }

    /// Receive more bytes (and any fds) into the buffer. Returns bytes read;
    /// `0` means EOF.
    async fn recv_some(&mut self) -> io::Result<usize> {
        loop {
            let mut guard = self.fd.readable_mut().await?;
            // Field-level borrows only: the guard owns `self.fd`, the call
            // borrows the other fields.
            let result = recvmsg_once(
                self.raw_fd,
                &mut self.rbuf,
                &mut self.cmsg_buf,
                &mut self.fdq,
                &mut self.buf,
            );
            match result {
                // Do NOT clear_ready on success: the fd is edge-triggered, so
                // dropping the ready bit while data remains buffered would
                // starve the next wait (no further edge would ever fire).
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    guard.clear_ready();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Write side of a D-Bus unix socket connection with fd support.
pub struct DbusWriter {
    fd: AsyncFd<OwnedFd>,
    /// Cached raw fd for syscalls while the AsyncFd guard borrows `self.fd`.
    raw_fd: RawFd,
    /// Scratch control buffer for sendmsg.
    cmsg_buf: Box<[u8]>,
}

impl DbusWriter {
    /// Create a writer over a socket fd (must be non-blocking).
    pub fn new(fd: OwnedFd) -> io::Result<Self> {
        let raw_fd = fd.as_raw_fd();
        Ok(Self {
            fd: AsyncFd::new(fd)?,
            raw_fd,
            cmsg_buf: vec![0u8; cmsg_buf_space()].into_boxed_slice(),
        })
    }

    /// Send one complete message, attaching `fds` via SCM_RIGHTS.
    ///
    /// The whole message is flushed before returning. If the socket accepts
    /// only part of it, the fds have already been delivered by the kernel and
    /// the remaining bytes are written without them.
    pub async fn send(&mut self, data: &[u8], fds: &[Arc<OwnedFd>]) -> io::Result<()> {
        if fds.len() > MAX_FDS_PER_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "message carries {} fds, max {}",
                    fds.len(),
                    MAX_FDS_PER_MESSAGE
                ),
            ));
        }

        let mut off = 0;
        let mut fds_pending = !fds.is_empty();
        while off < data.len() {
            let chunk = &data[off..];
            let send_fds: &[Arc<OwnedFd>] = if fds_pending { fds } else { &[] };
            let n = self.sendmsg_some(chunk, send_fds).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write any bytes",
                ));
            }
            off += n;
            // Any accepted byte means the cmsg (fds) was delivered.
            fds_pending = false;
        }
        Ok(())
    }

    /// One sendmsg attempt, waiting for writability on WouldBlock.
    async fn sendmsg_some(&mut self, data: &[u8], fds: &[Arc<OwnedFd>]) -> io::Result<usize> {
        loop {
            let mut guard = self.fd.writable_mut().await?;
            let result = sendmsg_once(self.raw_fd, data, fds, &mut self.cmsg_buf);
            match result {
                // Keep the ready bit on success (edge-triggered fd).
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    guard.clear_ready();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// One non-blocking recvmsg call; appends bytes to `buf` and fds to `fdq`.
fn recvmsg_once(
    raw_fd: RawFd,
    rbuf: &mut [u8],
    cmsg_buf: &mut [u8],
    fdq: &mut VecDeque<OwnedFd>,
    buf: &mut Vec<u8>,
) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: rbuf.as_mut_ptr() as *mut libc::c_void,
        iov_len: rbuf.len(),
    };

    // SAFETY: msghdr points at buffers owned by the caller, valid for the call.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    // `as _`: msg_controllen is size_t on glibc but socklen_t on musl
    msg.msg_controllen = cmsg_buf.len() as _;

    // CLOEXEC so received fds never leak into child processes.
    let n = unsafe { libc::recvmsg(raw_fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Ok(0);
    }

    // Adopt every descriptor the kernel installed BEFORE deciding whether to
    // keep the message: on any error path they are closed by dropping
    // `received`, and a leaked fd is invisible until the process runs out.
    // SAFETY: msg.msg_control holds kernel-written cmsg records inside our
    // buffer; CMSG_FIRSTHDR/CMSG_NXTHDR iterate them in bounds.
    let received = unsafe { adopt_scm_rights(&msg) };

    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control message truncated: too many fds in flight",
        ));
    }

    if fdq.len() + received.len() > MAX_FDS_PER_MESSAGE {
        // The kernel caps cmsgs to our buffer, so this is a peer sending far
        // too many fds.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "received more fds than allowed",
        ));
    }

    fdq.extend(received);

    buf.extend_from_slice(&rbuf[..n as usize]);
    Ok(n as usize)
}

/// Take ownership of every SCM_RIGHTS descriptor in a received message.
///
/// # Safety
/// `msg` must be a msghdr just filled in by `recvmsg`, whose control buffer
/// is still alive and unmodified.
unsafe fn adopt_scm_rights(msg: &libc::msghdr) -> Vec<OwnedFd> {
    let mut adopted = Vec::new();
    let mut cmsg = libc::CMSG_FIRSTHDR(msg);
    while !cmsg.is_null() {
        if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
            let data_len = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
            let count = data_len / std::mem::size_of::<RawFd>();
            let raw_fds = std::slice::from_raw_parts(libc::CMSG_DATA(cmsg) as *const RawFd, count);
            adopted.extend(raw_fds.iter().map(|&raw| OwnedFd::from_raw_fd(raw)));
        }
        cmsg = libc::CMSG_NXTHDR(msg, cmsg);
    }
    adopted
}

/// One non-blocking sendmsg with SCM_RIGHTS control data.
fn sendmsg_once(
    raw_fd: RawFd,
    data: &[u8],
    fds: &[Arc<OwnedFd>],
    cmsg_buf: &mut [u8],
) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: data.as_ptr() as *mut libc::c_void,
        iov_len: data.len(),
    };

    // SAFETY: msghdr points at iov (borrowed for the call) and cmsg_buf,
    // which we fill with one SCM_RIGHTS record below.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;

    if !fds.is_empty() {
        let data_len = fds.len() * std::mem::size_of::<RawFd>();
        // Report only the control bytes actually used; handing the kernel the
        // full scratch buffer makes it parse trailing zeros as another cmsg
        // and fail with EINVAL.
        let used = cmsg_space_for(data_len);
        msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
        // `as _`: msg_controllen is size_t on glibc but socklen_t on musl
        msg.msg_controllen = used as _;
        // SAFETY: control buffer is zeroed and sized via CMSG_SPACE; the
        // record is filled within bounds and consumed by the sendmsg below.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if cmsg.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cmsg space too small",
                ));
            }
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            // `as _`: cmsg_len is size_t on glibc but socklen_t on musl
            (*cmsg).cmsg_len = libc::CMSG_LEN(data_len as u32) as _;

            let cmsg_data = libc::CMSG_DATA(cmsg);
            let dst = std::slice::from_raw_parts_mut(cmsg_data, data_len);
            for (slot, fd) in dst
                .chunks_exact_mut(std::mem::size_of::<RawFd>())
                .zip(fds.iter())
            {
                slot.copy_from_slice(&fd.as_raw_fd().to_ne_bytes());
            }
        }
    }

    let n = unsafe { libc::sendmsg(raw_fd, &msg, libc::MSG_NOSIGNAL) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// Duplicate a socket fd (dup) for use with `AsyncFd`.
pub fn dup_fd(fd: impl AsFd) -> io::Result<OwnedFd> {
    // SAFETY: dup of a live fd; on success the new fd is owned by us.
    let raw = unsafe { libc::dup(fd.as_fd().as_raw_fd()) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socketpair() -> (
        std::os::unix::net::UnixStream,
        std::os::unix::net::UnixStream,
    ) {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        b.set_nonblocking(true).unwrap();
        (a, b)
    }

    fn tokio_rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct TempFile {
        file: std::fs::File,
        path: std::path::PathBuf,
    }

    impl TempFile {
        fn new(content: &[u8]) -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "dbus-router-sock-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            // O_RDWR: the received fd must be readable to verify content
            // (File::create opens O_WRONLY and read(2) on it is EBADF).
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            file.write_all(content).unwrap();
            Self { file, path }
        }

        fn fd(&self) -> OwnedFd {
            use std::os::fd::FromRawFd;
            // dup the file fd; Drop will still clean up the original file
            let raw = unsafe { libc::dup(self.file.as_raw_fd()) };
            if raw < 0 {
                panic!("dup failed");
            }
            unsafe { OwnedFd::from_raw_fd(raw) }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn read_fd(fd: &OwnedFd) -> String {
        let raw = fd.as_raw_fd();
        let clone = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
        assert!(clone >= 0, "F_DUPFD_CLOEXEC({raw}) failed");
        let off = unsafe { libc::lseek(clone, 0, libc::SEEK_SET) };
        assert!(off >= 0, "lseek({clone}) failed");
        let mut out = vec![0u8; 64];
        let n = unsafe { libc::read(clone, out.as_mut_ptr() as *mut _, out.len()) };
        assert!(
            n >= 0,
            "read({clone}) failed: {}",
            std::io::Error::last_os_error()
        );
        unsafe { libc::close(clone) };
        String::from_utf8_lossy(&out[..n as usize]).into_owned()
    }

    #[test]
    fn test_send_receive_message_with_fd() {
        let (a, b) = socketpair();
        let tmp = TempFile::new(b"hello-fd");

        let rt = tokio_rt();
        rt.block_on(async move {
            let mut writer = DbusWriter::new(dup_fd(&a).unwrap()).unwrap();
            let mut reader = DbusReader::new(dup_fd(&b).unwrap()).unwrap();

            let raw = b"l\x01\x00\x01\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00";
            writer.send(raw, &[Arc::new(tmp.fd())]).await.unwrap();

            assert!(reader.fill(raw.len()).await.unwrap());
            assert_eq!(reader.head(raw.len()), raw);
            assert_eq!(reader.fds_queued(), 1);

            let fds = reader.take_fds(1).unwrap();
            reader.consume(raw.len());
            assert_eq!(read_fd(&fds[0]), "hello-fd");
        });
    }

    #[test]
    fn test_fd_fifo_order_across_messages() {
        let (a, b) = socketpair();
        let t1 = TempFile::new(b"one");
        let t2 = TempFile::new(b"two");

        let rt = tokio_rt();
        rt.block_on(async move {
            let mut writer = DbusWriter::new(dup_fd(&a).unwrap()).unwrap();
            let mut reader = DbusReader::new(dup_fd(&b).unwrap()).unwrap();

            writer.send(b"msg-one", &[Arc::new(t1.fd())]).await.unwrap();
            writer.send(b"msg-two", &[Arc::new(t2.fd())]).await.unwrap();

            // The kernel attaches ancillary data per skb, and one sendmsg
            // produces one skb: a recvmsg returns at most the bytes of the
            // first fd-carrying skb it dequeues. So each message is filled,
            // fd-bound, and consumed one at a time.
            assert!(reader.fill(7).await.unwrap());
            let first = reader.take_fds(1).unwrap();
            reader.consume(7);
            assert!(reader.fill(7).await.unwrap());
            let second = reader.take_fds(1).unwrap();
            reader.consume(7);

            assert_eq!(read_fd(&first[0]), "one");
            assert_eq!(read_fd(&second[0]), "two");
        });
    }

    #[test]
    fn test_take_fds_short_queue_is_error() {
        let (a, _b) = socketpair();
        let rt = tokio_rt();
        rt.block_on(async {
            let mut reader = DbusReader::new(dup_fd(&a).unwrap()).unwrap();
            assert!(reader.take_fds(1).is_err());
        });
    }

    #[test]
    fn test_send_without_fds() {
        let (a, b) = socketpair();

        let rt = tokio_rt();
        rt.block_on(async move {
            let mut writer = DbusWriter::new(dup_fd(&a).unwrap()).unwrap();
            let mut reader = DbusReader::new(dup_fd(&b).unwrap()).unwrap();

            writer.send(b"plain", &[]).await.unwrap();
            assert!(reader.fill(5).await.unwrap());
            assert_eq!(reader.avail(), 5);
            assert_eq!(reader.fds_queued(), 0);
        });
    }

    #[test]
    fn test_too_many_fds_rejected() {
        let (a, _b) = socketpair();
        let rt = tokio_rt();
        rt.block_on(async move {
            let mut writer = DbusWriter::new(dup_fd(&a).unwrap()).unwrap();
            let fds: Vec<Arc<OwnedFd>> = (0..=MAX_FDS_PER_MESSAGE)
                .map(|i| Arc::new(TempFile::new(format!("x{}", i).as_bytes()).fd()))
                .collect();
            assert!(writer.send(b"x", &fds).await.is_err());
        });
    }
}
