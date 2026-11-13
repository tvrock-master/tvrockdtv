//! TS payload and section reassembly shared by all PSI/SI tables.
#[derive(Default)]
pub struct Receiver {
    pub data: Vec<u8>,
    last: Option<[u8; 188]>,
}
pub struct Payload<'a> {
    pub pid: u16,
    pub start: bool,
    pub cc: u8,
    pub discontinuity: bool,
    pub bytes: &'a [u8],
}
pub fn payload(p: &[u8; 188]) -> Option<Payload<'_>> {
    if p[0] != 0x47 || p[1] & 0x80 != 0 || p[3] & 0xc0 != 0 {
        return None;
    }
    let afc = (p[3] >> 4) & 3;
    if afc == 0 {
        return None;
    }
    let mut pos = 4;
    let mut discontinuity = false;
    if afc & 2 != 0 {
        let n = p[4] as usize;
        if 5 + n > 188 {
            return None;
        }
        discontinuity = n > 0 && p[5] & 0x80 != 0;
        pos = 5 + n;
    }
    Some(Payload {
        pid: (((p[1] & 31) as u16) << 8) | p[2] as u16,
        start: p[1] & 64 != 0,
        cc: p[3] & 15,
        discontinuity,
        bytes: if afc & 1 != 0 { &p[pos..] } else { &[] },
    })
}
impl Receiver {
    pub fn accept<'a>(&mut self, p: &'a [u8; 188]) -> Option<Payload<'a>> {
        let Some(h) = payload(p) else {
            self.data.clear();
            self.last = None;
            return None;
        };
        if h.discontinuity {
            self.data.clear();
            self.last = None
        }
        if h.bytes.is_empty() {
            return None;
        }
        if let Some(old) = &self.last {
            if old == p {
                return None;
            }
            if ((old[3].wrapping_add(1)) & 15) != h.cc {
                self.data.clear();
            }
        }
        self.last = Some(*p);
        Some(h)
    }
    pub fn sections(&mut self, p: &[u8; 188]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let Some(h) = self.accept(p) else { return out };
        if h.start {
            let Some((&pointer, rest)) = h.bytes.split_first() else {
                return out;
            };
            let pointer = pointer as usize;
            if pointer > rest.len() {
                self.data.clear();
                return out;
            }
            self.append(&rest[..pointer], false, &mut out);
            self.data.clear();
            self.append(&rest[pointer..], true, &mut out);
        } else {
            self.append(h.bytes, false, &mut out)
        }
        out
    }
    fn append(&mut self, mut bytes: &[u8], mut start: bool, out: &mut Vec<Vec<u8>>) {
        while !bytes.is_empty() {
            if self.data.is_empty() && (!start || bytes[0] == 0xff) {
                return;
            }
            let need = if self.data.len() < 3 {
                3
            } else {
                3 + length12(&self.data[1..])
            };
            if need > 4096 {
                self.data.clear();
                return;
            }
            let take = (need - self.data.len()).min(bytes.len());
            self.data.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.data.len() >= 3 && self.data.len() == 3 + length12(&self.data[1..]) {
                out.push(std::mem::take(&mut self.data));
                start = true
            }
        }
    }
}
pub fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
pub fn length12(b: &[u8]) -> usize {
    (be16(b) & 4095) as usize
}
pub fn crc32(b: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &v in b {
        crc = (crc << 8) ^ CRC_TABLE[((crc >> 24) as u8 ^ v) as usize];
    }
    crc
}
const CRC_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            value = (value << 1)
                ^ if value & 0x8000_0000 != 0 {
                    0x04c11db7
                } else {
                    0
                };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
};
pub fn descriptors(mut b: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 2 {
            return None;
        }
        let n = b[1] as usize;
        if n + 2 > b.len() {
            return None;
        }
        out.push((b[0], &b[2..2 + n]));
        b = &b[n + 2..]
    }
    Some(out)
}
#[derive(Default)]
pub struct Counter {
    pub total: u32,
    pub drop: u32,
    pub scramble: u32,
    last: Option<[u8; 188]>,
    dupes: u8,
}
impl Counter {
    pub fn packet(&mut self, p: &[u8; 188]) {
        self.total = self.total.saturating_add(1);
        if p[3] & 0xc0 != 0 {
            self.scramble = self.scramble.saturating_add(1)
        }
        let mut clean = *p;
        clean[3] &= 0x3f;
        let Some(h) = payload(&clean) else {
            self.last = None;
            return;
        };
        if h.discontinuity {
            self.last = None
        }
        if h.bytes.is_empty() {
            if self.last.is_some_and(|old| old[3] & 15 != h.cc) {
                self.drop = self.drop.saturating_add(1);
            }
            return;
        }
        if let Some(old) = self.last {
            if old == *p {
                self.dupes = self.dupes.saturating_add(1);
                if self.dupes > 1 {
                    self.drop = self.drop.saturating_add(1);
                }
                return;
            }
            let expected = (old[3].wrapping_add(1)) & 15;
            if expected != h.cc {
                // Preserve the original API's anomaly count, not an inferred lost-packet count.
                self.drop = self.drop.saturating_add(1)
            }
        }
        self.dupes = 0;
        self.last = Some(*p);
    }
}
