use crate::{
    caption_layout::CaptionDecoder,
    ts::{Receiver, be16},
};
#[derive(Default)]
pub struct Caption {
    pub receiver: Receiver,
    pub services: Vec<u16>,
    pub oneseg: bool,
    decoder: Option<CaptionDecoder>,
}
fn be24(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
}
impl Caption {
    pub fn packet(&mut self, p: &[u8; 188], pid: u16, service: u16) -> Vec<String> {
        let mut out = Vec::new();
        let Some(h) = self.receiver.accept(p) else {
            return out;
        };
        if h.start {
            if !self.receiver.data.is_empty() {
                out.extend(self.pes(pid, service));
            }
            self.receiver.data.clear();
        }
        if !h.start && self.receiver.data.is_empty() {
            return out;
        }
        if self.receiver.data.len() + h.bytes.len() > 65542 {
            self.receiver.data.clear();
            return out;
        }
        self.receiver.data.extend_from_slice(h.bytes);
        if self.receiver.data.len() >= 6 {
            let n = be16(&self.receiver.data[4..]) as usize;
            if n > 0 && self.receiver.data.len() >= n + 6 {
                self.receiver.data.truncate(n + 6);
                out.extend(self.pes(pid, service));
                self.receiver.data.clear();
            }
        }
        out
    }
    fn pes(&mut self, pid: u16, service: u16) -> Vec<String> {
        let mut out = Vec::new();
        if service != 0 && !self.services.contains(&service) {
            return out;
        }
        let s = &self.receiver.data;
        if s.len() < 9 || s[..4] != [0, 0, 1, 0xbd] {
            return out;
        }
        let mut p = 9 + s[8] as usize;
        if p > s.len() {
            return out;
        }
        let declared = be16(&s[4..]) as usize;
        if declared != 0 && declared + 6 > s.len() {
            return out;
        }
        let pts = if s[7] & 0x80 != 0 && s[8] >= 5 {
            (((s[9] & 14) as i64) << 29)
                | ((s[10] as i64) << 22)
                | (((s[11] & 254) as i64) << 14)
                | ((s[12] as i64) << 7)
                | ((s[13] >> 1) as i64)
        } else {
            -1
        };
        if p + 3 <= s.len() && matches!(s[p], 0x80 | 0x81) && s[p + 1] == 255 {
            p += 3 + (s[p + 2] & 15) as usize
        }
        while p + 5 <= s.len() {
            let end = p + 5 + be16(&s[p + 3..]) as usize;
            if end + 2 > s.len() {
                break;
            }
            let group = (s[p] >> 2) & 63;
            let lang = match group {
                1..=8 => group,
                0x21..=0x28 => group - 0x20,
                _ => {
                    p = end + 2;
                    continue;
                }
            };
            // The UI displays the primary language. Empty secondary-language groups
            // must not clear the primary captions. Fragmented groups are not decoded partially.
            if lang != 1 || s[p + 1] != 0 || s[p + 2] != 0 {
                p = end + 2;
                continue;
            }
            let mut units = Vec::new();
            let mut d = p + 5;
            if d < end {
                d += if matches!(s[d] >> 6, 1 | 2) { 5 } else { 1 };
                if d + 3 <= end {
                    let unit_end = d + 3 + be24(&s[d..]);
                    d += 3;
                    if unit_end <= end {
                        while d + 5 <= unit_end {
                            let n = be24(&s[d + 2..]);
                            if d + 5 + n > unit_end {
                                break;
                            }
                            if s[d] == 0x1f && matches!(s[d + 1], 0x20 | 0x30 | 0x31) {
                                units.push((s[d + 1], &s[d + 5..d + 5 + n]));
                            }
                            d += 5 + n;
                        }
                    }
                }
            }
            let decoder = self
                .decoder
                .get_or_insert_with(|| CaptionDecoder::new(self.oneseg));
            out.extend(
                decoder
                    .statement_units(&units, pid, pts, lang)
                    .into_iter()
                    .filter_map(|display| {
                        serde_json::to_string(&display)
                            .ok()
                            .map(|json| format!("CAP:{json}"))
                    }),
            );
            p = end + 2;
        }
        out
    }
}
