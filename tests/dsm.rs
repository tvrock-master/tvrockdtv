use tvrockdtv::{engine::Engine, ts};
fn section(mut s: Vec<u8>) -> Vec<u8> {
    let len = s.len() + 1;
    s[1] = 0xb0 | ((len >> 8) as u8);
    s[2] = len as u8;
    let crc = ts::crc32(&s);
    s.extend(crc.to_be_bytes());
    s
}
fn packets(pid: u16, s: &[u8], cc: &mut u8) -> Vec<[u8; 188]> {
    let mut bytes = vec![0];
    bytes.extend(s);
    bytes
        .chunks(184)
        .enumerate()
        .map(|(i, b)| {
            let mut p = [255; 188];
            p[0] = 0x47;
            p[1] = (pid >> 8) as u8 | if i == 0 { 64 } else { 0 };
            p[2] = pid as u8;
            p[3] = 0x10 | *cc;
            *cc = (*cc + 1) & 15;
            p[4..4 + b.len()].copy_from_slice(b);
            p
        })
        .collect()
}
fn feed(e: &mut Engine, pid: u16, s: &[u8], cc: &mut u8) -> Vec<String> {
    packets(pid, s, cc)
        .iter()
        .flat_map(|p| e.packet(p))
        .collect()
}
fn setup(e: &mut Engine) {
    feed(
        e,
        0,
        &section(vec![0, 0, 0, 0, 1, 0xc1, 0, 0, 0, 101, 0xe1, 0]),
        &mut 0,
    );
    // PMT: PCR 0x101, DSM PID 0x120, stream_identifier and data_component descriptors.
    feed(
        e,
        256,
        &section(vec![
            2, 0, 0, 0, 101, 0xc1, 0, 0, 0xe1, 1, 0xf0, 0, 0x0d, 0xe1, 0x20, 0xf0, 11, 0x52, 1,
            0x40, 0xfd, 6, 0, 7, 0x33, 0x40, 0xf8, 0x80,
        ]),
        &mut 0,
    );
}
#[test]
fn extracts_fragmented_dsm_and_preserves_metadata() {
    let mut e = Engine::default();
    setup(&mut e);
    let mut s = vec![0x3c, 0, 0, 0, 0, 0xc0, 254, 0, 0x11, 3, 0x10, 3];
    s.resize(800, 0);
    let s = section(s);
    // DDB block/last-section/current bits do not follow ordinary PSI restrictions.
    let out = feed(&mut e, 0x120, &s, &mut 0);
    assert_eq!(out.len(), 1);
    let v: serde_json::Value = serde_json::from_str(out[0].strip_prefix("DSM:").unwrap()).unwrap();
    assert_eq!(v["Sid"], 101);
    assert_eq!(v["Components"][0]["ComponentTag"], 64);
    assert_eq!(v["Data"], tvrockdtv::dsm::hex(&s));
    let mut bad = s.clone();
    bad[25] ^= 1;
    assert!(feed(&mut e, 0x120, &bad, &mut 5).is_empty());
    e.reset(0, 0, 0);
    assert!(feed(&mut e, 0x120, &s, &mut 10).is_empty());
}
#[test]
fn missing_packet_and_removed_pmt_do_not_leak_sections() {
    let mut e = Engine::default();
    setup(&mut e);
    let mut s = vec![0x3b, 0, 0, 0, 0, 0xc1, 0, 0];
    s.resize(700, 0);
    let s = section(s);
    let ps = packets(0x120, &s, &mut 0);
    assert!(e.packet(&ps[0]).is_empty());
    assert!(e.packet(&ps[2]).is_empty());
    assert!(e.packet(&ps[3]).is_empty());
    feed(
        &mut e,
        256,
        &section(vec![2, 0, 0, 0, 101, 0xc3, 0, 0, 0xe1, 1, 0xf0, 0]),
        &mut 1,
    );
    assert!(feed(&mut e, 0x120, &s, &mut 4).is_empty());
}
#[test]
fn pcr_is_selected_service_only_and_rate_limited() {
    let mut e = Engine::default();
    setup(&mut e);
    e.service = 101;
    let mut p = [0; 188];
    p[0] = 0x47;
    p[1] = 1;
    p[2] = 1;
    p[3] = 0x20;
    p[4] = 183;
    p[5] = 16;
    assert_eq!(e.packet(&p).len(), 1);
    assert!(e.packet(&p).is_empty());
    p[8] = 0x20;
    assert_eq!(e.packet(&p).len(), 1);
    e.service = 102;
    p[8] = 0x40;
    assert!(e.packet(&p).is_empty());
}
