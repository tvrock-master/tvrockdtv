//! CDT logos are matched to services by the SDT logo transmission descriptor.
use crate::ts::{be16, descriptors, length12};
use std::collections::{BTreeMap, HashSet};

type ServiceKey = (u16, u16); // ONID, SID
type LogoLink = (u16, Option<(u16, u16)>); // logo_id, (download_data_id, version)

#[derive(Default)]
pub struct Logos {
    links: BTreeMap<ServiceKey, LogoLink>,
    images: BTreeMap<(u16, u16, u16, u16, u8), Vec<u8>>,
    sent: HashSet<(u16, u16, u8)>,
}
impl Logos {
    pub fn reset_stream(&mut self) {
        self.links.clear();
        self.images.clear();
    }
    pub fn sdt(&mut self, s: &[u8]) {
        if s.len() < 15 {
            return;
        }
        let onid = be16(&s[8..]);
        let end = s.len() - 4;
        let mut p = 11;
        let mut links = Vec::new();
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
            let mut link = None;
            for (tag, d) in ds {
                if tag != 0xcf {
                    continue;
                }
                match d.first() {
                    Some(1) if d.len() == 7 => {
                        link = Some((
                            be16(&d[1..]) & 511,
                            Some((be16(&d[5..]), be16(&d[3..]) & 4095)),
                        ))
                    }
                    Some(2) if d.len() == 3 => link = Some((be16(&d[1..]) & 511, None)),
                    _ => (),
                }
            }
            links.push(((onid, be16(&s[p..])), link));
            p = next;
        }
        for (key, link) in links {
            self.links.remove(&key);
            if let Some(link) = link
                && self.links.len() < 4096
            {
                self.links.insert(key, link);
            }
        }
    }
    pub fn cdt(&mut self, s: &[u8]) {
        if s.len() < 24 || s[10] != 1 {
            return;
        }
        let end = s.len() - 4;
        let start = 13 + length12(&s[11..]);
        if start + 7 > end || descriptors(&s[13..start]).is_none() {
            return;
        }
        let d = &s[start..end];
        // ARIB logo module has a 16-bit data_size (not a 12-bit field).
        let size = be16(&d[5..]) as usize;
        if d[0] > 5 || size == 0 || size + 7 != d.len() {
            return;
        }
        let key = (
            be16(&s[8..]),
            be16(&s[3..]),
            be16(&d[1..]) & 511,
            be16(&d[3..]) & 4095,
            d[0],
        );
        if self.images.len() < 1024 || self.images.contains_key(&key) {
            self.images.insert(key, d[7..].to_vec());
        }
    }
    pub fn send_pending(
        &mut self,
        valid: impl Fn(u16) -> bool,
        mut send: impl FnMut(&str) -> bool,
    ) {
        for (&(onid, sid), &(id, download)) in &self.links {
            if !valid(sid) {
                continue;
            }
            for (&(network, download_id, logo_id, version, kind), data) in &self.images {
                let key = (onid, sid, kind);
                if network != onid
                    || id != logo_id
                    || download.is_some_and(|v| v != (download_id, version))
                    || self.sent.contains(&key)
                {
                    continue;
                }
                let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
                let msg = format!(
                    "CDT:{}",
                    serde_json::json!({"Sid":sid,"Onid":onid,"LogoType":kind,"LogoId":id,"LogoVersion":version,"Data":hex})
                );
                if send(&msg) {
                    self.sent.insert(key);
                }
            }
        }
    }
}
