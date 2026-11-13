use tvrockdtv::{arib, engine::Engine, events::Events, json, time, ts};
fn section(mut s: Vec<u8>, syntax: bool) -> Vec<u8> {
    let n = s.len() + 1;
    s[1] = (if syntax { 0xb0 } else { 0x70 }) | ((n >> 8) as u8);
    s[2] = n as u8;
    let crc = ts::crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}
fn packet(pid: u16, data: &[u8], cc: u8, start: bool, pointer: bool, exact: bool) -> [u8; 188] {
    let mut p = [255; 188];
    p[0] = 0x47;
    p[1] = (pid >> 8) as u8 | if start { 64 } else { 0 };
    p[2] = pid as u8;
    p[3] = 0x10 | cc;
    let mut at = 4;
    if exact {
        p[3] |= 0x20;
        let n = 183 - data.len() - usize::from(pointer);
        p[4] = n as u8;
        if n > 0 {
            p[5] = 0
        }
        at = 5 + n;
    }
    if pointer {
        p[at] = 0;
        at += 1;
    }
    assert!(at + data.len() <= 188);
    p[at..at + data.len()].copy_from_slice(data);
    p
}
fn eit(eid: u8, extra: &[u8]) -> Vec<u8> {
    let mut s = vec![
        0x4e, 0, 0, 0, 101, 0xc1, 0, 1, 0x12, 0x34, 0, 4, 1, 0x4e, 0, eid, 0xeb, 0x96, 0x12, 0, 0,
        1, 0, 0, 0x80, 9,
    ];
    s.extend_from_slice(extra);
    s[25] += extra.len() as u8;
    s.extend_from_slice(&[0x4d, 7, b'j', b'p', b'n', 2, 0x0e, b'A', 0]);
    section(s, true)
}
fn fixed_now() -> i64 {
    time::timestamp(&[0xeb, 0x96, 0x12, 0, 0]).unwrap()
}
fn sdt() -> Vec<u8> {
    section(
        vec![
            0x42, 0, 0, 0x12, 0x34, 0xc3, 0, 0, 0, 4, 0xff, 0, 101, 0xff, 0x80, 7, 0x48, 5, 1, 0,
            2, 0x0e, b'A',
        ],
        true,
    )
}
fn nit(sid: u8, version: u8, number: u8, last: u8) -> Vec<u8> {
    section(
        vec![
            0x40,
            0,
            0,
            0,
            4,
            0xc1 | (version << 1),
            number,
            last,
            0xf0,
            0,
            0xf0,
            10,
            0x12,
            0x34,
            0,
            4,
            0xf0,
            4,
            0xfb,
            2,
            0,
            sid,
        ],
        true,
    )
}
#[test]
fn arib_vectors() {
    for (input, expected) in [
        (vec![0x24, 0x22], "あ"),
        (vec![0xa2], "あ"),
        (vec![0x0e, 0x41], "Ａ"),
        (vec![0x0e, 0x89, 0x41, 0x5c, 0x7e], "A¥‾"),
        (vec![0x24, 0x0e, 0x41], "Ａ"),
        (vec![0x98, 0x43, 0x20], "　　　"),
        (vec![0x1b, 0x24, 0x29, 0x42, 0x0e, 0x24, 0x22], "あ"),
    ] {
        assert_eq!(arib::decode(&input, 4, 256).text, expected)
    }
    assert_eq!(arib::decode(&[0x7e, 0x41], 8, 2).text, "");
    assert_eq!(arib::decode(&[0x98, 0x43, 0x7e, 0x41], 8, 4).text, "🄐");
    assert_eq!(arib::decode(&[0x1d, 0x6f, 0x41], 5, 2).text, "Ａ");
}
#[test]
fn malformed_arib_is_bounded() {
    let mut rng = 0x12345678u32;
    for run in 0..25000 {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = 1 + rng as usize % 128;
        let mut data = vec![0; n];
        for b in &mut data {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (rng >> 24) as u8
        }
        let cap = 1 + (rng >> 8) as usize % 32;
        let out = arib::decode(&data, run & 15, cap);
        assert!(out.text.encode_utf16().count() < cap);
    }
}
#[test]
fn section_split_duplicate_gap_and_pointer() {
    let s = eit(7, &[]);
    let first = packet(0x12, &s[..10], 15, true, true, true);
    let rest = packet(0x12, &s[10..], 0, false, false, false);
    let mut r = ts::Receiver::default();
    assert!(r.sections(&first).is_empty());
    assert!(r.sections(&first).is_empty());
    assert_eq!(r.sections(&rest), vec![s.clone()]);
    let mut r = ts::Receiver::default();
    r.sections(&first);
    let mut bad = rest;
    bad[3] = 0x11;
    assert!(r.sections(&bad).is_empty());
    assert_eq!(
        r.sections(&packet(0x12, &s, 2, true, true, false)),
        vec![s.clone()]
    );
    let mut r = ts::Receiver::default();
    r.sections(&first);
    let mut tail = vec![(s.len() - 10) as u8];
    tail.extend_from_slice(&s[10..]);
    tail.extend_from_slice(&eit(8, &[]));
    assert_eq!(
        r.sections(&packet(0x12, &tail, 0, true, false, false))
            .len(),
        2
    );
}
#[test]
fn tdt_tot_split_and_reset() {
    let s = [0x70, 0x70, 5, 0xeb, 0x96, 0x12, 0, 0];
    let p = packet(0x14, &s, 0, true, true, false);
    for split in 0..188 {
        let mut e = Engine::default();
        e.process(&p[..split]);
        assert!(e.broadcast.is_none());
        e.process(&p[split..]);
        assert_eq!(e.broadcast.unwrap().0, fixed_now());
        e.reset(2, 101, 7);
        assert!(e.broadcast.is_none());
    }
    let mut e = Engine::default();
    let tot = section(vec![0x73, 0, 0, 0xeb, 0x96, 0x12, 0, 1, 0xf0, 0], false);
    e.process(&packet(0x14, &tot, 0, true, true, false));
    assert_eq!(e.broadcast.unwrap().0, fixed_now() + 1);
    let mut tot = tot;
    tot[4] ^= 1;
    let mut e = Engine::default();
    e.process(&packet(0x14, &tot, 0, true, true, false));
    assert!(e.broadcast.is_none());
}
#[test]
fn timestamps_are_jst_and_64_bit() {
    assert_eq!(ts::crc32(b"123456789"), 0x0376e6e7);
    assert_eq!(time::timestamp(&[0x9e, 0x8b, 0x09, 0, 0]), Some(0));
    assert!(time::timestamp(&[0xeb, 0x96, 0x29, 0, 0]).is_none());
    assert!(time::duration(&[1, 0x6a, 0]).is_none());
    assert_eq!(time::duration(&[255; 3]), None);
    assert_eq!(json::quote("a\"\\\n"), "\"a\\\"\\\\\\u000a\"");
}
#[test]
fn event_updates_and_descriptors_before_title() {
    let mut e = Events::default();
    e.section(&eit(7, &[0x54, 2, 0x12, 0]), fixed_now());
    assert_eq!(e.entries.len(), 1);
    let sent = e.pending(|_| true).unwrap();
    assert!(sent.contains("\"Title\":\"Ａ\""));
    assert!(sent.contains("\"Cnib_a\":1"));
    assert!(e.pending(|_| true).is_none());
    let mut s = eit(7, &[0x54, 2, 0x12, 0]);
    s[21..24].fill(255);
    e.section(&s, fixed_now());
    assert!(e.pending(|_| true).unwrap().contains("\"End\":-1"));
    assert!(e.current(101).is_some());
    assert!(e.current(101).is_none());
}
#[test]
fn malformed_event_sections_do_not_commit() {
    let mut e = Events::default();
    let mut s = eit(7, &[]);
    s[25] = 255;
    e.section(&s, fixed_now());
    assert!(e.entries.is_empty());
    let mut s = eit(7, &[]);
    s[27] = 255;
    e.section(&s, fixed_now());
    assert!(e.entries.is_empty());
}
#[test]
fn same_and_other_network_relays() {
    let mut e = Events::default();
    let msg = e.section(&eit(7, &[0xd6, 5, 0x21, 0, 102, 0, 8]), fixed_now());
    assert!(msg[0].contains("\"RelaySid\":102"));
    assert!(
        e.section(&eit(7, &[0xd6, 5, 0x21, 0, 102, 0, 8]), fixed_now())
            .is_empty()
    );
    let msg = e.section(
        &eit(7, &[0xd6, 9, 0x40, 0, 5, 0, 6, 0, 103, 0, 9]),
        fixed_now(),
    );
    assert!(msg[0].contains("\"RelayOnid\":5"));
}
#[test]
fn sdt_nit_version_and_crc() {
    let mut e = Engine::default();
    let s = sdt();
    let m = e.process(&packet(0x11, &s, 0, true, true, false));
    assert!(m[0].contains("\"OneSeg\":-1"));
    assert!(m[0].contains("\"ServiceName\":\"Ａ\""));
    let m = e.process(&packet(0x10, &nit(101, 1, 0, 0), 0, true, true, false));
    assert!(m[0].contains("\"OneSeg\":1"));
    assert!(
        e.process(&packet(0x10, &nit(101, 1, 0, 0), 1, true, true, false))
            .is_empty()
    );
    let m = e.process(&packet(0x10, &nit(102, 2, 0, 1), 2, true, true, false));
    assert!(m[0].contains("\"OneSeg\":-1"));
    let m = e.process(&packet(0x10, &nit(102, 2, 1, 1), 3, true, true, false));
    assert!(m[0].contains("\"OneSeg\":0"));
    let mut bad = s;
    bad[4] ^= 1;
    assert!(
        e.process(&packet(0x11, &bad, 1, true, true, false))
            .is_empty()
    );
}
#[test]
fn pat_pmt_caption_and_split_pes() {
    let mut e = Engine::default();
    let pat = section(vec![0, 0, 0, 0x12, 0x34, 0xc1, 0, 0, 0, 101, 0xe1, 0], true);
    e.process(&packet(0, &pat, 0, true, true, false));
    let pmt = section(
        vec![
            2, 0, 0, 0, 101, 0xc1, 0, 0, 0xe1, 0, 0xf0, 0, 6, 0xe1, 0x50, 0xf0, 6, 0xfd, 4, 0, 8,
            0, 0,
        ],
        true,
    );
    e.process(&packet(0x100, &pmt, 0, true, true, false));
    let cap = [
        0, 0, 1, 0xbd, 0, 0x26, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1, 0x80, 255, 0xf0, 4, 0, 0, 0, 0x14,
        0, 0, 0, 0x10, 0x1f, 0x20, 0, 0, 0x0b, 0x1c, 0x45, 0x4a, 0x81, 0x89, 0x0e, b'H', b'e',
        b'l', b'l', b'o', 0, 0,
    ];
    let m = e.process(&packet(0x150, &cap, 0, true, false, false));
    assert_eq!(m.len(), 1);
    assert!(m[0].contains("\"Text\":\"Hello\""));
    assert!(m[0].contains("\"Color\":1"));
    assert!(
        e.process(&packet(0x150, &cap[..20], 1, true, false, true))
            .is_empty()
    );
    e.process(&packet(0x100, &pmt, 1, true, true, false));
    let m = e.process(&packet(0x150, &cap[20..], 2, false, false, true));
    assert_eq!(m.len(), 1);
    e.service = 999;
    assert!(
        e.process(&packet(0x150, &cap, 3, true, false, false))
            .is_empty()
    );
    e.service = 101;
    let mut secondary = cap;
    secondary[17] = 8; // second language statement
    assert!(
        e.process(&packet(0x150, &secondary, 4, true, false, false))
            .is_empty()
    );
}
#[test]
fn malformed_ts_and_sections_do_not_panic() {
    let mut rng = 12345u32;
    let mut e = Engine::default();
    for run in 0..10000 {
        let mut p = [0; 188];
        for b in &mut p {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (rng >> 16) as u8
        }
        p[0] = 0x47;
        p[1] = (p[1] & 0xe0) | 0x40;
        p[2] = [0, 0x10, 0x11, 0x12, 0x14, 0x27][run % 6];
        e.process(&p);
        let mut s = eit(7, &[]);
        for _ in 0..3 {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let at = 3 + rng as usize % (s.len() - 7);
            s[at] = (rng >> 16) as u8;
        }
        s.truncate(s.len() - 4);
        let s = section(s, true);
        e.events.section(&s, fixed_now());
    }
}

