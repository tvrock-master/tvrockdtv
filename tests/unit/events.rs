use super::*;

fn text(value: &[u8]) -> Vec<u8> {
    // Select small alphanumeric characters, just as the EIT decoder supports.
    [vec![0x0e, 0x89], value.to_vec()].concat()
}

fn descriptor(number: u8, last: u8, items: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
    let mut data = vec![(number << 4) | last, b'j', b'p', b'n', 0];
    for (name, body) in items {
        data.push(name.len() as u8);
        data.extend_from_slice(name);
        data.push(body.len() as u8);
        data.extend_from_slice(body);
    }
    data[4] = (data.len() - 5) as u8;
    data.push(0);
    assert!(valid_extended(&data));
    data
}

#[test]
fn extended_items_have_a_blank_line_between_story_and_cast() {
    let ds = BTreeMap::from([(
        0,
        descriptor(
            0,
            0,
            &[
                (b"Story", text(b"Story!?")),
                (b"Cast", text(b"Actor A\rActor B")),
            ],
        ),
    )]);
    let content = extended_text(&ds);
    assert_eq!(content, "Story!?\r\n\r\nActor A\r\nActor B");
    let event = Event {
        content,
        ..Event::default()
    };
    assert!(
        event
            .json(true)
            .contains(&quote("Story!?\r\n\r\nActor A\r\nActor B"))
    );
}

#[test]
fn extended_item_continuations_keep_decoder_state_across_descriptors() {
    let ds = BTreeMap::from([
        (0, descriptor(0, 1, &[(b"Story", text(b"Part1"))])),
        (
            1,
            descriptor(1, 1, &[(b"", b"Part2".to_vec()), (b"Cast", text(b"Actor"))]),
        ),
    ]);
    assert_eq!(extended_text(&ds), "Part1Part2\r\n\r\nActor");
}

#[test]
fn extended_item_boundaries_preserve_existing_line_breaks() {
    for (story, cast, expected) in [
        (&b"Story\r"[..], &b"Actor"[..], "Story\r\n\r\nActor"),
        (&b"Story"[..], &b"\rActor"[..], "Story\r\n\r\nActor"),
        (&b"Story\r\r"[..], &b"Actor"[..], "Story\r\n\r\nActor"),
        (&b"Story"[..], &b"\r\rActor"[..], "Story\r\n\r\nActor"),
        (&b"Story\r"[..], &b"\rActor"[..], "Story\r\n\r\nActor"),
        (&b"Story\r\r\r"[..], &b"Actor"[..], "Story\r\n\r\n\r\nActor"),
    ] {
        let ds = BTreeMap::from([(
            0,
            descriptor(0, 0, &[(b"Story", text(story)), (b"Cast", text(cast))]),
        )]);
        assert_eq!(extended_text(&ds), expected);
    }
}

#[test]
fn empty_extended_items_do_not_add_extra_blank_lines() {
    let ds = BTreeMap::from([(
        0,
        descriptor(
            0,
            0,
            &[
                (b"Empty", text(b"")),
                (b"Story", text(b"Story")),
                (b"Empty", text(b"")),
                (b"Cast", text(b"Actor")),
                (b"Empty", text(b"")),
            ],
        ),
    )]);
    assert_eq!(extended_text(&ds), "Story\r\n\r\nActor");
    assert_eq!(extended_text(&BTreeMap::new()), "");
}

fn sharing_eit(table: u8, version: u8, data: &[u8]) -> Vec<u8> {
    let mut section = vec![
        table,
        0xb0,
        0,
        0,
        142,
        0xc1 | (version << 1),
        0,
        0,
        0x40,
        0xd0,
        0,
        4,
        0,
        table,
    ];
    section.extend_from_slice(&[
        0,
        20,
        0xff,
        0xff,
        0xff,
        0xff,
        0xff,
        0,
        30,
        0,
        0xf0,
        (data.len() + 2) as u8,
    ]);
    section.extend_from_slice(&[0xd6, data.len() as u8]);
    section.extend_from_slice(data);
    section.extend_from_slice(&[0; 4]);
    section
}

#[test]
fn common_event_without_short_descriptor_is_sent_with_child_identity() {
    let mut events = Events::default();
    assert!(
        events
            .section(&sharing_eit(0x50, 0, &[0x11, 0, 141, 0, 10]), 0)
            .is_empty()
    );
    let text = events
        .pending(|sid| sid == 142)
        .expect("sharing-only event must be sent");
    let json: serde_json::Value = serde_json::from_str(&text[4..]).unwrap();
    assert_eq!(json[0]["Sid"], 142);
    assert_eq!(json[0]["Eid"], 20);
    assert_eq!(json[0]["CommonSid"], 141);
    assert_eq!(json[0]["CommonEid"], 10);
    assert_eq!(json[0]["Title"], "");
    events.resend();
    assert!(events.pending(|_| true).unwrap().contains("CommonSid"));
}

