#[link(name = "kernel32")]
unsafe extern "system" {
    fn OutputDebugStringW(text: *const u16);
    fn MultiByteToWideChar(
        cp: u32,
        flags: u32,
        input: *const u8,
        len: i32,
        out: *mut u16,
        cap: i32,
    ) -> i32;
}
pub fn debug(message: &str) {
    let text: Vec<u16> = format!("TvRockDTV: {message}\n\0").encode_utf16().collect();
    unsafe { OutputDebugStringW(text.as_ptr()) }
}
pub fn decode_cp932(input: &[u8]) -> String {
    let Ok(len) = i32::try_from(input.len()) else {
        return String::new();
    };
    if len == 0 {
        return String::new();
    }
    let n = unsafe { MultiByteToWideChar(932, 1, input.as_ptr(), len, std::ptr::null_mut(), 0) };
    if n <= 0 {
        return "□".into();
    }
    let mut out = vec![0u16; n as usize];
    let n = unsafe { MultiByteToWideChar(932, 1, input.as_ptr(), len, out.as_mut_ptr(), n) };
    String::from_utf16_lossy(&out[..n.max(0) as usize])
}
