//! Stateful B24 horizontal text placement, separate from plain EPG string decoding.
use crate::arib::{CaptionText, CaptionToken, Format};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Layout {
    pub x: f64,
    pub y: f64,
    pub cell_width: f64,
    pub cell_height: f64,
    pub glyph_width: f64,
    pub glyph_height: f64,
    pub cells: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Display {
    pid: u16,
    pts: i64,
    lang: u8,
    pub clear: bool,
    x: i32,
    y: i32,
    color: u8,
    color_name: &'static str,
    bg_color: u8,
    size: &'static str,
    pub text: String,
    pub delay_ms: u32,
    timing: u8,
    pub layout: Option<Layout>,
}
pub struct CaptionDecoder {
    text: CaptionText,
    position: Position,
}
struct Position {
    oneseg: bool,
    mode: u32,
    origin: (f64, f64),
    area: (f64, f64),
    cursor: Option<(f64, f64)>,
    font: (f64, f64),
    spacing: (f64, f64),
    size: u8,
}
impl Position {
    fn new(oneseg: bool, mode: u32) -> Self {
        let (area, font, spacing) = if oneseg {
            ((320., 180.), (18., 18.), (2., 6.))
        } else {
            match mode {
                5 => ((1920., 1080.), (36., 36.), (4., 24.)),
                9 => ((720., 480.), (36., 36.), (4., 16.)),
                _ => ((960., 540.), (36., 36.), (4., 24.)),
            }
        };
        Self {
            oneseg,
            mode,
            origin: (0., 0.),
            area,
            cursor: None,
            font,
            spacing,
            size: 2,
        }
    }
    fn dimensions(&self) -> (f64, f64, f64, f64) {
        let (sx, sy) = match self.size {
            0 => (0.5, 0.5),
            1 => (0.5, 1.),
            3 => (0.25, 0.25),
            4 => (1., 2.),
            5 => (2., 1.),
            6 => (2., 2.),
            _ => (1., 1.),
        };
        (
            (self.font.0 + self.spacing.0) * sx,
            (self.font.1 + self.spacing.1) * sy,
            self.font.0 * sx,
            self.font.1 * sy,
        )
    }
    fn cursor(&mut self) -> (f64, f64) {
        let (_, height, _, _) = self.dimensions();
        *self
            .cursor
            .get_or_insert((self.origin.0, self.origin.1 + height))
    }
    fn advance(&mut self, columns: i32, rows: i32) {
        let (cw, ch, _, _) = self.dimensions();
        let (mut x, mut y) = self.cursor();
        x += f64::from(columns) * cw;
        y += f64::from(rows) * ch;
        if x >= self.origin.0 + self.area.0 {
            x = self.origin.0;
            y += ch;
        }
        if x < self.origin.0 {
            x = (self.origin.0 + self.area.0 - cw).max(self.origin.0);
            y -= ch;
        }
        if y > self.origin.1 + self.area.1 {
            y = self.origin.1 + ch;
        }
        if y <= self.origin.1 {
            y = self.origin.1 + self.area.1;
        }
        self.cursor = Some((x, y));
    }
    fn csi(&mut self, bytes: &[u8]) {
        let Some((&command, params)) = bytes.split_last() else {
            return;
        };
        let Ok(params) = std::str::from_utf8(params) else {
            return;
        };
        let values: Vec<u32> = params
            .trim()
            .split(';')
            .filter_map(|p| p.parse().ok())
            .collect();
        let Some(&a) = values.first() else { return };
        let b = values.get(1).copied();
        // Coordinates and dimensions are bounded before any renderer sees them.
        if a > 4096 || b.is_some_and(|v| v > 4096) {
            return;
        }
        match (command, b) {
            (b'S', _) => {
                let defaults = Self::new(self.oneseg, a);
                self.mode = a;
                self.font = defaults.font;
                self.spacing = defaults.spacing;
                self.area = defaults.area;
            }
            (b'V', Some(b)) if a > 0 && b > 0 => self.area = (a.into(), b.into()),
            (b'W', Some(b)) if a > 0 && b > 0 => self.font = (a.into(), b.into()),
            (b'X', _) => self.spacing.0 = a.into(),
            (b'Y', _) => self.spacing.1 = a.into(),
            (b'_', b) => {
                self.origin.0 = a.into();
                if let Some(b) = b {
                    self.origin.1 = b.into();
                }
            }
            (b'a', Some(b)) => self.cursor = Some((a.into(), b.into())),
            _ => (),
        }
    }
}
impl CaptionDecoder {
    pub fn new(oneseg: bool) -> Self {
        Self {
            text: CaptionText::new(oneseg),
            position: Position::new(oneseg, 7),
        }
    }
    pub fn statement(&mut self, units: &[&[u8]], pid: u16, pts: i64, lang: u8) -> Vec<Display> {
        let units: Vec<_> = units.iter().map(|unit| (0x20, *unit)).collect();
        self.statement_units(&units, pid, pts, lang)
    }
    pub fn statement_units(
        &mut self,
        units: &[(u8, &[u8])],
        pid: u16,
        pts: i64,
        lang: u8,
    ) -> Vec<Display> {
        let mut out: Vec<Display> = Vec::new();
        let mut delay = 0u32;
        for &(kind, unit) in units {
            if matches!(kind, 0x30 | 0x31) {
                self.text.define_drcs(unit);
                continue;
            }
            if kind != 0x20 {
                continue;
            }
            for token in self.text.decode(unit) {
                match token {
                    CaptionToken::Text(text, format) => {
                        self.position.size = format.size;
                        let (cw, ch, gw, gh) = self.position.dimensions();
                        let (x, y) = self.position.cursor();
                        let layout = Layout {
                            x,
                            y: y - ch,
                            cell_width: cw,
                            cell_height: ch,
                            glyph_width: gw,
                            glyph_height: gh,
                            cells: 1,
                        };
                        let merged = out.last_mut().is_some_and(|previous| {
                            if previous.clear
                                || previous.delay_ms != delay
                                || previous.color != format.color
                                || previous.bg_color != format.background
                                || previous.size != size_name(format.size)
                            {
                                return false;
                            }
                            let Some(old) = &mut previous.layout else {
                                return false;
                            };
                            if old.y != layout.y
                                || old.cell_width != cw
                                || old.cell_height != ch
                                || old.glyph_width != gw
                                || old.glyph_height != gh
                                || old.x + old.cells as f64 * cw != x
                            {
                                return false;
                            }
                            old.cells += 1;
                            previous.text.push_str(&text);
                            true
                        });
                        if !merged {
                            out.push(Display::new(
                                pid,
                                pts,
                                lang,
                                delay,
                                false,
                                text,
                                format,
                                Some(layout),
                            ));
                        }
                        self.position.advance(1, 0);
                    }
                    CaptionToken::Control(code, params) => match code {
                        0x0c => {
                            out.push(Display::new(
                                pid,
                                pts,
                                lang,
                                delay,
                                true,
                                String::new(),
                                Format {
                                    pos: 0,
                                    size: 2,
                                    color: 7,
                                    background: 8,
                                    raster: 8,
                                },
                                None,
                            ));
                            self.position = Position::new(self.position.oneseg, self.position.mode);
                        }
                        0x08 => self.position.advance(-1, 0),
                        0x09 => self.position.advance(1, 0),
                        0x0a => self.position.advance(0, 1),
                        0x0b => self.position.advance(0, -1),
                        0x0d => {
                            self.position.advance(0, 1);
                            self.position.cursor.as_mut().unwrap().0 = self.position.origin.0;
                        }
                        0x16 if !params.is_empty() => {
                            self.position.advance(i32::from(params[0] & 63), 0)
                        }
                        0x1c if params.len() == 2 => {
                            let (cw, ch, _, _) = self.position.dimensions();
                            self.position.cursor = Some((
                                self.position.origin.0 + f64::from(params[1] & 63) * cw,
                                self.position.origin.1 + f64::from((params[0] & 63) + 1) * ch,
                            ));
                        }
                        0x88..=0x8a => self.position.size = code - 0x88,
                        0x8b if !params.is_empty() => {
                            self.position.size = match params[0] {
                                0x60 => 3,
                                0x41 => 4,
                                0x44 => 5,
                                0x45 => 6,
                                _ => self.position.size,
                            }
                        }
                        0x9b => self.position.csi(&params),
                        0x9d if params.len() == 2 && params[0] == 0x20 => {
                            delay = delay.saturating_add(u32::from(params[1] & 63) * 100);
                        }
                        _ => (),
                    },
                }
            }
        }
        out
    }
}
fn size_name(size: u8) -> &'static str {
    [
        "small",
        "medium",
        "normal",
        "micro",
        "high_w",
        "width_w",
        "w",
        "special_1",
        "special_2",
    ]
    .get(size as usize)
    .copied()
    .unwrap_or("normal")
}
impl Display {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pid: u16,
        pts: i64,
        lang: u8,
        delay_ms: u32,
        clear: bool,
        text: String,
        format: Format,
        layout: Option<Layout>,
    ) -> Self {
        Self {
            pid,
            pts,
            lang,
            delay_ms,
            clear,
            text,
            timing: 1,
            x: layout.as_ref().map_or(-1, |p| (p.x / p.cell_width) as i32),
            y: layout.as_ref().map_or(-1, |p| (p.y / p.cell_height) as i32),
            color: format.color,
            bg_color: format.background,
            size: size_name(format.size),
            color_name: [
                "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
            ]
            .get(format.color as usize)
            .copied()
            .unwrap_or("unknown"),
            layout,
        }
    }
}