#[test]
fn packet_counter_preserves_anomaly_semantics() {
    let mut c = ts::Counter::default();
    let p = packet(100, &[1, 2], 15, false, false, false);
    c.packet(&p);
    c.packet(&p);
    assert_eq!(c.drop, 0);
    c.packet(&p);
    c.packet(&p);
    assert_eq!(c.drop, 2);
    c.packet(&packet(100, &[3], 0, false, false, false));
    assert_eq!(c.drop, 2);
    c.packet(&packet(100, &[4], 4, false, false, false));
    assert_eq!(c.drop, 3);
}

#[test]
fn extended_description_updates_are_resent_and_merge_with_schedule() {
    let mut e = Events::default();
    let make = |letter| {
        eit(
            7,
            &[
                0x54, 2, 0x12, 0, 0x4e, 11, 0, b'j', b'p', b'n', 5, 0, 3, 0x0e, 0x89, letter, 0,
            ],
        )
    };
    e.section(&make(b'A'), fixed_now());
    assert!(e.pending(|_| true).unwrap().contains("\"Content\":\"A\""));
    e.section(&make(b'B'), fixed_now());
    assert!(e.pending(|_| true).unwrap().contains("\"Content\":\"B\""));
    let mut schedule = eit(7, &[]);
    schedule[0] = 0x50;
    e.section(&schedule, fixed_now());
    assert!(e.pending(|_| true).unwrap().contains("\"Content\":\"B\""));
}

