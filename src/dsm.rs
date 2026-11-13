//! Validated DSM-CC sections, with PMT routing, for server-side carousel assembly.
use crate::ts::{be16, descriptors, length12};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Component {
    pub pid: u16,
    pub component_tag: u8,
    pub data_component_id: u16,
    pub info: String,
}
#[derive(Default)]
pub struct Streams {
    services: BTreeMap<u16, Vec<Component>>,
    pub onid: u16,
    manifests: BTreeMap<u16, (String, i64)>,
    pcr_pids: BTreeMap<u16, u16>,
    pcr_last: BTreeMap<u16, u64>,
}
pub fn hex(bytes: &[u8]) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}
impl Streams {
    pub fn manifest(&mut self, sid: u16, tsid: u16) -> Option<String> {
        let components = self.services.get(&sid)?;
        if components.is_empty() && !self.manifests.contains_key(&sid) {
            return None;
        }
        let msg = format!(
            "DSM:{}",
            serde_json::json!({"Kind":"pmt","Sid":sid,"Tsid":tsid,"Onid":self.onid,"Components":components})
        );
        let now = crate::time::now();
        if self
            .manifests
            .get(&sid)
            .is_some_and(|(old, t)| old == &msg && now - *t < 5)
        {
            return None;
        }
        self.manifests.insert(sid, (msg.clone(), now));
        Some(msg)
    }
    pub fn clock(
        &mut self,
        p: &[u8; 188],
        sid: u16,
        tsid: u16,
        broadcast: Option<(i64, i64)>,
    ) -> Vec<String> {
        let pid = be16(&p[1..]) & 8191;
        if self.pcr_pids.get(&sid) != Some(&pid)
            || p[1] & 128 != 0
            || p[3] & 0xc0 != 0
            || p[3] & 0x20 == 0
            || p[4] < 7
            || p[4] > 183
            || p[5] & 16 == 0
        {
            return vec![];
        }
        let base = ((p[6] as u64) << 25)
            | ((p[7] as u64) << 17)
            | ((p[8] as u64) << 9)
            | ((p[9] as u64) << 1)
            | ((p[10] as u64) >> 7);
        if p[5] & 128 == 0
            && self
                .pcr_last
                .get(&sid)
                .is_some_and(|old| base.wrapping_sub(*old) & 0x1ffffffff < 9000)
        {
            return vec![];
        }
        self.pcr_last.insert(sid, base);
        let ext = (((p[10] & 1) as u16) << 8) | p[11] as u16;
        let clock = broadcast.map(|(t, received)| (t + crate::time::now() - received) * 1000);
        vec![format!(
            "DSM:{}",
            serde_json::json!({"Sid":sid,"Tsid":tsid,"Onid":self.onid,"PCRBase":base,"PCRExtension":ext,"TimeUnixMillis":clock})
        )]
    }
    pub fn contains(&self, pid: u16) -> bool {
        self.services
            .values()
            .any(|v| v.iter().any(|c| c.pid == pid))
    }
    pub fn pmt(&mut self, s: &[u8]) {
        if s.len() < 16 {
            return;
        }
        let end = s.len() - 4;
        let mut p = 12 + length12(&s[10..]);
        if p > end {
            return;
        }
        let mut list = Vec::new();
        while p < end {
            if p + 5 > end {
                return;
            }
            let next = p + 5 + length12(&s[p + 3..]);
            if next > end {
                return;
            }
            let Some(ds) = descriptors(&s[p + 5..next]) else {
                return;
            };
            if matches!(s[p], 0x0b | 0x0d) {
                let mut c = Component {
                    pid: be16(&s[p + 1..]) & 8191,
                    component_tag: 0xff,
                    data_component_id: 0,
                    info: String::new(),
                };
                for (tag, d) in ds {
                    if tag == 0x52 && !d.is_empty() {
                        c.component_tag = d[0];
                    }
                    if tag == 0xfd && d.len() >= 2 {
                        c.data_component_id = be16(d);
                        c.info = hex(&d[2..]);
                    }
                }
                if list.len() < 32 {
                    list.push(c);
                }
            }
            p = next;
        }
        let sid = be16(&s[3..]);
        if self.pcr_pids.len() < 256 {
            self.pcr_pids.insert(sid, be16(&s[8..]) & 8191);
        }
        if self.services.len() < 256 || self.services.contains_key(&sid) {
            self.services.insert(sid, list);
        }
    }
    pub fn section(&self, pid: u16, s: &[u8], tsid: u16) -> Vec<String> {
        // DSM-CC DDB version/current fields are not ordinary PSI semantics.
        if s.len() < 12 || !matches!(s[0], 0x3b..=0x3d) {
            return vec![];
        }
        self.services.iter().filter(|(_,cs)|cs.iter().any(|c|c.pid==pid)).map(|(&sid,cs)| {
            format!("DSM:{}",serde_json::json!({"Sid":sid,"Tsid":tsid,"Onid":self.onid,"Pid":pid,"Components":cs,"Data":hex(s)}))
        }).collect()
    }
}
