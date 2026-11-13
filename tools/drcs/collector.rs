//! Opt-in DRCS collection for development tools. No background thread survives DLL unloading.
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Mutex, OnceLock},
};

pub(super) struct Bitmap {
    pub code: u16,
    pub font_id: u8,
    pub mode: u8,
    pub depth: u8,
    pub width: u8,
    pub height: u8,
    pub bits: u8,
    pub data: Vec<u8>,
}

pub(super) fn collect(bitmap: &Bitmap) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let Ok(mut seen) = SEEN.get_or_init(Default::default).try_lock() else {
        return;
    };
    let name = bitmap.name();
    if seen.contains(&name) {
        return;
    }
    let result = std::env::current_exe().and_then(|exe| {
        let parent = exe
            .parent()
            .ok_or_else(|| io::Error::other("No executable directory"))?;
        bitmap.save(&parent.join("drcs-unmapped"))
    });
    if let Err(error) = result {
        crate::windows::debug(&format!("DRCS collection failed: {error}"));
    }
    // Bound the session cache. Existing files also deduplicate across restarts.
    // Failed writes are not retried for each repetition of the same glyph.
    if seen.len() >= 4096 {
        seen.clear();
    }
    seen.insert(name);
}

impl Bitmap {
    fn name(&self) -> String {
        let hash = crate::drcs::bitmap_hash(
            self.width,
            self.height,
            self.bits,
            self.mode,
            self.depth,
            &self.data,
        );
        format!(
            "{}x{}-{}bpp-{hash:016x}",
            self.width, self.height, self.bits
        )
    }
    fn bmp(&self) -> Vec<u8> {
        let w = usize::from(self.width);
        let h = usize::from(self.height);
        let stride = (w * 3).div_ceil(4) * 4;
        let mut out = vec![0; 54 + stride * h];
        out[0..2].copy_from_slice(b"BM");
        let len = out.len() as u32;
        out[2..6].copy_from_slice(&len.to_le_bytes());
        out[10..14].copy_from_slice(&54u32.to_le_bytes());
        out[14..18].copy_from_slice(&40u32.to_le_bytes());
        out[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        out[22..26].copy_from_slice(&(h as i32).to_le_bytes());
        out[26..28].copy_from_slice(&1u16.to_le_bytes());
        out[28..30].copy_from_slice(&24u16.to_le_bytes());
        let max = if self.mode == 0 {
            1
        } else {
            u32::from(self.depth) + 1
        };
        for y in 0..h {
            for x in 0..w {
                let start = (y * w + x) * usize::from(self.bits);
                let mut value = 0u32;
                for bit in start..start + usize::from(self.bits) {
                    value = (value << 1) | u32::from((self.data[bit / 8] >> (7 - bit % 8)) & 1);
                }
                let gray = (255 - value.min(max) * 255 / max) as u8;
                let offset = 54 + (h - 1 - y) * stride + x * 3;
                out[offset..offset + 3].fill(gray);
            }
        }
        out
    }
    fn save(&self, directory: &Path) -> io::Result<()> {
        fs::create_dir_all(directory)?;
        let name = self.name();
        let hex: String = self.data.iter().map(|b| format!("{b:02x}")).collect();
        let metadata = serde_json::json!({
            "format_version": 1, "character_code": format!("0x{:04X}", self.code),
            "font_id": self.font_id, "mode": self.mode, "depth": self.depth,
            "width": self.width, "height": self.height, "bits_per_pixel": self.bits,
            "bitmap_hex": hex,
        });
        write_new(
            &directory.join(format!("{name}.json")),
            &serde_json::to_vec_pretty(&metadata)?,
        )?;
        write_new(&directory.join(format!("{name}.bmp")), &self.bmp())
    }
}
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error),
    };
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/drcs_dump.rs"]
mod tests;
