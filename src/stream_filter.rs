//! Service/PID selection for UDP output. Reuses the core's PSI reassembler/CRC.
use crate::ts::{Receiver, be16, crc32, descriptors, length12};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Table {
    identity: Option<(u16, u8, u8)>,
    sections: BTreeMap<u8, Vec<u8>>,
}
impl Table {
    fn accept(&mut self, s: Vec<u8>) -> Option<Vec<Vec<u8>>> {
        if s.len() < 12 || s[1] & 0x80 == 0 || s[5] & 1 == 0 || s[6] > s[7] || crc32(&s) != 0 {
            return None;
        }
        let id = (be16(&s[3..]), s[5] >> 1, s[7]);
        if self.identity != Some(id) {
            self.identity = Some(id);
            self.sections.clear();
        }
        self.sections.insert(s[6], s);
        (self.sections.len() == usize::from(id.2) + 1)
            .then(|| self.sections.values().cloned().collect())
    }
}
#[derive(Default)]
struct Psi {
    receiver: Receiver,
    table: Table,
}
impl Psi {
    fn accept(&mut self, p: &[u8; 188]) -> Vec<Vec<Vec<u8>>> {
        self.receiver
            .sections(p)
            .into_iter()
            .filter_map(|s| self.table.accept(s))
            .collect()
    }
}
pub(crate) struct Filter {
    flags: u32,
    parent: u16,
    selected: u16,
    pat: Psi,
    cat: Psi,
    pmts: BTreeMap<u16, Psi>,
    services: BTreeMap<u16, u16>,
    streams: BTreeMap<u16, BTreeSet<u16>>,
    emm: BTreeSet<u16>,
    allowed: Box<[bool; 8192]>,
    pat_header: Option<[u8; 8]>,
    pat_cc: u8,
}
impl Filter {
    pub fn new(service: u16, flags: u32) -> Self {
        Self {
            flags,
            parent: service,
            selected: if flags & 8 != 0 { 0 } else { service },
            pat: Psi::default(),
            cat: Psi::default(),
            pmts: BTreeMap::new(),
            services: BTreeMap::new(),
            streams: BTreeMap::new(),
            emm: BTreeSet::new(),
            allowed: Box::new([false; 8192]),
            pat_header: None,
            pat_cc: 0,
        }
    }
    pub fn selected(&self) -> u16 {
        self.selected
    }
    fn ca(pids: &mut BTreeSet<u16>, data: &[u8]) {
        if let Some(desc) = descriptors(data) {
            for (tag, b) in desc {
                if tag == 9 && b.len() >= 4 {
                    pids.insert(be16(&b[2..]) & 8191);
                }
            }
        }
    }
    fn update_allowed(&mut self) {
        self.allowed.fill(false);
        for (&service, &pid) in &self.services {
            if (self.flags & 1 == 0 && self.flags & 8 == 0) || service == self.selected {
                self.allowed[pid as usize] = true;
                if let Some(pids) = self.streams.get(&pid) {
                    for &p in pids {
                        if p < 8191 {
                            self.allowed[p as usize] = true;
                        }
                    }
                }
            }
        }
        for &pid in &self.emm {
            if pid < 8191 {
                self.allowed[pid as usize] = true;
            }
        }
    }
    fn pat(&mut self, p: &[u8; 188]) {
        for sections in self.pat.accept(p) {
            if sections
                .iter()
                .any(|s| s[0] != 0 || (s.len() - 12) % 4 != 0)
            {
                continue;
            }
            let mut services = BTreeMap::new();
            for s in &sections {
                if s[0] != 0 || (s.len() - 12) % 4 != 0 {
                    continue;
                }
                for entry in s[8..s.len() - 4].as_chunks::<4>().0 {
                    let service = be16(entry);
                    let pid = be16(&entry[2..]) & 8191;
                    if service != 0 && (0x30..8191).contains(&pid) {
                        services.insert(service, pid);
                    }
                }
            }
            let mut header = [0; 8];
            header.copy_from_slice(&sections[0][..8]);
            self.pat_header = Some(header);
            if self.flags & 8 != 0 {
                self.selected = services
                    .iter()
                    .filter(|(_, pid)| (0x1fc8..=0x1fcf).contains(*pid))
                    .min_by_key(|(_, pid)| **pid)
                    .map_or(0, |(&sid, _)| sid);
            }
            let keep: BTreeSet<u16> = services.values().copied().collect();
            self.pmts.retain(|pid, _| keep.contains(pid));
            self.streams.retain(|pid, _| keep.contains(pid));
            for &pid in &keep {
                self.pmts.entry(pid).or_default();
            }
            if services != self.services {
                self.streams.clear();
            }
            self.services = services;
            self.update_allowed();
        }
    }
    fn pmt(&mut self, pid: u16, p: &[u8; 188]) {
        let Some(psi) = self.pmts.get_mut(&pid) else {
            return;
        };
        for sections in psi.accept(p) {
            let mut pids = BTreeSet::new();
            let mut valid = true;
            for s in sections {
                if s[0] != 2 || s.len() < 16 || self.services.get(&be16(&s[3..])) != Some(&pid) {
                    valid = false;
                    break;
                }
                pids.insert(be16(&s[8..]) & 8191);
                let end = s.len() - 4;
                let n = length12(&s[10..]);
                let mut pos = 12 + n;
                if pos > end {
                    valid = false;
                    break;
                }
                Self::ca(&mut pids, &s[12..pos]);
                while pos < end {
                    if end - pos < 5 {
                        valid = false;
                        break;
                    }
                    let typ = s[pos];
                    let es = be16(&s[pos + 1..]) & 8191;
                    let next = pos + 5 + length12(&s[pos + 3..]);
                    if next > end {
                        valid = false;
                        break;
                    }
                    let allowed = if self.flags & 6 == 6 {
                        true
                    } else {
                        matches!(
                            typ,
                            1 | 2
                                | 3
                                | 4
                                | 0xf
                                | 0x10
                                | 0x11
                                | 0x1b
                                | 0x24
                                | 0x81
                                | 0x82
                                | 0x83
                                | 0x87
                        ) || (typ == 6 && self.flags & 2 != 0)
                            || (typ == 0xd && self.flags & 4 != 0)
                    };
                    if allowed {
                        pids.insert(es);
                        Self::ca(&mut pids, &s[pos + 5..next]);
                    }
                    pos = next;
                }
            }
            if valid {
                self.streams.insert(pid, pids);
                self.update_allowed();
            }
        }
    }
    fn selected_pat(&mut self) -> Option<[u8; 188]> {
        let header = self.pat_header?;
        let pid = *self.services.get(&self.selected)?;
        let mut section = header.to_vec();
        section[1] = 0xb0;
        section[2] = 17;
        section[6] = 0;
        section[7] = 0;
        section.extend_from_slice(&[
            0,
            0,
            0xe0,
            0x10,
            (self.selected >> 8) as u8,
            self.selected as u8,
            0xe0 | ((pid >> 8) as u8),
            pid as u8,
        ]);
        section.extend_from_slice(&crc32(&section).to_be_bytes());
        let mut out = [0xff; 188];
        out[..5].copy_from_slice(&[0x47, 0x40, 0, 0x10 | self.pat_cc, 0]);
        self.pat_cc = (self.pat_cc + 1) & 15;
        out[5..5 + section.len()].copy_from_slice(&section);
        Some(out)
    }
    pub fn packet(&mut self, p: &[u8; 188]) -> Option<[u8; 188]> {
        if p[0] != 0x47 || p[1] & 0x80 != 0 || (p[3] >> 4) & 3 == 0 {
            return None;
        }
        if (p[3] >> 4) & 2 != 0 && p[4] > 183 {
            return None;
        }
        let pid = be16(&p[1..]) & 8191;
        if pid == 0 {
            self.pat(p)
        } else if pid == 1 {
            for sections in self.cat.accept(p) {
                let mut emm = BTreeSet::new();
                for s in sections {
                    if s[0] == 1 {
                        Self::ca(&mut emm, &s[8..s.len() - 4]);
                    }
                }
                self.emm = emm;
                self.update_allowed();
            }
        } else {
            self.pmt(pid, p)
        }
        if self.parent == 0 || self.parent == 0xffff || (self.flags & 8 != 0 && self.selected == 0)
        {
            return None;
        }
        // All services with all ES types can bypass PMT discovery.
        if self.flags & 1 == 0 && self.flags & 8 == 0 && self.flags & 6 == 6 {
            return Some(*p);
        }
        if pid == 0 && self.flags & 1 != 0 {
            return self.selected_pat();
        }
        (pid < 0x30 || self.allowed[pid as usize]).then_some(*p)
    }
}
