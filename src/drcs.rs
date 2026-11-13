//! Stable 64-bit hashes of verified DRCS glyphs; no reference bitmaps are embedded.
use std::collections::BTreeMap;

// Sorted FNV-1a hashes of [width, height, bits, mode, depth] followed by bitmap bytes.
const MAPPINGS: &[(u64, &str)] = include!("drcs/mappings.rs");

/// Shared with the collector so capture IDs and decoder keys stay identical.
pub(crate) fn bitmap_hash(
    width: u8,
    height: u8,
    bits: u8,
    mode: u8,
    depth: u8,
    bitmap: &[u8],
) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in [width, height, bits, mode, depth].iter().chain(bitmap) {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    hash
}

fn lookup(hash: u64) -> Option<&'static str> {
    MAPPINGS
        .binary_search_by_key(&hash, |&(key, _)| key)
        .ok()
        .map(|index| MAPPINGS[index].1)
}

#[derive(Default)]
pub(crate) struct Map(BTreeMap<u16, &'static str>);
impl Map {
    pub fn get(&self, code: u16) -> Option<&'static str> {
        self.0.get(&code).copied()
    }
    pub fn define(&mut self, data: &[u8]) {
        if let Some(entries) = parse(data) {
            self.install(entries);
        }
    }
    fn install(&mut self, entries: Vec<Entry>) {
        for entry in entries {
            let (code, text) = (entry.code, entry.text);
            #[cfg(feature = "drcs-collect")]
            for bitmap in entry.unmapped {
                super::drcs_dump::collect(&bitmap);
            }
            // Codes are reused; an unknown replacement must invalidate the old mapping.
            self.0.remove(&code);
            if let Some(text) = text {
                self.0.insert(code, text);
            }
        }
    }
}
fn take<'a>(data: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let result = data.get(..n)?;
    *data = &data[n..];
    Some(result)
}
struct Entry {
    code: u16,
    text: Option<&'static str>,
    #[cfg(feature = "drcs-collect")]
    unmapped: Vec<super::drcs_dump::Bitmap>,
}
fn parse(data: &[u8]) -> Option<Vec<Entry>> {
    parse_with_lookup(data, lookup)
}
fn parse_with_lookup(
    mut data: &[u8],
    lookup: impl Fn(u64) -> Option<&'static str>,
) -> Option<Vec<Entry>> {
    let count = take(&mut data, 1)?[0];
    let mut entries = Vec::new();
    for _ in 0..count {
        let header = take(&mut data, 3)?;
        let code = u16::from_be_bytes([header[0], header[1]]);
        let mut text = None;
        #[cfg(feature = "drcs-collect")]
        let mut unmapped = Vec::new();
        for font in 0..header[2] {
            let font_header = take(&mut data, 1)?[0];
            let mode = font_header & 15;
            if mode > 1 {
                let header = take(&mut data, 4)?;
                take(
                    &mut data,
                    u16::from_be_bytes([header[2], header[3]]) as usize,
                )?;
                continue;
            }
            let header = take(&mut data, 3)?;
            let (depth, width, height) = (header[0], header[1] as usize, header[2] as usize);
            if width == 0 || height == 0 {
                return None;
            }
            let bits = if mode == 0 {
                1
            } else {
                (u16::from(depth) + 1).ilog2() as usize + 1
            };
            let bitmap = take(&mut data, (width * height * bits).div_ceil(8))?;
            let mapped = lookup(bitmap_hash(
                width as u8,
                height as u8,
                bits as u8,
                mode,
                depth,
                bitmap,
            ));
            if font == 0 {
                text = mapped;
            }
            #[cfg(feature = "drcs-collect")]
            if mapped.is_none() {
                unmapped.push(super::drcs_dump::Bitmap {
                    code,
                    font_id: font_header >> 4,
                    mode,
                    depth,
                    width: width as u8,
                    height: height as u8,
                    bits: bits as u8,
                    data: bitmap.to_vec(),
                });
            }
        }
        entries.push(Entry {
            code,
            text,
            #[cfg(feature = "drcs-collect")]
            unmapped,
        });
    }
    Some(entries)
}

#[cfg(test)]
#[path = "../tests/unit/drcs.rs"]
mod tests;
