//! Internal C ABI. Client pointers are shared, immutable handles until dtv_free.
//! The C++ adapter retains ownership while any call or worker can reference them.
use crate::dtvclient::Client;
use std::{
    ffi::{CStr, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    time::Duration,
};
fn guard<T: Default>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_default()
}
#[repr(C)]
pub struct RemoteEvent {
    kind: u32,
    channel: u32,
    service: u32,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_new(
    device: *const c_char,
    server: *const c_char,
    parent: *const c_char,
) -> *mut Client {
    guard(|| {
        if device.is_null() || server.is_null() {
            return std::ptr::null_mut();
        }
        let (Ok(device), Ok(server)) = (
            unsafe { CStr::from_ptr(device) }.to_str(),
            unsafe { CStr::from_ptr(server) }.to_str(),
        ) else {
            return std::ptr::null_mut();
        };
        let parent = if parent.is_null() {
            None
        } else {
            let Ok(parent) = (unsafe { CStr::from_ptr(parent) }).to_str() else {
                return std::ptr::null_mut();
            };
            Some(parent)
        };
        match Client::start_with_parent(device, server, parent) {
            Ok(c) => Box::into_raw(Box::new(c)),
            Err(e) => {
                crate::windows::debug(&e);
                std::ptr::null_mut()
            }
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_free(p: *mut Client) {
    guard(|| {
        if !p.is_null() {
            unsafe { drop(Box::from_raw(p)) }
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_stop(p: *const Client) {
    guard(|| {
        if let Some(c) = unsafe { p.as_ref() } {
            c.stop()
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_wait_ready(p: *const Client, ms: u32) -> bool {
    guard(|| unsafe { p.as_ref() }.is_some_and(|c| c.wait_ready(Duration::from_millis(ms as u64))))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_tuner(p: *const Client) -> u32 {
    guard(|| unsafe { p.as_ref() }.map_or(255, Client::tuner_type))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_valid(p: *const Client, sid: u16) -> bool {
    guard(|| unsafe { p.as_ref() }.is_some_and(|c| c.valid_service(sid)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_generation(p: *const Client) -> u64 {
    guard(|| unsafe { p.as_ref() }.map_or(0, Client::generation))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_send(p: *const Client, text: *const c_char) -> bool {
    guard(|| {
        let Some(c) = (unsafe { p.as_ref() }) else {
            return false;
        };
        if text.is_null() {
            return false;
        }
        let Ok(text) = unsafe { CStr::from_ptr(text) }.to_str() else {
            return false;
        };
        c.send(text)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_reset_service(p: *const Client) {
    guard(|| {
        if let Some(c) = unsafe { p.as_ref() } {
            c.reset_service();
        }
    });
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_send_wide(p: *const Client, text: *const u16, len: usize) -> bool {
    guard(|| {
        let Some(c) = (unsafe { p.as_ref() }) else {
            return false;
        };
        if text.is_null() || len > 1024 * 1024 {
            return false;
        }
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
        c.send(&text)
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtv_next(
    p: *const Client,
    event: *mut RemoteEvent,
    path: *mut u16,
    capacity: usize,
    ms: u32,
) -> i32 {
    guard(|| {
        let Some(c) = (unsafe { p.as_ref() }) else {
            return -1;
        };
        if event.is_null() || path.is_null() || capacity < 32768 {
            return -1;
        }
        if let Some(e) = c.next_event(Duration::from_millis(ms as u64)) {
            let name: Vec<u16> = e.filename.encode_utf16().collect();
            if name.len() >= capacity {
                return -1;
            }
            unsafe {
                *event = RemoteEvent {
                    kind: e.kind,
                    channel: e.channel,
                    service: e.service,
                };
                std::ptr::copy_nonoverlapping(name.as_ptr(), path, name.len());
                *path.add(name.len()) = 0
            };
            1
        } else if c.stopped() {
            -1
        } else {
            0
        }
    })
}
