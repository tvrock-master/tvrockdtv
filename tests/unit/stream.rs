use crate::{
    stream::{Status, Streamer},
    stream_filter::Filter,
    ts::{Receiver, be16, crc32},
};
use std::{
    net::{Ipv4Addr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::SeqCst},
    },
    thread,
    time::{Duration, Instant},
};
fn section(pid: u16, mut data: Vec<u8>, cc: u8) -> Vec<[u8; 188]> {
    data.extend_from_slice(&crc32(&data).to_be_bytes());
    let mut packets = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let first = pos == 0;
        let mut p = [0xff; 188];
        p[..4].copy_from_slice(&[
            0x47,
            ((pid >> 8) as u8) | if first { 0x40 } else { 0 },
            pid as u8,
            0x10 | ((cc + packets.len() as u8) & 15),
        ]);
        let offset = if first {
            p[4] = 0;
            5
        } else {
            4
        };
        let n = (188 - offset).min(data.len() - pos);
        p[offset..offset + n].copy_from_slice(&data[pos..pos + n]);
        pos += n;
        packets.push(p);
    }
    packets
}
fn tables(one_seg: bool) -> Vec<[u8; 188]> {
    let second = if one_seg { 0x1fc8 } else { 0x200 };
    let mut all = section(
        0,
        vec![
            0,
            0xb0,
            17,
            0,
            1,
            0xc1,
            0,
            0,
            0,
            1,
            0xe1,
            0,
            0,
            2,
            0xe0 | ((second >> 8) as u8),
            second as u8,
        ],
        0,
    );
    for (service, pid) in [(1, 0x100), (2, second)] {
        let base = service << 8;
        let mut pmt = vec![
            2,
            0xb0,
            0,
            0,
            service as u8,
            0xc1,
            0,
            0,
            0xe0 | ((base >> 8) as u8),
            1,
            0xf0,
            0,
        ];
        for (typ, offset) in [(0x1b, 1), (0xf, 2), (6, 3), (0xd, 4)] {
            pmt.extend_from_slice(&[typ, 0xe0 | ((base >> 8) as u8), offset, 0xf0, 0]);
        }
        pmt[2] = (pmt.len() + 1) as u8;
        all.extend(section(pid, pmt, 0));
    }
    all
}
fn packet(pid: u16) -> [u8; 188] {
    let mut p = [0x55; 188];
    p[..4].copy_from_slice(&[0x47, (pid >> 8) as u8, pid as u8, 0x10]);
    p
}
#[test]
fn service_option_matrix_and_pat_crc() {
    for flags in [7, 1, 0, 6, 9, 15] {
        let mut f = Filter::new(1, flags);
        let mut rewritten = None;
        for p in tables(true) {
            if let Some(out) = f.packet(&p)
                && out[1] & 31 == 0
                && out[2] == 0
            {
                rewritten = Some(out);
            }
        }
        let selected = if flags & 8 != 0 { 2 } else { 1 };
        assert_eq!(f.selected(), selected);
        for service in [1, 2] {
            for offset in 1..=4 {
                let included = (flags & 1 == 0 && flags & 8 == 0 || service == selected)
                    && (offset <= 2
                        || offset == 3 && flags & 2 != 0
                        || offset == 4 && flags & 4 != 0);
                assert_eq!(
                    f.packet(&packet((service << 8) | offset)).is_some(),
                    included,
                    "flags {flags}, service {service}, ES {offset}"
                );
            }
        }
        if flags & 1 != 0 {
            let p = rewritten.unwrap();
            let mut r = Receiver::default();
            let sections = r.sections(&p);
            assert_eq!(sections.len(), 1);
            let s = &sections[0];
            assert_eq!(crc32(s), 0);
            assert_eq!(be16(&s[12..]), selected);
            assert_eq!(s.len(), 20);
        }
    }
}
#[test]
fn unknown_service_corrupt_pat_and_partial_tables_do_not_leak() {
    let mut f = Filter::new(77, 7);
    for p in tables(false) {
        f.packet(&p);
    }
    assert!(f.packet(&packet(0x101)).is_none());
    let mut f = Filter::new(1, 0);
    assert!(f.packet(&packet(0x101)).is_none());
    let mut p = tables(false)[0];
    p[11] ^= 1;
    f.packet(&p);
    assert!(f.packet(&packet(0x101)).is_none());
    let mut f = Filter::new(1, 9);
    for p in tables(false) {
        f.packet(&p);
    }
    assert_eq!(f.selected(), 0);
    assert!(f.packet(&packet(0x101)).is_none());
    // Multi-section PAT is published atomically, even when the selected service
    // arrives in a later section. Invalid input cannot advertise an unrelated ES.
    let mut f = Filter::new(2, 7);
    let first = section(0, vec![0, 0xb0, 13, 0, 1, 0xc1, 0, 1, 0, 1, 0xe1, 0], 0);
    let last = section(0, vec![0, 0xb0, 13, 0, 1, 0xc1, 1, 1, 0, 2, 0xe2, 0], 1);
    for p in first {
        assert!(f.packet(&p).is_none());
    }
    assert!(f.packet(&packet(0x201)).is_none());
    for p in last {
        assert!(f.packet(&p).is_some());
    }
}
fn receiver() -> UdpSocket {
    let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    s
}
fn token(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}
fn receive(s: &UdpSocket) -> Vec<u8> {
    let mut p = vec![0; 65536];
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match s.recv(&mut p) {
            Ok(n) => {
                p.truncate(n);
                return p;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) && Instant::now() < deadline =>
            {
                continue;
            }
            Err(e) => panic!("UDP receive did not complete: {e}"),
        }
    }
}
fn assert_sent_bytes(sender: &Streamer, token: &str, expected: u64) {
    // Receiving the datagram can precede the worker's post-send status update.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let status = sender.status(token).unwrap();
        if status.bytes >= expected || Instant::now() >= deadline {
            assert_eq!(status.bytes, expected);
            assert_eq!(status.error, 0);
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn udp_peers_options_reset_stop_and_invalid_destinations() {
    let a = receiver();
    let b = receiver();
    let sender = Streamer::default();
    sender.reset(1);
    let ta = token('a');
    let tb = token('b');
    for address in [
        "example.invalid",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "127.0.0.1 & cmd",
    ] {
        assert!(!sender.start(&ta, address, 1234, 7));
    }
    assert!(!sender.start("bad", "127.0.0.1", 1234, 7));
    assert!(!sender.start(&ta, "127.0.0.1", 0, 7));
    assert!(!sender.start(&ta, "127.0.0.1", 1234, 16));
    assert!(sender.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 7));
    assert!(sender.start(&tb, "127.0.0.1", b.local_addr().unwrap().port(), 0));
    let duplicate = Streamer::default();
    assert!(!duplicate.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 7));
    sender.feed(&packet(0x12));
    assert_eq!(receive(&a).len(), 188);
    assert_eq!(receive(&b).len(), 188);
    assert!(sender.stop(&ta));
    assert!(sender.stop(&token('c')));
    sender.reset(0);
    sender.feed(&packet(0x12));
    thread::sleep(Duration::from_millis(25));
    assert!(b.recv(&mut [0; 188]).is_err());
    sender.reset(2);
    sender.feed(&packet(0x12));
    assert_eq!(receive(&b).len(), 188);
    assert_eq!(sender.status(&tb).unwrap().service, 2);
    assert_eq!(sender.status(&tb).unwrap().flags, 0);
    assert_eq!(sender.status(&ta).unwrap().enabled, 0);
    assert!(a.recv(&mut [0; 188]).is_err());
    assert!(sender.stop(&tb));
    assert_eq!(std::mem::size_of::<Status>(), 56);
}
#[test]
fn oneseg_missing_is_an_explicit_error() {
    let receiver = receiver();
    let sender = Streamer::default();
    sender.reset(1);
    let token = token('e');
    assert!(sender.start(
        &token,
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        8
    ));
    for p in tables(false) {
        sender.feed(&p);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while sender.status(&token).unwrap().error != 1168 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let status = sender.status(&token).unwrap();
    assert_eq!(status.error, 1168);
    assert_eq!(status.service, 0);
    assert_eq!(status.flags, 9);
    assert!(receiver.recv(&mut [0; 188]).is_err());
}
#[test]
fn realtime_udp_coalesces_small_callbacks_without_loss() {
    let receiver = receiver();
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        #[link(name = "ws2_32")]
        unsafe extern "system" {
            fn setsockopt(s: usize, l: i32, n: i32, p: *const u8, len: i32) -> i32;
        }
        let bytes: i32 = 4 * 1024 * 1024;
        assert_eq!(
            unsafe {
                setsockopt(
                    receiver.as_raw_socket() as usize,
                    0xffff,
                    0x1002,
                    (&bytes as *const i32).cast(),
                    4,
                )
            },
            0
        );
    }
    let sender = Streamer::default();
    sender.reset(1);
    let token = token('f');
    assert!(sender.start(
        &token,
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        6
    ));
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let count = 160000u32;
    let reader = thread::spawn(move || {
        let mut data = [0; 65536];
        let mut expected = 0;
        let mut maximum = 0;
        let mut small = 0;
        while !flag.load(SeqCst) {
            match receiver.recv(&mut data) {
                Ok(n) => {
                    assert!(n <= 24064 && n % 188 == 0);
                    maximum = maximum.max(n);
                    if n < 32 * 188 {
                        small += 1
                    }
                    for p in data[..n].as_chunks::<188>().0 {
                        assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), expected);
                        expected += 1;
                    }
                    if expected == count {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
        (expected, maximum, small)
    });
    let start = Instant::now();
    let mut p = packet(0x12);
    for sequence in 0..count {
        p[4..8].copy_from_slice(&sequence.to_le_bytes());
        assert!(sender.feed(&p));
        if sequence % 532 == 531 {
            let due = start + Duration::from_micros((u64::from(sequence) + 1) * 188 * 8 / 80);
            let now = Instant::now();
            if due > now {
                thread::sleep(due - now);
            }
        }
    }
    thread::sleep(Duration::from_millis(250));
    done.store(true, SeqCst);
    let (received, maximum, _) = reader.join().unwrap();
    assert_eq!(received, count);
    assert!(maximum >= 32 * 188);
    let status = sender.status(&token).unwrap();
    assert_eq!(status.bytes, u64::from(count) * 188);
    assert_eq!(status.error, 0);
    assert!(status.last_tick > 0);
    assert!(start.elapsed() < Duration::from_secs(5));
    println!("Rust UDP: 80 Mbit/s, {received} TS packets, zero loss, max payload {maximum}");
}

#[test]
fn partial_input_overflow_recovery_and_recorder_port_sharing() {
    let a = receiver();
    let b = receiver();
    let sender = Streamer::new(true);
    sender.reset(1);
    let ta = token('a');
    let tb = token('b');
    assert!(sender.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 6));
    let p = packet(0x12);
    assert!(sender.feed(&p[..17]));
    assert!(sender.feed(&p[17..]));
    assert_eq!(receive(&a), p);
    assert!(!sender.feed(&vec![0; 8 * 1024 * 1024 + 1]));
    assert_eq!(sender.status(&ta).unwrap().error, 10055);
    assert!(sender.feed(&p));
    assert_eq!(receive(&a), p);
    // Different peers can use one UDP port; RecTest's standard sender sees
    // one port reservation while each remote peer retains its own endpoint.
    let port = a.local_addr().unwrap().port();
    let second = UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 2), port)).unwrap();
    second
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert!(sender.start(&tb, "127.0.0.2", port, 6));
    assert!(sender.feed(&p));
    assert_eq!(receive(&a), p);
    assert_eq!(receive(&second), p);
    assert!(sender.stop(&ta));
    assert!(sender.feed(&p));
    assert_eq!(receive(&second), p);
    assert!(sender.stop(&tb));
    let different = Streamer::new(true);
    different.reset(1);
    assert!(different.start(&ta, "127.0.0.1", port, 6));
    assert!(different.start(&tb, "127.0.0.1", b.local_addr().unwrap().port(), 6));
}

