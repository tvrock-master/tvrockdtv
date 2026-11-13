use crate::{
    arib,
    json::quote,
    time,
    ts::{be16, descriptors, length12},
};
use std::collections::{BTreeMap, HashMap, VecDeque};
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Event {
    pub sid: u16,
    pub eid: u16,
    pub onid: u16,
    pub tsid: u16,
    pub start: i64,
    pub end: i64,
    pub title: String,
    pub subtitle: String,
    pub content: String,
    pub nibble: (u8, u8),
    pub series: Vec<u8>,
    pub groups: Vec<u8>,
    pub common: Option<(u16, u16)>,
    pub version: u8,
    pub table: u8,
    pub short: bool,
    pub genre: bool,
    extended: BTreeMap<u8, Vec<u8>>,
}
impl Event {
    pub fn json(&self, full: bool) -> String {
        let base = format!(
            "\"Sid\":{},\"Eid\":{},\"Start\":{},\"End\":{},\"Title\":{},\"Subtitle\":{}",
            self.sid,
            self.eid,
            self.start,
            self.end,
            quote(&self.title),
            quote(&self.subtitle)
        );
        let common = match self.common {
            Some((sid, eid)) => format!(",\"CommonSid\":{sid},\"CommonEid\":{eid}"),
            None => ",\"CommonSid\":null,\"CommonEid\":null".to_owned(),
        };
        let base = format!("{base}{common}");
        if full {
            // Content descriptors may be absent; do not classify missing genres as news.
            let nibble = if self.genre { self.nibble } else { (15, 15) };
            format!(
                "{{{base},\"Content\":{},\"Cnib_a\":{},\"Cnib_b\":{}}}",
                quote(&self.content),
                nibble.0,
                nibble.1
            )
        } else {
            format!("{{{base}}}")
        }
    }
}
#[derive(Default)]
pub struct Events {
    pub entries: HashMap<(u16, u16, u16, u16), Event>,
    dirty: VecDeque<(u16, u16, u16, u16)>,
    pub pf: HashMap<u16, [Option<Event>; 2]>,
    last_pf: String,
    relays: VecDeque<String>,
    last_epg_check: Option<std::time::Instant>,
    last_epg_batch: Option<std::time::Instant>,
}
impl Events {
    pub fn section(&mut self, s: &[u8], now: i64) -> Vec<String> {
        let mut messages = Vec::new();
        if s.len() < 18 || !(0x4e..=0x6f).contains(&s[0]) {
            return messages;
        }
        let end = s.len() - 4;
        let mut p = 14;
        let mut parsed = Vec::new();
        while p < end {
            if p + 12 > end {
                return messages;
            }
            let next = p + 12 + length12(&s[p + 10..]);
            if next > end {
                return messages;
            }
            let Some(ds) = descriptors(&s[p + 12..next]) else {
                return messages;
            };
            parsed.push((p, ds));
            p = next;
        }
        for (p, ds) in parsed {
            let key = (be16(&s[10..]), be16(&s[8..]), be16(&s[3..]), be16(&s[p..]));
            let start = time::timestamp(&s[p + 2..p + 7]).unwrap_or(-1);
            let duration = time::duration(&s[p + 7..p + 10]).unwrap_or(-1);
            if start != -1 && (start < now - 172800 || start > now + 8 * 86400)
                || duration == 0
                || duration >= 172800
            {
                continue;
            }
            let version = (s[5] >> 1) & 31;
            let mut e = self.entries.get(&key).cloned().unwrap_or_default();
            // A new version of the same table must not combine old/new extended fragments.
            // Other table IDs (e.g. p/f and schedule) may legitimately supply complementary data.
            if e.table == s[0] && e.version != version {
                e.extended.clear();
            }
            e.sid = key.2;
            e.eid = key.3;
            e.onid = key.0;
            e.tsid = key.1;
            e.start = start;
            e.end = if start == -1 || duration == -1 {
                -1
            } else {
                start + duration
            };
            e.version = version;
            e.table = s[0];
            // Basic EIT and p/f carry event-sharing references. Extended-only tables
            // must not clear a reference received from a complementary basic table.
            let basic = s[0] <= 0x4f || s[0] & 0x08 == 0;
            if basic {
                e.common = None;
            }
            let mut short_received = false;
            for (tag, d) in ds {
                match tag {
                    0x4d if d.len() >= 5 => {
                        let n = d[3] as usize;
                        if n + 5 <= d.len() && n + 5 + d[n + 4] as usize <= d.len() {
                            e.title = arib::decode(&d[4..4 + n], 4, 128).text;
                            e.subtitle =
                                arib::decode(&d[5 + n..5 + n + d[n + 4] as usize], 4, 192).text;
                            e.short = true;
                            short_received = true;
                        }
                    }
                    0x54 if d.len() >= 2 && d.len() % 2 == 0 => {
                        e.nibble = (d[0] >> 4, d[0] & 15);
                        e.genre = true;
                    }
                    0x4e if valid_extended(d) => {
                        e.extended.insert(d[0] >> 4, d.to_vec());
                    }
                    0xd5 if d.len() >= 8 => e.series = d.to_vec(),
                    0xd6 if !d.is_empty() => {
                        e.groups = d.to_vec();
                        let group = d[0] >> 4;
                        let count = (d[0] & 15) as usize;
                        if group == 1 && count == 1 && d.len() >= 5 {
                            let sid = be16(&d[1..]);
                            if sid != e.sid {
                                e.common = Some((sid, be16(&d[3..])));
                            }
                        }
                        let mut targets = Vec::new();
                        if group == 2 && d.len() > count * 4 {
                            for b in d[1..1 + count * 4].as_chunks::<4>().0 {
                                targets.push((key.0, key.1, be16(b), be16(&b[2..])))
                            }
                        }
                        // Group type 4 has event_count local entries before the other-network loop.
                        if group == 4 && d.len() > count * 4 {
                            for b in d[1 + count * 4..].as_chunks::<8>().0 {
                                targets.push((be16(b), be16(&b[2..]), be16(&b[4..]), be16(&b[6..])))
                            }
                        }
                        let mut fresh = Vec::new();
                        for target in targets {
                            let msg = format!(
                                "{{\"Onid\":{},\"Tsid\":{},\"Sid\":{},\"Eid\":{},\"GroupType\":{},\"RelayOnid\":{},\"RelayTsid\":{},\"RelaySid\":{},\"RelayEid\":{}}}",
                                key.0,
                                key.1,
                                key.2,
                                key.3,
                                group,
                                target.0,
                                target.1,
                                target.2,
                                target.3
                            );
                            if !self.relays.contains(&msg) {
                                if self.relays.len() == 256 {
                                    self.relays.pop_front();
                                }
                                self.relays.push_back(msg.clone());
                                fresh.push(msg)
                            }
                        }
                        if !fresh.is_empty() {
                            messages.push(format!("EVR:[{}]", fresh.join(",")))
                        }
                    }
                    _ => (),
                }
            }
            if basic && e.common.is_some() && !short_received {
                e.title.clear();
                e.subtitle.clear();
                e.short = false;
            }
            e.content = extended_text(&e.extended);
            if s[0] == 0x4e
                && s[6] < 2
                && (e.short || e.common.is_some())
                && (self.pf.contains_key(&e.sid) || self.pf.len() < 256)
            {
                self.pf.entry(e.sid).or_default()[s[6] as usize] = Some(e.clone());
            }
            if self.entries.get(&key) != Some(&e) {
                if self.entries.len() >= 16000 && !self.entries.contains_key(&key) {
                    self.entries
                        .retain(|_, e| e.end < 0 || e.end >= now - 172800);
                    if self.entries.len() >= 16000 {
                        continue;
                    }
                }
                if !self.dirty.contains(&key) {
                    self.dirty.push_back(key)
                }
                self.entries.insert(key, e);
            }
        }
        messages
    }
    pub fn current(&mut self, sid: u16) -> Option<String> {
        let p = self.pf.get(&sid)?;
        let empty = Event::default();
        let s = format!(
            "CPF:[{},{}]",
            p[0].as_ref().unwrap_or(&empty).json(false),
            p[1].as_ref().unwrap_or(&empty).json(false)
        );
        if self.last_pf == s {
            return None;
        }
        self.last_pf = s.clone();
        Some(s)
    }
    pub fn pending(&mut self, valid: impl Fn(u16) -> bool) -> Option<String> {
        let mut message = None;
        self.send_pending(valid, |text| {
            message = Some(text.to_owned());
            true
        });
        message
    }
    pub fn resend(&mut self) {
        self.dirty = self.entries.keys().copied().collect();
        self.last_pf.clear();
    }
    /// CPF is independent of this schedule. Near-term EIT is checked every
    /// 100 ms; other EIT is batched at most once per two seconds.
    pub fn send_scheduled(
        &mut self,
        tick: std::time::Instant,
        now: i64,
        valid: impl Fn(u16) -> bool,
        send: impl FnOnce(&str) -> bool,
    ) {
        use std::time::Duration;
        if self
            .last_epg_check
            .is_some_and(|last| tick.duration_since(last) < Duration::from_millis(100))
        {
            return;
        }
        self.last_epg_check = Some(tick);
        let full = self
            .last_epg_batch
            .is_none_or(|last| tick.duration_since(last) >= Duration::from_secs(2));
        if full {
            self.last_epg_batch = Some(tick);
        }
        self.send_selected(
            valid,
            send,
            64,
            self.dirty.len(),
            if full { None } else { Some(now + 1800) },
        );
    }

