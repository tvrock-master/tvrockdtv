//! Nonblocking SRT caller; only the stream worker accesses the socket.
use std::{ffi::CString, io, net::SocketAddrV4};

#[derive(Clone, PartialEq, Eq)]
pub struct Options {
    pub latency_ms: u32,
    pub passphrase: String,
}
impl Options {
    pub fn valid(&self) -> bool {
        self.latency_ms <= 60000
            && (self.passphrase.is_empty()
                || ((10..=79).contains(&self.passphrase.len())
                    && self.passphrase.bytes().all(|c| (32..=126).contains(&c))))
    }
}
#[cfg(windows)]
unsafe extern "C" {
    fn tr_srt_open(ip: *const u8, port: u16, latency: i32, secret: *const std::ffi::c_char) -> i32;
    fn tr_srt_send(socket: i32, bytes: *const u8, size: i32) -> i32;
    fn tr_srt_state(socket: i32) -> i32;
    fn tr_srt_close(socket: i32);
}
pub struct Socket(i32);
impl Socket {
    #[cfg(windows)]
    pub fn open(endpoint: SocketAddrV4, options: &Options) -> io::Result<Self> {
        let secret =
            CString::new(options.passphrase.as_str()).map_err(|_| io::ErrorKind::InvalidInput)?;
        let socket = unsafe {
            tr_srt_open(
                endpoint.ip().octets().as_ptr(),
                endpoint.port(),
                options.latency_ms as i32,
                secret.as_ptr(),
            )
        };
        if socket < 0 {
            Err(io::ErrorKind::ConnectionRefused.into())
        } else {
            Ok(Self(socket))
        }
    }
    #[cfg(not(windows))]
    pub fn open(_: SocketAddrV4, _: &Options) -> io::Result<Self> {
        Err(io::ErrorKind::Unsupported.into())
    }
    #[cfg(windows)]
    pub fn send(&self, bytes: &[u8]) -> io::Result<usize> {
        match unsafe { tr_srt_send(self.0, bytes.as_ptr(), bytes.len() as i32) } {
            0 => Err(io::ErrorKind::WouldBlock.into()),
            n if n > 0 => Ok(n as usize),
            _ => Err(io::ErrorKind::ConnectionReset.into()),
        }
    }
    #[cfg(not(windows))]
    pub fn send(&self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }
    #[cfg(windows)]
    pub fn ready(&self) -> bool {
        unsafe { tr_srt_state(self.0) == 1 }
    }
    #[cfg(not(windows))]
    pub fn ready(&self) -> bool {
        false
    }
    #[cfg(windows)]
    pub fn broken(&self) -> bool {
        unsafe { tr_srt_state(self.0) < 0 }
    }
    #[cfg(not(windows))]
    pub fn broken(&self) -> bool {
        true
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            tr_srt_close(self.0)
        };
    }
}