#[test]
fn repeated_service_notifications_keep_the_pmt_selection() {
    let receiver = receiver();
    let sender = Streamer::default();
    sender.reset(1);
    let token = token('d');
    assert!(sender.start(
        &token,
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        7
    ));
    for p in tables(false) {
        sender.feed(&p);
    }
    thread::sleep(Duration::from_millis(30));
    while receiver.recv(&mut [0; 65536]).is_ok() {}
    // PMT/SDT notifications can report the same service repeatedly. They must
    // retain learned PIDs and pending data, while an explicit reset invalidates it.
    sender.set_service(1);
    let p = packet(0x101);
    sender.feed(&p);
    assert_eq!(receive(&receiver), p);
    sender.reset(0);
    sender.set_service(1);
    sender.feed(&p);
    assert!(receiver.recv(&mut [0; 65536]).is_err());
}

#[test]
fn pacing_is_per_session_and_does_not_restart_or_reorder() {
    let a = receiver();
    let b = receiver();
    let sender = Streamer::default();
    sender.reset(1);
    let ta = token('c');
    let tb = token('d');
    assert!(!sender.pace(&ta));
    assert!(sender.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 6));
    assert!(sender.start(&tb, "127.0.0.1", b.local_addr().unwrap().port(), 6));
    let mut data = Vec::new();
    for n in 0..128u32 {
        let mut p = packet(0x12);
        p[4..8].copy_from_slice(&n.to_le_bytes());
        data.extend_from_slice(&p)
    }
    assert!(sender.feed(&data));
    assert_eq!(receive(&a), data);
    assert_eq!(receive(&b), data);
    let before = data.len() as u64;
    assert_sent_bytes(&sender, &ta, before);
    assert!(sender.pace(&ta));
    assert!(sender.pace(&ta));
    assert!(!sender.is_paced(&tb));
    assert!(sender.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 6));
    assert!(sender.is_paced(&ta));
    assert!(sender.feed(&data));
    let mut received = Vec::new();
    while received.len() < data.len() {
        let d = receive(&a);
        assert!(d.len() <= 1316);
        received.extend(d)
    }
    assert_eq!(received, data);
    assert_eq!(receive(&b), data);
    assert_sent_bytes(&sender, &ta, before + data.len() as u64);
    sender.reset(2);
    assert!(sender.is_paced(&ta));
    assert!(sender.stop(&ta));
    assert!(!sender.pace(&ta));
    assert!(sender.start(&ta, "127.0.0.1", a.local_addr().unwrap().port(), 6));
    assert!(!sender.is_paced(&ta));
    assert!(!sender.is_paced(&tb));
}

