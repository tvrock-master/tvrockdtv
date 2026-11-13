use tvrockdtv::caption_layout::CaptionDecoder;

#[test]
fn time_before_clear_preserves_order_and_accumulates_across_units() {
    let mut d = CaptionDecoder::new(false);
    let events = d.statement(
        &[&[0x0c, 0x0e, b'A'], &[0x9d, 0x20, 0x6a, 0x0c]],
        278,
        90000,
        1,
    );
    assert_eq!(events.len(), 3);
    assert!(events[0].clear);
    assert_eq!(events[1].text, "Ａ");
    assert!(events[2].clear);
    assert_eq!(events[2].delay_ms, 4200);
    let events = d.statement(
        &[&[0x9d, 0x20, 0x7f, 0x9d, 0x20, 0x7f, 0x9d, 0x20, 0x46, 0x0c]],
        278,
        90000,
        1,
    );
    assert_eq!(events[0].delay_ms, 13200);
}

#[test]
fn csi_positions_background_and_ruby_are_separate_runs() {
    let mut d = CaptionDecoder::new(false);
    let events = d.statement(&[b"\x0c\x9b7 S\x9b620;480 V\x9b170;30 _\x9b4 X\x9b24 Y\x9b36;36 W\x9b170;509 a\x89\x90\x20\x44\x90\x51\x0eAB\x88\x9b180;469 aCD"], 278, 0, 1);
    assert_eq!(events.len(), 3);
    let body = &events[1];
    let ruby = &events[2];
    assert_eq!(body.text, "AB");
    assert_eq!(body.layout.as_ref().unwrap().y, 449.);
    assert_eq!(ruby.layout.as_ref().unwrap().y, 439.);
    assert_eq!(body.layout.as_ref().unwrap().glyph_height, 36.);
    assert_eq!(ruby.layout.as_ref().unwrap().glyph_height, 18.);
    for event in [body, ruby] {
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["BgColor"], 65);
        assert_eq!(value["Timing"], 1);
    }
}

#[test]
fn style_changes_mid_line_do_not_apply_to_preceding_text() {
    let mut d = CaptionDecoder::new(false);
    let events = d.statement(&[b"\x0c\x0e\x89A\x81B\x90\x50C"], 1, 0, 1);
    let values: Vec<_> = events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert_eq!(values[1]["Color"], 7);
    assert_eq!(values[1]["BgColor"], 8);
    assert_eq!(values[2]["Color"], 1);
    assert_eq!(values[2]["BgColor"], 8);
    assert_eq!(values[3]["BgColor"], 0);
    assert_eq!(values[3]["Layout"]["X"], 40.);
}

#[test]
fn clear_resets_charset_and_style_but_keeps_statement_delay() {
    let mut d = CaptionDecoder::new(false);
    let events = d.statement(&[b"\x0e\x89A\x9d\x20\x4a\x0c\x0eB"], 1, -1, 1);
    assert_eq!(events[0].text, "A");
    assert!(events[1].clear);
    assert_eq!(events[2].text, "Ｂ");
    assert_eq!(events[2].delay_ms, 1000);
}

#[test]
fn truncated_control_sequences_do_not_become_caption_text() {
    for bytes in [
        b"\x9b36;36 ".as_slice(),
        &[0x9d, 0x20],
        &[0x1c, 0x40],
        &[0x90, 0x20],
    ] {
        let mut d = CaptionDecoder::new(false);
        assert!(d.statement(&[bytes], 1, 0, 1).is_empty());
    }
}

fn drcs_unit(bitmap: &[u8]) -> Vec<u8> {
    let mut unit = vec![1, 0x41, 0x21, 1, 1, 2, 36, 36];
    unit.extend_from_slice(bitmap);
    unit
}
#[test]
fn truncated_drcs_does_not_panic_or_install_partial_definitions() {
    let right = drcs_unit(&[0x55; 324]);
    for end in 0..right.len() {
        let mut d = CaptionDecoder::new(false);
        let events = d.statement_units(
            &[(0x30, &right[..end]), (0x20, b"\x1b\x28\x20\x41\x21")],
            304,
            90,
            1,
        );
        assert_eq!(events[0].text, "□", "truncated at {end}");
    }
}
