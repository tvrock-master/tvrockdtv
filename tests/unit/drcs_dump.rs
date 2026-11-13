use super::*;
#[test]
fn grayscale_orientation_and_unaligned_rows() {
    let b = Bitmap {
        code: 0x4121,
        font_id: 0,
        mode: 1,
        depth: 2,
        width: 3,
        height: 2,
        bits: 2,
        data: vec![0b00011011, 0b10010000],
    };
    let bmp = b.bmp();
    assert_eq!(&bmp[54..63], &[0, 0, 0, 85, 85, 85, 170, 170, 170]);
    assert_eq!(&bmp[66..75], &[255, 255, 255, 170, 170, 170, 85, 85, 85]);
    assert_eq!(bmp.len(), 78);
}
#[test]
fn lossless_metadata_and_deduplication_across_codes() {
    let directory = std::env::temp_dir().join(format!(
        "tvrock-drcs-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut b = Bitmap {
        code: 0x4121,
        font_id: 0,
        mode: 0,
        depth: 0,
        width: 2,
        height: 1,
        bits: 1,
        data: vec![0x40],
    };
    b.save(&directory).unwrap();
    let name = b.name();
    b.code = 0x4122;
    b.save(&directory).unwrap();
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join(format!("{name}.json"))).unwrap()).unwrap();
    assert_eq!(metadata["bitmap_hex"], "40");
    assert_eq!(metadata["character_code"], "0x4121");
    b.data[0] = 0x80;
    b.save(&directory).unwrap();
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 4);
    for entry in fs::read_dir(&directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}