#[test]
fn paced_udp_tracks_large_bitrate_changes_without_loss_or_latency_growth() {
    let receiver = receiver();
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        #[link(name = "ws2_32")]
        unsafe extern "system" {
            fn setsockopt(s: usize, l: i32, n: i32, p: *const u8, len: i32) -> i32;
        }
        let bytes: i32 = 4 * 1024 * 1024;
        assert_eq!(
            unsafe {
                setsockopt(
                    receiver.as_raw_socket() as usize,
                    0xffff,
                    0x1002,
                    (&bytes as *const i32).cast(),
                    4,
                )
            },
            0
        );
    }

    let sender = Streamer::default();
    sender.reset(1);
    let token = token('e');
    assert!(sender.start(
        &token,
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        6
    ));
    assert!(sender.pace(&token));
    // Each phase is one second: higher-rate content is measured at the input,
    // not capped to the rate previously observed by the sending worker.
    let rates = [40u64, 240, 80];
    let counts = rates.map(|r| (r * 1_000_000 / (188 * 8)) as u32);
    let total: u32 = counts.iter().sum();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let reader = thread::spawn(move || {
        let mut data = [0; 65536];
        let mut expected = 0;
        while !flag.load(SeqCst) {
            if let Ok(n) = receiver.recv(&mut data) {
                assert!(n <= 1316 && n % 188 == 0);
                for p in data[..n].as_chunks::<188>().0 {
                    assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), expected);
                    expected += 1
                }
                if expected == total {
                    break;
                }
            }
        }
        expected
    });
    let start = Instant::now();
    let mut sequence: u32 = 0;
    let mut p = packet(0x12);
    for (phase, rate) in rates.iter().enumerate() {
        let phase_start = Instant::now();
        for n in 0..counts[phase] {
            p[4..8].copy_from_slice(&sequence.to_le_bytes());
            sequence += 1;
            assert!(sender.feed(&p));
            if n % 256 == 255 {
                let due =
                    phase_start + Duration::from_nanos((u64::from(n) + 1) * 188 * 8 * 1000 / rate);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    thread::sleep(wait)
                }
            }
        }
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while sender.status(&token).unwrap().bytes < u64::from(total) * 188 && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(2))
    }
    thread::sleep(Duration::from_millis(100));
    done.store(true, SeqCst);
    assert_eq!(reader.join().unwrap(), total);
    assert_eq!(sender.status(&token).unwrap().error, 0);
    assert!(start.elapsed() < Duration::from_secs(5));
}
