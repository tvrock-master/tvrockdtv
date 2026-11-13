use crate::{
    caption::Caption,
    events::Events,
    si::ServiceInfo,
    time,
    ts::{self, Counter, Receiver, be16, length12},
};
use std::collections::{BTreeMap, HashMap};
#[derive(Default)]
pub struct Engine {
    receivers: HashMap<u16, Receiver>,
    pub counters: BTreeMap<u16, Counter>,
    pub events: Events,
    si: ServiceInfo,
    pub logos: crate::cdt::Logos,
    dsm: crate::dsm::Streams,
    pmts: HashMap<u16, u16>,
    pat_id: Option<(u16, u8, u8)>,
    pat_sections: BTreeMap<u8, Vec<u8>>,
    pmt_sections: HashMap<u16, Vec<u8>>,
    captions: HashMap<u16, Caption>,
    partial: Vec<u8>,
    pub broadcast: Option<(i64, i64)>,
    pub channel: u32,
    pub service: u16,
    pub device: u32,
    last_info: i64,
}
impl Engine {
    /// PID, service membership and profile for offline diagnostics.
    pub fn caption_streams(&self) -> Vec<(u16, Vec<u16>, bool)> {
        self.captions
            .iter()
            .map(|(pid, c)| (*pid, c.services.clone(), c.oneseg))
            .collect()
    }
    pub fn reset(&mut self, channel: u32, service: u16, device: u32) {
        let mut logos = std::mem::take(&mut self.logos);
        logos.reset_stream();
        *self = Self {
            logos,
            channel,
            service,
            device,
            ..Self::default()
        };
    }
    pub fn process(&mut self, mut data: &[u8]) -> Vec<String> {
        let mut messages = Vec::new();
        while !data.is_empty() {
            if self.partial.is_empty() {
                let Some(n) = data.iter().position(|b| *b == 0x47) else {
                    break;
                };
                data = &data[n..];
            }
            if self.partial.is_empty() && data.len() >= 188 {
                messages.extend(self.packet(data[..188].try_into().unwrap()));
                data = &data[188..];
            } else {
                let n = (188 - self.partial.len()).min(data.len());
                self.partial.extend_from_slice(&data[..n]);
                data = &data[n..];
                if self.partial.len() == 188 {
                    let packet: [u8; 188] = self.partial[..].try_into().unwrap();
                    self.partial.clear();
                    messages.extend(self.packet(&packet));
                }
            }
        }
        messages
    }
    pub fn packet(&mut self, p: &[u8; 188]) -> Vec<String> {
        let mut out = Vec::new();
        if p[0] != 0x47 {
            return out;
        }
        let pid = (((p[1] & 31) as u16) << 8) | p[2] as u16;
        out.extend(self.dsm.clock(
            p,
            self.service,
            self.pat_id.map_or(0, |id| id.0),
            self.broadcast,
        ));
        if pid != 8191 {
            self.counters.entry(pid).or_default().packet(p);
        }
        if let Some(c) = self.captions.get_mut(&pid) {
            return c.packet(p, pid, self.service);
        }
        if !matches!(pid, 0 | 0x10 | 0x11 | 0x12 | 0x14 | 0x27 | 0x29)
            && !self.pmts.contains_key(&pid)
            && !self.dsm.contains(pid)
        {
            return out;
        }
        let sections = self.receivers.entry(pid).or_default().sections(p);
        for s in sections {
            if s.len() < 3 {
                continue;
            }
            let tid = s[0];
            if pid == 0x14 && tid == 0x70 {
                if s.len() == 8 && s[1] & 0x80 == 0 {
                    self.clock(&s[3..8]);
                }
                continue;
            }
            if ts::crc32(&s) != 0 {
                continue;
            }
            if tid == 0x73 && pid == 0x14 {
                if s.len() >= 14
                    && 10 + length12(&s[8..]) + 4 == s.len()
                    && ts::descriptors(&s[10..s.len() - 4]).is_some()
                {
                    self.clock(&s[3..8]);
                }
                continue;
            }
            if self.dsm.contains(pid) && matches!(tid, 0x3b..=0x3d) {
                out.extend(self.dsm.section(pid, &s, self.pat_id.map_or(0, |id| id.0)));
                continue;
            }
            if s.len() < 8 || s[1] & 0x80 == 0 || s[5] & 1 == 0 || s[6] > s[7] {
                continue;
            }
            match (pid, tid) {
                (0, 0) => self.pat(&s),
                (0x10, 0x40) => out.extend(self.si.nit(&s)),
                (0x29, 0xc8) => self.logos.cdt(&s),
                (0x11, 0x42 | 0x46) => {
                    self.logos.sdt(&s);
                    if tid == 0x42 && s.len() >= 15 {
                        self.dsm.onid = be16(&s[8..]);
                    }
                    if let Some(msg) = self.si.sdt(&s) {
                        out.push(msg)
                    }
                }
                (0x12 | 0x27, 0x4e..=0x6f) => out.extend(self.events.section(&s, time::now())),
                (_, 2) if self.pmts.contains_key(&pid) => {
                    self.pmt(pid, &s);
                    if let Some(msg) = self
                        .dsm
                        .manifest(self.pmts[&pid], self.pat_id.map_or(0, |id| id.0))
                    {
                        out.push(msg);
                    }
                }
                _ => (),
            }
        }
        out
    }
    fn clock(&mut self, b: &[u8]) {
        if let Some(t) = time::timestamp(b) {
            self.broadcast = Some((t, time::now()));
        }
    }
    fn pat(&mut self, s: &[u8]) {
        if s.len() < 12 || !(s.len() - 12).is_multiple_of(4) {
            return;
        }
        let id = (be16(&s[3..]), (s[5] >> 1) & 31, s[7]);
        if self.pat_id != Some(id) {
            self.pat_id = Some(id);
            self.pat_sections.clear();
        }
        self.pat_sections.insert(s[6], s.to_vec());
        if self.pat_sections.len() != s[7] as usize + 1 {
            return;
        }
        let mut pmts = HashMap::new();
        for s in self.pat_sections.values() {
            for b in s[8..s.len() - 4].as_chunks::<4>().0 {
                let service = be16(b);
                let pid = be16(&b[2..]) & 8191;
                if service != 0 && (0x20..8191).contains(&pid) && pmts.len() < 256 {
                    pmts.insert(pid, service);
                }
            }
        }
        if pmts != self.pmts {
            self.receivers.retain(|pid, _| !self.pmts.contains_key(pid));
            self.pmt_sections.clear();
            self.captions.clear();
            self.dsm = Default::default();
            self.receivers.clear();
            self.pmts = pmts;
        }
    }
    fn pmt(&mut self, pid: u16, s: &[u8]) {
        if s.len() < 16
            || self.pmts.get(&pid) != Some(&be16(&s[3..]))
            || self.pmt_sections.get(&pid).is_some_and(|old| old == s)
        {
            return;
        }
        let end = s.len() - 4;
        let mut p = 12 + length12(&s[10..]);
        if p > end || ts::descriptors(&s[12..p]).is_none() {
            return;
        }
        let mut caps = Vec::new();
        while p < end {
            if p + 5 > end {
                return;
            }
            let next = p + 5 + length12(&s[p + 3..]);
            if next > end {
                return;
            }
            let Some(ds) = ts::descriptors(&s[p + 5..next]) else {
                return;
            };
            if s[p] == 6 {
                let mut caption = false;
                let mut oneseg = false;
                for (tag, d) in ds {
                    if tag == 0x52 && !d.is_empty() {
                        caption |= (0x30..=0x37).contains(&d[0]) || (0x87..=0x88).contains(&d[0]);
                        oneseg |= (0x87..=0x88).contains(&d[0]);
                    }
                    if tag == 0xfd && d.len() >= 2 && be16(d) == 8 {
                        caption = true;
                    }
                }
                if caption {
                    caps.push((be16(&s[p + 1..]) & 8191, oneseg));
                }
            }
            p = next;
        }
        let sid = be16(&s[3..]);
        self.pmt_sections.insert(pid, s.to_vec());
        self.dsm.pmt(s);
        self.receivers.retain(|pid, _| {
            matches!(*pid, 0 | 0x10 | 0x11 | 0x12 | 0x14 | 0x27 | 0x29)
                || self.pmts.contains_key(pid)
                || self.dsm.contains(*pid)
        });
        for c in self.captions.values_mut() {
            c.services.retain(|id| *id != sid)
        }
        self.captions.retain(|_, c| !c.services.is_empty());
        for (pid, one) in caps {
            if self.captions.len() < 32 {
                let c = self.captions.entry(pid).or_default();
                if !c.services.contains(&sid) {
                    c.services.push(sid)
                }
                c.oneseg = one;
            }
        }
    }
    pub fn status(
        &mut self,
        scard: u32,
        signal: f32,
        bitrate: u32,
        error: u32,
        scramble: u32,
    ) -> Vec<String> {
        let Some((t, _)) = self.broadcast else {
            return vec![];
        };
        if t == self.last_info {
            return vec![];
        }
        self.last_info = t;
        let pki = self
            .counters
            .iter()
            .map(|(pid, c)| {
                format!(
                    "{{\"pid\":{pid},\"total\":{},\"drop\":{},\"scramble\":{}}}",
                    c.total, c.drop, c.scramble
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        vec![
            format!(
                "DIF:{{\"unixtime\":{t},\"events\":{},\"ct\":0,\"scard\":{scard},\"signal\":{},\"bitrate\":{bitrate},\"error\":{error},\"scramble\":{scramble}}}",
                self.events.entries.len(),
                (signal * 1000.) as u32
            ),
            format!("PKI:[{pki}]"),
        ]
    }
}
