//! ARIB broadcasts use JST, regardless of the host's time zone.
pub fn bcd(b: u8, max: i64) -> Option<i64> {
    let n = (b >> 4) as i64 * 10 + (b & 15) as i64;
    (b >> 4 <= 9 && b & 15 <= 9 && n <= max).then_some(n)
}
pub fn timestamp(b: &[u8]) -> Option<i64> {
    if b.len() != 5 || b == [255; 5] {
        return None;
    }
    Some(
        (crate::ts::be16(b) as i64 - 40587) * 86400
            + bcd(b[2], 23)? * 3600
            + bcd(b[3], 59)? * 60
            + bcd(b[4], 59)?
            - 9 * 3600,
    )
}
pub fn duration(b: &[u8]) -> Option<i64> {
    if b.len() != 3 {
        return None;
    }
    Some(bcd(b[0], 99)? * 3600 + bcd(b[1], 59)? * 60 + bcd(b[2], 59)?)
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