#[test]
fn common_event_is_also_available_in_present_following() {
    let mut events = Events::default();
    events.section(&sharing_eit(0x4e, 0, &[0x11, 0, 141, 0, 10]), 0);
    let text = events
        .current(142)
        .expect("sharing-only p/f must be available");
    let json: serde_json::Value = serde_json::from_str(&text[4..]).unwrap();
    assert_eq!(json[0]["Sid"], 142);
    assert_eq!(json[0]["CommonEid"], 10);
    assert!(json[1]["CommonSid"].is_null());
}

#[test]
fn common_event_rejects_truncated_multiple_and_self_references() {
    for data in [
        &[0x11, 0, 141, 0][..],
        &[0x12, 0, 141, 0, 10, 0, 143, 0, 11],
        &[0x11, 0, 142, 0, 20],
    ] {
        let mut events = Events::default();
        events.section(&sharing_eit(0x50, 0, data), 0);
        assert!(events.entries.values().all(|event| event.common.is_none()));
        assert!(events.pending(|_| true).is_none());
    }
}

#[test]
fn complementary_extended_table_preserves_sharing_but_new_basic_clears_it() {
    let mut events = Events::default();
    events.section(&sharing_eit(0x50, 0, &[0x11, 0, 141, 0, 10]), 0);
    events.section(&sharing_eit(0x58, 0, &[0x20]), 0);
    assert_eq!(
        events.entries.values().next().unwrap().common,
        Some((141, 10))
    );
    events.section(&sharing_eit(0x50, 1, &[0x20]), 0);
    assert_eq!(events.entries.values().next().unwrap().common, None);
}

#[test]
fn scheduled_epg_batches_future_and_prioritizes_near_events_without_losing_retry() {
    use std::time::{Duration, Instant};
    let mut events = Events::default();
    let now = 1_800_000_000;
    let tick = Instant::now();
    for eid in 0..130u16 {
        let key = (1, 1, 101, eid);
        events.entries.insert(
            key,
            Event {
                sid: 101,
                eid,
                start: now + 86400,
                short: true,
                ..Event::default()
            },
        );
        events.dirty.push_back(key);
    }
    let mut batches = Vec::new();
    events.send_scheduled(
        tick,
        now,
        |_| true,
        |s| {
            batches.push(s.to_owned());
            true
        },
    );
    let first: serde_json::Value = serde_json::from_str(&batches[0][4..]).unwrap();
    assert_eq!(first.as_array().unwrap().len(), 64);
    events.send_scheduled(
        tick + Duration::from_secs(1),
        now,
        |_| true,
        |_| panic!("future EPG was not throttled"),
    );
    let key = (1, 1, 101, 999);
    events.entries.insert(
        key,
        Event {
            sid: 101,
            eid: 999,
            start: now + 60,
            short: true,
            ..Event::default()
        },
    );
    events.dirty.push_back(key);
    events.send_scheduled(
        tick + Duration::from_millis(1100),
        now,
        |_| true,
        |s| {
            assert!(s.contains("\"Eid\":999"));
            false
        },
    );
    assert!(events.dirty.contains(&key));
    events.send_scheduled(
        tick + Duration::from_millis(1200),
        now,
        |_| true,
        |s| {
            assert!(s.contains("\"Eid\":999"));
            true
        },
    );
    assert!(!events.dirty.contains(&key));
    events.send_scheduled(
        tick + Duration::from_secs(2),
        now,
        |_| true,
        |s| {
            batches.push(s.to_owned());
            true
        },
    );
    assert_eq!(events.dirty.len(), 2);
    events.send_scheduled(tick + Duration::from_secs(4), now, |_| true, |_| true);
    assert!(events.dirty.is_empty());
}

#[test]
fn scheduled_epg_preserves_current_program_notifications() {
    let mut events = Events::default();
    events.pf.insert(
        101,
        [
            Some(Event {
                sid: 101,
                eid: 1,
                title: "current".into(),
                ..Event::default()
            }),
            None,
        ],
    );
    let tick = std::time::Instant::now();
    events.send_scheduled(tick, 0, |_| true, |_| true);
    assert!(events.current(101).unwrap().contains("current"));
    events.pf.get_mut(&101).unwrap()[0].as_mut().unwrap().title = "updated".into();
    events.send_scheduled(tick, 0, |_| true, |_| panic!("EPG interval ignored"));
    assert!(events.current(101).unwrap().contains("updated"));
}
