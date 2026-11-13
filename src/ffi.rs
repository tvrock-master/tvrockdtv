//! Only this module accepts raw pointers. The host serializes calls per engine.
use crate::{arib, engine::Engine};
use std::{
    ffi::{c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    slice,
};
type Emit = unsafe extern "C" fn(*const c_char) -> bool;
fn guarded<T: Default>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_default()
}
fn emit(messages: Vec<String>, send: Emit) {
    for s in messages {
        if let Ok(s) = std::ffi::CString::new(s) {
            unsafe {
                send(s.as_ptr());
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn tr_new() -> *mut Engine {
    guarded(|| Box::into_raw(Box::<Engine>::default()))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_free(p: *mut Engine) {
    if !p.is_null() {
        guarded(|| unsafe { drop(Box::from_raw(p)) })
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_reset(p: *mut Engine, ch: u32, sv: u16, id: u32) {
    guarded(|| {
        if let Some(p) = unsafe { p.as_mut() } {
            p.reset(ch, sv, id)
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_process(p: *mut Engine, data: *const u8, n: usize, send: Emit) -> bool {
    guarded(|| {
        let Some(p) = (unsafe { p.as_mut() }) else {
            return false;
        };
        if n == 0 {
            return true;
        }
        if data.is_null() || n > isize::MAX as usize {
            return false;
        }
        let bytes = unsafe { slice::from_raw_parts(data, n) };
        emit(p.process(bytes), send);
        true
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_status(
    p: *mut Engine,
    a: u32,
    b: f32,
    c: u32,
    d: u32,
    e: u32,
    send: Emit,
) {
    guarded(|| {
        if let Some(p) = unsafe { p.as_mut() } {
            emit(p.status(a, b, c, d, e), send)
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_flush(
    p: *mut Engine,
    sid: u16,
    send: Emit,
    valid: unsafe extern "C" fn(u16) -> bool,
) {
    guarded(|| {
        if let Some(p) = unsafe { p.as_mut() } {
            if let Some(s) = p.events.current(sid) {
                emit(vec![s], send)
            }
            p.logos.send_pending(
                |sid| unsafe { valid(sid) },
                |s| std::ffi::CString::new(s).is_ok_and(|s| unsafe { send(s.as_ptr()) }),
            );
            p.events.send_scheduled(
                std::time::Instant::now(),
                crate::time::now(),
                |sid| unsafe { valid(sid) },
                |s| std::ffi::CString::new(s).is_ok_and(|s| unsafe { send(s.as_ptr()) }),
            );
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_resend(p: *mut Engine) {
    guarded(|| {
        if let Some(p) = unsafe { p.as_mut() } {
            p.events.resend();
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_time(p: *const Engine) -> i64 {
    guarded(|| {
        unsafe { p.as_ref() }
            .and_then(|p| p.broadcast)
            .map_or(0, |(t, received)| t + crate::time::now() - received)
    })
}
#[repr(C)]
pub struct Format {
    pos: u32,
    size: u32,
    color: u8,
    background: u8,
    raster: u8,
}
type Drcs = unsafe extern "C" fn(*mut c_void, u16) -> *const u16;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_decode(
    dst: *mut u16,
    capacity: u32,
    src: *const u8,
    len: u32,
    flags: u32,
    context: *mut c_void,
    map: Option<Drcs>,
    format: Option<unsafe extern "C" fn(*mut c_void, *const Format)>,
) -> u32 {
    guarded(|| {
        if dst.is_null() || capacity == 0 {
            return 0;
        }
        unsafe { *dst = 0 };
        if src.is_null() {
            return 0;
        }
        let data = unsafe { slice::from_raw_parts(src, len as usize) };
        let d = arib::decode_with_drcs(data, flags, capacity as usize, &|code| {
            let map = map?;
            let ptr = unsafe { map(context, code) };
            if ptr.is_null() {
                return None;
            }
            let mut len = 0;
            while len < 65536 && unsafe { *ptr.add(len) } != 0 {
                len += 1
            }
            Some(String::from_utf16_lossy(unsafe {
                slice::from_raw_parts(ptr, len)
            }))
        });
        let out: Vec<u16> = d.text.encode_utf16().collect();
        unsafe {
            std::ptr::copy_nonoverlapping(out.as_ptr(), dst, out.len());
            *dst.add(out.len()) = 0;
        }
        if let Some(f) = format {
            for v in d.formats {
                unsafe {
                    f(
                        context,
                        &Format {
                            pos: v.pos as u32,
                            size: v.size as u32,
                            color: v.color,
                            background: v.background,
                            raster: v.raster,
                        },
                    )
                }
            }
        }
        out.len() as u32
    })
}