#[test]
fn dates_beyond_2038_stay_positive_in_json() {
    let mut s = eit(7, &[0x54, 2, 0, 0]);
    s[16..21].copy_from_slice(&[255, 255, 0x12, 0, 0]);
    let now = time::timestamp(&s[16..21]).unwrap();
    assert!(now > i32::MAX as i64);
    let mut e = Events::default();
    e.section(&s, now);
    assert!(
        e.pending(|_| true)
            .unwrap()
            .contains(&format!("\"Start\":{now}"))
    );
}

#[test]
fn engine_dispatches_eit_on_both_broadcast_pids() {
    let now = time::now() + 9 * 3600;
    let mjd = (now / 86400 + 40587) as u16;
    let bcd = |v: i64| ((v / 10) * 16 + v % 10) as u8;
    let mut s = eit(7, &[0x54, 2, 0, 0]);
    s[16..18].copy_from_slice(&mjd.to_be_bytes());
    s[18..21].copy_from_slice(&[bcd(now / 3600 % 24), bcd(now / 60 % 60), bcd(now % 60)]);
    s.truncate(s.len() - 4);
    let s = section(s, true);
    for pid in [0x12, 0x27] {
        let mut e = Engine::default();
        e.process(&packet(pid, &s, 0, true, true, false));
        assert_eq!(e.events.entries.len(), 1);
        assert!(e.events.pending(|_| true).is_some());
    }
}

