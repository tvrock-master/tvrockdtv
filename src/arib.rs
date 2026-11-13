//! Bounded ARIB STD-B24 text decoder. Rendering/DRCS bitmap composition is outside this API.
use crate::tables::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub pos: usize,
    pub size: u8,
    pub color: u8,
    pub background: u8,
    pub raster: u8,
}
#[derive(Clone, Debug)]
pub(crate) enum CaptionToken {
    Text(String, Format),
    Control(u8, Vec<u8>),
}

pub struct Decoded {
    pub text: String,
    pub formats: Vec<Format>,
}
pub fn decode(data: &[u8], flags: u32, capacity: usize) -> Decoded {
    decode_with_drcs(data, flags, capacity, &|_| None)
}
pub fn decode_with_drcs(
    data: &[u8],
    flags: u32,
    capacity: usize,
    drcs: &dyn Fn(u16) -> Option<String>,
) -> Decoded {
    let mut d = Decoder::new(flags, capacity, drcs);
    d.process(data);
    Decoded {
        text: d.out,
        formats: d.formats,
    }
}
impl<'a> Decoder<'a> {
    fn new(flags: u32, capacity: usize, drcs: &'a dyn Fn(u16) -> Option<String>) -> Self {
        let caption = flags & 1 != 0;
        let oneseg = caption && flags & 2 != 0;
        Self {
            g: [
                1,
                if oneseg { 17 } else { 2 },
                3,
                if caption { 32 } else { 4 },
            ],
            gl: if oneseg { 1 } else { 0 },
            gr: if oneseg { 0 } else { 2 },
            single: None,
            esc: 0,
            index: 0,
            drcs_escape: false,
            flags,
            repeat: 1,
            palette: 0,
            format: Format {
                pos: 0,
                size: 2,
                color: if caption { 7 } else { 0 },
                background: if caption { 8 } else { 0 },
                raster: if caption { 8 } else { 0 },
            },
            formats: Vec::new(),
            out: String::new(),
            units: 0,
            limit: capacity.saturating_sub(1),
            drcs,
            drcs_map: Default::default(),
            tokens: Vec::new(),
        }
    }
}
pub(crate) struct CaptionText {
    decoder: Decoder<'static>,
}
impl CaptionText {
    pub fn new(oneseg: bool) -> Self {
        Self {
            decoder: Decoder::new(1 | 4 | 8 | 16 | if oneseg { 2 } else { 0 }, 65536, &|_| {
                None
            }),
        }
    }
    pub fn define_drcs(&mut self, data: &[u8]) {
        self.decoder.drcs_map.define(data);
    }
    pub fn decode(&mut self, data: &[u8]) -> Vec<CaptionToken> {
        self.decoder.out.clear();
        self.decoder.formats.clear();
        self.decoder.units = 0;
        self.decoder.process(data);
        std::mem::take(&mut self.decoder.tokens)
    }
}
struct Decoder<'a> {
    tokens: Vec<CaptionToken>,
    g: [u8; 4],
    gl: usize,
    gr: usize,
    single: Option<usize>,
    esc: u8,
    index: usize,
    drcs_escape: bool,
    flags: u32,
    repeat: u8,
    palette: u8,
    format: Format,
    formats: Vec<Format>,
    out: String,
    units: usize,
    limit: usize,
    drcs: &'a dyn Fn(u16) -> Option<String>,
    drcs_map: crate::drcs::Map,
}
impl Decoder<'_> {
    fn emit(&mut self, s: &str, repeat: bool) -> bool {
        let n = s.encode_utf16().count();
        if n == 0 {
            return true;
        }
        if n > self.limit - self.units {
            return false;
        }
        let count = if repeat {
            usize::from(self.repeat.max(1)).min((self.limit - self.units) / n)
        } else {
            1
        };
        for _ in 0..count {
            self.out.push_str(s);
            if self.flags & 16 != 0 && s != "\r\n" && s != "\r" && s != "\u{c}" {
                self.tokens.push(CaptionToken::Text(s.into(), self.format));
            }
        }
        self.units += count * n;
        if repeat {
            self.repeat = 1
        }
        true
    }
    fn format(&mut self) {
        self.format.pos = self.units;
        if self.formats.last().is_some_and(|f| f.pos == self.units) {
            self.formats.pop();
        }
        self.formats.push(self.format);
    }
    fn designate(&mut self, b: u8, drcs: bool) -> bool {
        let code = if drcs {
            match b {
                0x40..=0x4f => 16 + b - 0x40,
                0x70 => 32,
                _ => return false,
            }
        } else {
            match b {
                0x42 => 1,
                0x4a => 2,
                0x30 => 3,
                0x31 => 4,
                0x32..=0x38 => 5 + b - 0x32,
                0x49 => 12,
                0x39 => 13,
                0x3a => 14,
                0x3b => 15,
                _ => return false,
            }
        };
        self.g[self.index] = code;
        true
    }
    fn escape(&mut self, b: u8) {
        match self.esc {
            1 => match b {
                0x6e | 0x6f => {
                    self.gl = (b - 0x6c) as usize;
                    self.esc = 0;
                    return;
                }
                0x7c..=0x7e => {
                    self.gr = (0x7f - b) as usize;
                    self.esc = 0;
                    return;
                }
                0x24 | 0x28 => self.index = 0,
                0x29..=0x2b => self.index = (b - 0x28) as usize,
                _ => {
                    self.esc = 0;
                    return;
                }
            },
            2 => {
                if self.designate(b, false) {
                    self.esc = 0;
                    return;
                }
                match b {
                    0x20 => self.drcs_escape = true,
                    0x28..=0x2b => {
                        self.index = (b - 0x28) as usize;
                        self.drcs_escape = false
                    }
                    _ => {
                        self.esc = 0;
                        return;
                    }
                }
            }
            3 => {
                if self.designate(b, self.drcs_escape) {
                    self.esc = 0;
                    return;
                }
                if b == 0x20 {
                    self.drcs_escape = true
                } else {
                    self.esc = 0;
                    return;
                }
            }
            _ => {
                self.designate(b, true);
                self.esc = 0;
                return;
            }
        }
        self.esc += 1;
    }
    fn glyph(&self, code: u16, set: u8) -> String {
        match set {
            1 | 13 | 14 if code < 0x7521 => kanji(code),
            1 | 13..=15 => symbol(code, self.flags & 8 != 0).to_owned(),
            2 | 9 => char::from_u32(if self.flags & 4 != 0 && self.format.size == 1 {
                match code {
                    0x5c => 0xa5,
                    0x7e => 0x203e,
                    _ => code as u32,
                }
            } else {
                code as u32 + 0xfee0
            })
            .unwrap_or('□')
            .to_string(),
            3 | 10 => ACHIRAGANATABLE
                .chars()
                .nth(code as usize)
                .unwrap_or('□')
                .to_string(),
            4 | 11 => ACKATAKANATABLE
                .chars()
                .nth(code as usize)
                .unwrap_or('□')
                .to_string(),
            12 => ACJISKATAKANATABLE
                .chars()
                .nth(code as usize)
                .unwrap_or('□')
                .to_string(),
            16..=31 => {
                let code = if set == 16 {
                    code
                } else {
                    ((set as u16 - 16 + 0x40) << 8) | code
                };
                self.drcs_map
                    .get(code)
                    .map(str::to_owned)
                    .or_else(|| (self.drcs)(code))
                    .unwrap_or_else(|| "□".into())
            }
            _ => "□".into(),
        }
    }
    fn process(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() && self.units < self.limit {
            let token_start = i;
            let b = data[i];
            i += 1;
            if self.esc != 0 {
                self.escape(b);
                continue;
            }
            if (0x21..=0x7e).contains(&b) || (0xa1..=0xfe).contains(&b) {
                let right = b >= 0xa1;
                let set = self.g[if right {
                    self.gr
                } else {
                    self.single.take().unwrap_or(self.gl)
                }];
                let mut code = (b & 0x7f) as u16;
                if matches!(set, 1 | 13..=16) {
                    let Some(&next) = data.get(i) else { break };
                    if !(if right { 0xa1..=0xfe } else { 0x21..=0x7e }).contains(&next) {
                        continue;
                    }
                    code = (code << 8) | (next & 0x7f) as u16;
                    i += 1;
                }
                if set == 32 {
                    if (0x60..=0x6f).contains(&code) {
                        self.process(MACROS[(code & 15) as usize])
                    }
                    continue;
                }
                if !self.emit(&self.glyph(code, set), true) {
                    break;
                }
                continue;
            }
            match b {
                0x0d => {
                    if !self.emit("\r\n", false) {
                        self.emit("\r", false);
                    }
                }
                0x0c => {
                    if self.flags & 16 != 0 {
                        let fresh = Self::new(self.flags, self.limit + 1, self.drcs);
                        self.g = fresh.g;
                        self.gl = fresh.gl;
                        self.gr = fresh.gr;
                        self.single = None;
                        self.palette = 0;
                        self.repeat = 1;
                        self.format = fresh.format;
                    }
                    self.emit("\u{c}", false);
                }
                0x0f => self.gl = 0,
                0x0e => self.gl = 1,
                0x19 => self.single = Some(2),
                0x1d => self.single = Some(3),
                0x1b => self.esc = 1,
                0x20 | 0xa0 => {
                    self.emit(
                        if b == 0xa0
                            || (self.flags & 4 != 0 && matches!(self.format.size, 0 | 1 | 3))
                        {
                            " "
                        } else {
                            "　"
                        },
                        true,
                    );
                }
                0x80..=0x87 => {
                    self.format.color = (self.palette << 4) | (b & 15);
                    self.format();
                }
                0x88..=0x8a => {
                    self.format.size = b - 0x88;
                    self.format();
                }
                0x8b => {
                    let Some(&v) = data.get(i) else { break };
                    i += 1;
                    self.format.size = match v {
                        0x60 => 3,
                        0x41 => 4,
                        0x44 => 5,
                        0x45 => 6,
                        0x6b => 7,
                        0x64 => 8,
                        _ => self.format.size,
                    };
                    self.format();
                }
                0x16 | 0x91 | 0x93 | 0x94 | 0x97 => {
                    if i >= data.len() {
                        break;
                    }
                    i += 1
                }
                0x1c => {
                    if i + 2 > data.len() {
                        break;
                    }
                    i += 2
                }
                0x90 => {
                    let Some(&v) = data.get(i) else { break };
                    i += 1;
                    if v == 0x20 {
                        let Some(&v) = data.get(i) else { break };
                        i += 1;
                        self.palette = v & 7
                    } else {
                        match v & 0xf0 {
                            0x40 => self.format.color = (self.palette << 4) | (v & 15),
                            0x50 => self.format.background = (self.palette << 4) | (v & 15),
                            _ => (),
                        }
                        self.format();
                    }
                }
                0x92 => {
                    let Some(&v) = data.get(i) else { break };
                    i += 1;
                    if v == 0x20 {
                        if i >= data.len() {
                            break;
                        }
                        i += 1
                    }
                }
                0x95 => {
                    let Some(&v) = data.get(i) else { break };
                    i += 1;
                    if v != 0x4f {
                        while i + 1 < data.len() && data[i..i + 2] != [0x95, 0x4f] {
                            i += 1
                        }
                        if i + 1 >= data.len() {
                            break;
                        }
                        i += 2
                    }
                }
                0x98 => {
                    let Some(&v) = data.get(i) else { break };
                    i += 1;
                    self.repeat = v & 63
                }
                0x9b if self.flags & 16 != 0 => {
                    while i < data.len() && !(0x40..=0x7e).contains(&data[i]) {
                        i += 1;
                    }
                    if i >= data.len() {
                        break;
                    }
                    i += 1;
                }
                0x9b => {
                    let start = i;
                    while i < data.len() && data[i] <= 0x3b {
                        i += 1
                    }
                    let Some(&v) = data.get(i) else { break };
                    if v == 0x69 {
                        if i - start != 2 {
                            break;
                        }
                        if data[start] >= 0x32 {
                            i += 1;
                            while i < data.len() && data[i] != 0x9b {
                                i += 1
                            }
                            if i + 4 > data.len() {
                                break;
                            }
                            i += 3;
                        }
                    }
                    i += 1;
                }
                0x9d => {
                    let Some(&v) = data.get(i) else { break };
                    if v == 0x20 {
                        if i + 2 > data.len() {
                            break;
                        }
                        i += 2
                    } else {
                        while i < data.len() && !(0x40..=0x43).contains(&data[i]) {
                            i += 1
                        }
                        if i == data.len() {
                            break;
                        }
                        i += 1
                    }
                }
                _ => (),
            }
            if self.flags & 16 != 0 {
                self.tokens
                    .push(CaptionToken::Control(b, data[token_start + 1..i].to_vec()));
            }
        }
    }
}
fn symbol(code: u16, unicode: bool) -> &'static str {
    let tables: &[(u16, &[&str], &[&str])] = &[
        (0x7521, KANJITABLE1, KANJITABLE1),
        (0x7621, KANJITABLE2, KANJITABLE2),
        (0x7a21, SYMBOLSTABLE_90_01, SYMBOLSTABLE_90_01),
        (0x7a4d, SYMBOLSTABLE_90_45, SYMBOLSTABLE_90_45_U),
        (0x7b21, SYMBOLSTABLE_91, SYMBOLSTABLE_91),
        (0x7c21, SYMBOLSTABLE_92, SYMBOLSTABLE_92_U),
        (0x7d21, SYMBOLSTABLE_93, SYMBOLSTABLE_93_U),
        (0x7e21, SYMBOLSTABLE_94, SYMBOLSTABLE_94_U),
    ];
    for &(start, normal, uni) in tables {
        if code >= start
            && let Some(s) = (if unicode { uni } else { normal }).get((code - start) as usize)
        {
            return s;
        }
    }
    "□"
}
fn kanji(code: u16) -> String {
    let row = (code >> 8) as u8 - 0x21;
    let mut second = (code & 255) as u8;
    if row & 1 == 0 {
        second += 0x1f;
        if second >= 0x7f {
            second += 1
        }
    } else {
        second += 0x7e
    }
    let first = (row >> 1) + if row >> 1 >= 0x1f { 0xc1 } else { 0x81 };
    crate::windows::decode_cp932(&[first, second])
}