    /// Commit queue removal only after the in-process transport accepted the batch.
    pub fn send_pending(&mut self, valid: impl Fn(u16) -> bool, send: impl FnOnce(&str) -> bool) {
        self.send_selected(valid, send, 10, self.dirty.len().min(10), None);
    }

    fn send_selected(
        &mut self,
        valid: impl Fn(u16) -> bool,
        send: impl FnOnce(&str) -> bool,
        limit: usize,
        scan: usize,
        before: Option<i64>,
    ) {
        let mut out = Vec::new();
        let mut keys = Vec::new();
        let mut bytes = 6;
        for _ in 0..scan {
            let Some(key) = self.dirty.pop_front() else {
                break;
            };
            if let Some(e) = self.entries.get(&key) {
                if (e.short || e.common.is_some())
                    && before.is_none_or(|cutoff| e.start <= cutoff)
                    && valid(e.sid)
                {
                    let text = e.json(true);
                    if !out.is_empty() && bytes + text.len() + 1 > 256 * 1024 {
                        self.dirty.push_front(key);
                        break;
                    }
                    bytes += text.len() + 1;
                    out.push(text);
                    keys.push(key);
                    if out.len() == limit {
                        break;
                    }
                } else {
                    self.dirty.push_back(key)
                }
            }
        }
        if !out.is_empty() && !send(&format!("EIT:[{}]", out.join(","))) {
            for key in keys.into_iter().rev() {
                self.dirty.push_front(key);
            }
        }
    }
}
fn valid_extended(d: &[u8]) -> bool {
    if d.len() < 6 || d[0] >> 4 > d[0] & 15 {
        return false;
    }
    let end = 5 + d[4] as usize;
    if end >= d.len() || end + 1 + d[end] as usize > d.len() {
        return false;
    }
    let mut p = 5;
    while p < end {
        let n = d[p] as usize;
        p += 1;
        if p + n >= end {
            return false;
        }
        p += n;
        let n = d[p] as usize;
        p += 1;
        if p + n > end {
            return false;
        }
        p += n;
    }
    true
}
fn extended_text(ds: &BTreeMap<u8, Vec<u8>>) -> String {
    let mut raw = Vec::new();
    let mut texts = Vec::new();
    for d in ds.values() {
        let end = 5 + d[4] as usize;
        let mut p = 5;
        while p < end {
            let n = d[p] as usize;
            p += 1;
            if n > 0 && !raw.is_empty() {
                texts.push(arib::decode(&raw, 4, 65536).text);
                raw.clear()
            }
            p += n;
            let n = d[p] as usize;
            p += 1;
            raw.extend_from_slice(&d[p..p + n]);
            p += n;
        }
        if d[end] > 0 {
            raw.extend_from_slice(&d[end + 1..end + 1 + d[end] as usize]);
        }
    }
    if !raw.is_empty() {
        texts.push(arib::decode(&raw, 4, 65536).text)
    }
    let mut content = String::new();
    for text in texts.into_iter().filter(|text| !text.is_empty()) {
        // Named items are separate paragraphs. Unnamed continuation fragments
        // have already been joined in raw before decoding, so keep them intact.
        if !content.is_empty() {
            // ARIB decoding emits CRLF. Ensure one blank line between items,
            // counting existing line breaks on both sides of the boundary.
            let trailing = content
                .chars()
                .rev()
                .take_while(|c| matches!(c, '\r' | '\n'))
                .filter(|&c| c == '\n')
                .count();
            let leading = text
                .chars()
                .take_while(|c| matches!(c, '\r' | '\n'))
                .filter(|&c| c == '\n')
                .count();
            for _ in trailing + leading..2 {
                content.push_str("\r\n");
            }
        }
        content.push_str(&text);
    }
    content
}

#[cfg(test)]
#[path = "../tests/unit/events.rs"]
mod tests;
