use crate::{
    arib,
    json::quote,
    ts::{be16, descriptors, length12},
};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct ServiceInfo {
    nit_id: Option<(u16, u8, u8)>,
    nit: BTreeMap<u8, Vec<u8>>,
    sdt: Vec<Vec<u8>>,
}
impl ServiceInfo {
    pub fn nit(&mut self, s: &[u8]) -> Vec<String> {
        if s.len() < 16 || s[0] != 0x40 {
            return vec![];
        }
        let end = s.len() - 4;
        let ne = 10 + length12(&s[8..]);
        if ne + 2 > end || descriptors(&s[10..ne]).is_none() || ne + 2 + length12(&s[ne..]) != end {
            return vec![];
        }
        let mut p = ne + 2;
        while p < end {
            if p + 6 > end {
                return vec![];
            }
            let next = p + 6 + length12(&s[p + 4..]);
            if next > end {
                return vec![];
            }
            let Some(ds) = descriptors(&s[p + 6..next]) else {
                return vec![];
            };
            if ds.iter().any(|(t, d)| *t == 0xfb && d.len() % 2 != 0) {
                return vec![];
            }
            p = next;
        }
        let id = (be16(&s[3..]), (s[5] >> 1) & 31, s[7]);
        if self.nit_id != Some(id) {
            self.nit_id = Some(id);
            self.nit.clear()
        }
        if self.nit.get(&s[6]).is_some_and(|old| old == s) {
            return vec![];
        }
        self.nit.insert(s[6], s.to_vec());
        self.sdt.iter().filter_map(|s| self.message(s)).collect()
    }
    fn oneseg(&self, onid: u16, tsid: u16, sid: u16) -> i32 {
        let Some((_, _, last)) = self.nit_id else {
            return -1;
        };
        if self.nit.len() != last as usize + 1 {
            return -1;
        }
        for s in self.nit.values() {
            let mut p = 12 + length12(&s[8..]);
            while p < s.len() - 4 {
                let next = p + 6 + length12(&s[p + 4..]);
                if be16(&s[p..]) == tsid && be16(&s[p + 2..]) == onid {
                    for (t, d) in descriptors(&s[p + 6..next]).unwrap() {
                        if t == 0xfb && d.as_chunks::<2>().0.iter().any(|b| be16(b) == sid) {
                            return 1;
                        }
                    }
                    return 0;
                }
                p = next;
            }
        }
        -1
    }
    pub fn sdt(&mut self, s: &[u8]) -> Option<String> {
        let message = self.message(s)?;
        self.sdt.retain(|old| {
            !(old[0] == s[0]
                && old[3..5] == s[3..5]
                && old[8..10] == s[8..10]
                && (old[5] != s[5] || old[6] == s[6]))
        });
        if self.sdt.len() >= 512 {
            self.sdt.remove(0);
        }
        self.sdt.push(s.to_vec());
        Some(message)
    }
    fn message(&self, s: &[u8]) -> Option<String> {
        if s.len() < 15 || !matches!(s[0], 0x42 | 0x46) {
            return None;
        }
        let mut items = Vec::new();
        let end = s.len() - 4;
        let mut p = 11;
        while p < end {
            if p + 5 > end {
                return None;
            }
            let next = p + 5 + length12(&s[p + 3..]);
            if next > end {
                return None;
            }
            for (tag, d) in descriptors(&s[p + 5..next])? {
                if tag != 0x48 {
                    continue;
                }
                if d.len() < 3 {
                    return None;
                }
                let pn = d[1] as usize;
                if pn + 3 > d.len() || d[pn + 2] as usize + pn + 3 != d.len() {
                    return None;
                }
                let provider = arib::decode(&d[2..2 + pn], 4, 256).text;
                let name = arib::decode(&d[3 + pn..], 4, 256).text;
                items.push(format!("{{\"TableId\":{},\"Tsid\":{},\"Onid\":{},\"Sid\":{},\"Version\":{},\"Section\":{},\"LastSection\":{},\"ServiceType\":{},\"OneSeg\":{},\"ProviderName\":{},\"ServiceName\":{},\"EitSchedule\":{},\"EitPresentFollowing\":{},\"RunningStatus\":{},\"FreeCA\":{}}}",s[0],be16(&s[3..]),be16(&s[8..]),be16(&s[p..]),(s[5]>>1)&31,s[6],s[7],d[0],self.oneseg(be16(&s[8..]),be16(&s[3..]),be16(&s[p..])),quote(&provider),quote(&name),(s[p+2]>>1)&1,s[p+2]&1,s[p+3]>>5,(s[p+3]>>4)&1));
            }
            p = next;
        }
        if items.is_empty() {
            None
        } else {
            Some(format!("SDT:[{}]", items.join(",")))
        }
    }
}