#[test]
fn rejected_eit_batch_is_retried_and_reconnect_resends() {
    let mut e = Events::default();
    e.section(&eit(7, &[0x54, 2, 0, 0]), fixed_now());
    let mut rejected = String::new();
    e.send_pending(
        |_| true,
        |message| {
            rejected = message.into();
            false
        },
    );
    assert!(!rejected.is_empty());
    assert_eq!(e.pending(|_| true).unwrap(), rejected);
    assert!(e.pending(|_| true).is_none());
    e.resend();
    assert_eq!(e.pending(|_| true).unwrap(), rejected);
}

#[test]
fn five_minute_event_without_genre_is_sent_and_later_updated() {
    let mut events = Events::default();
    let mut s = eit(7, &[]);
    s[21..24].copy_from_slice(&[0, 5, 0]);
    events.section(&s, fixed_now());
    let sent = events
        .pending(|_| true)
        .expect("short program without genre must be sent");
    let data: serde_json::Value = serde_json::from_str(&sent[4..]).unwrap();
    assert_eq!(
        data[0]["End"].as_i64().unwrap() - data[0]["Start"].as_i64().unwrap(),
        300
    );
    assert_eq!(data[0]["Cnib_a"], 15);
    assert_eq!(data[0]["Cnib_b"], 15);
    assert!(events.pending(|_| true).is_none());
    let mut s = eit(7, &[0x54, 2, 0x12, 0]);
    s[21..24].copy_from_slice(&[0, 5, 0]);
    events.section(&s, fixed_now());
    assert!(events.pending(|_| true).unwrap().contains("\"Cnib_a\":1"));
}
