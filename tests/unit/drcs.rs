use super::*;

#[test]
fn mapping_keys_are_sorted_unique_and_nonempty() {
    assert_eq!(MAPPINGS.len(), 3877);
    assert!(MAPPINGS.windows(2).all(|pair| pair[0].0 < pair[1].0));
    for &(hash, text) in MAPPINGS {
        assert!(!text.is_empty());
        assert_eq!(lookup(hash), Some(text));
    }
    assert_eq!(lookup(0), None);
}

#[test]
fn stable_hash_vectors_include_all_metadata_and_bytes() {
    assert_eq!(bitmap_hash(2, 1, 1, 0, 0, &[0x40]), 0x245dce3dde67bc9d);
    let key = bitmap_hash(18, 36, 2, 1, 2, &[0x55; 162]);
    for (w, h, bits, mode, depth) in [
        (36, 18, 2, 1, 2),
        (18, 36, 1, 1, 2),
        (18, 36, 2, 0, 2),
        (18, 36, 2, 1, 1),
    ] {
        assert_ne!(key, bitmap_hash(w, h, bits, mode, depth, &[0x55; 162]));
    }
    assert_ne!(key, bitmap_hash(18, 36, 2, 1, 2, &[0x54; 162]));
}

#[test]
fn supported_size_profiles_parse_without_row_alignment() {
    for (width, height, bits, mode, depth) in [
        (36u8, 36u8, 2, 1, 2),
        (30, 30, 2, 1, 2),
        (18, 36, 2, 1, 2),
        (15, 30, 2, 1, 2),
        (18, 18, 2, 1, 2),
        (15, 15, 2, 1, 2),
        (16, 18, 1, 0, 0),
    ] {
        let length = (usize::from(width) * usize::from(height) * usize::from(bits)).div_ceil(8);
        let bitmap = vec![0x55; length];
        let key = bitmap_hash(width, height, bits, mode, depth, &bitmap);
        let mut data = vec![1, 0x41, 0x21, 1, mode, depth, width, height];
        data.extend_from_slice(&bitmap);
        let resolve = |hash| (hash == key).then_some("test");
        assert_eq!(
            parse_with_lookup(&data, resolve).unwrap()[0].text,
            Some("test")
        );
        data.pop();
        assert!(parse_with_lookup(&data, resolve).is_none());
    }
}

#[test]
fn synthetic_glyphs_follow_reused_codes_and_reject_partial_units() {
    // Artificial patterns exercise the parser without distributing broadcast bitmaps.
    let mut data = vec![1, 0x41, 0x21, 1, 1, 2, 18, 36];
    data.extend_from_slice(&[0x55; 162]);
    let key = bitmap_hash(18, 36, 2, 1, 2, &data[8..]);
    let resolve = |hash| (hash == key).then_some("𠮷");
    let mut map = Map::default();
    map.install(parse_with_lookup(&data, resolve).unwrap());
    assert_eq!(map.get(0x4121), Some("𠮷"));
    #[cfg(feature = "drcs-collect")]
    assert!(
        parse_with_lookup(&data, resolve).unwrap()[0]
            .unmapped
            .is_empty()
    );
    for end in 0..data.len() {
        assert!(parse_with_lookup(&data[..end], resolve).is_none());
    }
    data[6] = 36;
    data[7] = 18;
    assert_eq!(parse_with_lookup(&data, resolve).unwrap()[0].text, None);
    data[6] = 18;
    data[7] = 36;
    data[8] ^= 1;
    let entries = parse_with_lookup(&data, resolve).unwrap();
    #[cfg(feature = "drcs-collect")]
    {
        assert_eq!(entries[0].unmapped.len(), 1);
        assert_eq!(entries[0].unmapped[0].data, data[8..]);
    }
    map.install(entries);
    assert_eq!(map.get(0x4121), None);
    data[0] = 2;
    assert!(parse_with_lookup(&data, resolve).is_none());
}
