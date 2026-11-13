//! Raw pointers are confined to this internal, guarded C ABI.
use crate::stream::{Status, Streamer};
use std::{
    ffi::{CStr, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
};
fn guard<T: Default>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_default()
}
unsafe fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(p) }.to_str().ok()
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn udp_new(options: u32) -> *mut Streamer {
    guard(|| Box::into_raw(Box::new(Streamer::new(options & 0x100 != 0))))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_free(p: *mut Streamer) {
    if !p.is_null() {
        guard(|| unsafe { drop(Box::from_raw(p)) })
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_start(
    p: *const Streamer,
    token: *const c_char,
    address: *const c_char,
    port: u16,
    flags: u32,
) -> bool {
    guard(|| {
        let (Some(p), Some(token), Some(address)) =
            (unsafe { p.as_ref() }, unsafe { text(token) }, unsafe {
                text(address)
            })
        else {
            return false;
        };
        p.start(token, address, port, flags)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_stop(p: *const Streamer, token: *const c_char) -> bool {
    guard(|| {
        let (Some(p), Some(token)) = (unsafe { p.as_ref() }, unsafe { text(token) }) else {
            return false;
        };
        p.stop(token)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_pace(p: *const Streamer, token: *const c_char) -> bool {
    guard(|| {
        let (Some(p), Some(token)) = (unsafe { p.as_ref() }, unsafe { text(token) }) else {
            return false;
        };
        p.pace(token)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_is_paced(p: *const Streamer, token: *const c_char) -> bool {
    guard(|| {
        let (Some(p), Some(token)) = (unsafe { p.as_ref() }, unsafe { text(token) }) else {
            return false;
        };
        p.is_paced(token)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_reset(p: *const Streamer, service: u16) {
    guard(|| {
        if let Some(p) = unsafe { p.as_ref() } {
            p.reset(service)
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_feed(p: *const Streamer, data: *const u8, n: usize) -> bool {
    guard(|| {
        let Some(p) = (unsafe { p.as_ref() }) else {
            return false;
        };
        if n == 0 {
            return true;
        }
        if data.is_null() || n > isize::MAX as usize {
            return false;
        }
        p.feed(unsafe { std::slice::from_raw_parts(data, n) })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_status(
    p: *const Streamer,
    token: *const c_char,
    out: *mut Status,
) -> bool {
    guard(|| {
        let (Some(p), Some(token), Some(out)) =
            (unsafe { p.as_ref() }, unsafe { text(token) }, unsafe {
                out.as_mut()
            })
        else {
            return false;
        };
        let Some(status) = p.status(token) else {
            return false;
        };
        *out = status;
        true
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn udp_service(p: *const Streamer, service: u16) {
    guard(|| {
        if let Some(p) = unsafe { p.as_ref() } {
            p.set_service(service)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_start_srt(
    p: *const Streamer,
    token: *const c_char,
    address: *const c_char,
    port: u16,
    flags: u32,
    latency_ms: u32,
    secret: *const c_char,
) -> bool {
    guard(|| {
        let (Some(p), Some(token), Some(address), Some(secret)) = (
            unsafe { p.as_ref() },
            unsafe { text(token) },
            unsafe { text(address) },
            unsafe { text(secret) },
        ) else {
            return false;
        };
        p.start_transport(
            token,
            address,
            port,
            flags,
            Some(crate::srt::Options {
                latency_ms,
                passphrase: secret.to_owned(),
            }),
        )
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_is_srt(p: *const Streamer, token: *const c_char) -> bool {
    guard(|| {
        let (Some(p), Some(token)) = (unsafe { p.as_ref() }, unsafe { text(token) }) else {
            return false;
        };
        p.is_srt(token)
    })
}
