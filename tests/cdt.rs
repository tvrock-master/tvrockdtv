use tvrockdtv::{engine::Engine, ts};

fn section(mut s: Vec<u8>) -> Vec<u8> {
    let length = s.len() + 1;
    s[1] = 0xb0 | (length >> 8) as u8;
    s[2] = length as u8;
    s.extend_from_slice(&ts::crc32(&s).to_be_bytes());
    s
}
fn sdt() -> Vec<u8> {
    section(vec![
        0x42, 0, 0, 0, 1, 0xc1, 0, 0, 0, 4, 0xff, 0, 101, 0xff, 0x80, 9, 0xcf, 7, 1, 0xfe, 3, 0xf0,
        2, 0, 9,
    ])
}
fn cdt(kind: u8) -> Vec<u8> {
    let mut s = vec![
        0xc8, 0, 0, 0, 9, 0xc1, 0, 0, 0, 4, 1, 0xf0, 0, kind, 0xfe, 3, 0xf0, 2, 1, 0,
    ];
    s.extend((0..256).map(|v| v as u8));
    section(s)
}
fn feed(e: &mut Engine, pid: u16, s: &[u8], cc: &mut u8) {
    let mut data = vec![0];
    data.extend(s);
    for (i, b) in data.chunks(184).enumerate() {
        let mut p = [255; 188];
        p[..4].copy_from_slice(&[
            0x47,
            (pid >> 8) as u8 | if i == 0 { 64 } else { 0 },
            pid as u8,
            0x10 | *cc,
        ]);
        *cc = (*cc + 1) & 15;
        p[4..4 + b.len()].copy_from_slice(b);
        e.packet(&p);
    }
}
#[test]
fn match_filter_retry_deduplicate_and_retune() {
    let mut e = Engine::default();
    let mut cc = 0;
    feed(&mut e, 0x29, &cdt(0), &mut cc);
    e.logos.send_pending(|_| true, |_| panic!("no SDT yet"));
    feed(&mut e, 0x11, &sdt(), &mut cc);
    e.logos
        .send_pending(|_| false, |_| panic!("unrequested service"));
    let mut tries = 0;
    e.logos.send_pending(
        |s| s == 101,
        |_| {
            tries += 1;
            false
        },
    );
    assert_eq!(tries, 1);
    e.logos.send_pending(
        |_| true,
        |msg| {
            let v: serde_json::Value = serde_json::from_str(&msg[4..]).unwrap();
            assert_eq!(v["Sid"], 101);
            assert_eq!(v["Data"].as_str().unwrap().len(), 512);
            tries += 1;
            true
        },
    );
    e.logos.send_pending(|_| true, |_| panic!("duplicate"));
    e.reset(1, 101, 0);
    feed(&mut e, 0x11, &sdt(), &mut cc);
    feed(&mut e, 0x29, &cdt(0), &mut cc);
    e.logos
        .send_pending(|_| true, |_| panic!("duplicate after retune"));
    feed(&mut e, 0x29, &cdt(5), &mut cc);
    e.logos.send_pending(
        |_| true,
        |_| {
            tries += 1;
            true
        },
    );
    assert_eq!(tries, 3);
}
#[test]
fn bad_crc_wrong_download_and_truncation_are_rejected() {
    let mut e = Engine::default();
    let mut cc = 0;
    feed(&mut e, 0x11, &sdt(), &mut cc);
    let mut bad = cdt(0);
    bad[30] ^= 1;
    feed(&mut e, 0x29, &bad, &mut cc);
    let mut wrong = cdt(0);
    wrong.truncate(wrong.len() - 4);
    wrong[4] = 10;
    feed(&mut e, 0x29, &section(wrong), &mut cc);
    for n in 0..24 {
        e.logos.cdt(&bad[..n]);
        e.logos.sdt(&bad[..n]);
    }
    e.logos.send_pending(|_| true, |_| panic!("invalid CDT"));
}
