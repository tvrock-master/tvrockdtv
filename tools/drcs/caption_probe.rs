//! Offline TS caption inspection; does not contact a server or create recordings.
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
};
use tvrockdtv::engine::Engine;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() < 3 {
        return Err("usage: caption_probe input.ts output.jsonl [service-id]".into());
    }
    let mut input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    let mut engine = Engine::default();
    if let Some(sid) = args.get(3) {
        engine.service = sid.to_string_lossy().parse()?;
    }
    let mut buffer = vec![0u8; 188 * 4096];
    let mut bytes = 0u64;
    let mut captions = 0u64;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        for message in engine.process(&buffer[..n]) {
            if let Some(json) = message.strip_prefix("CAP:") {
                writeln!(output, "{json}")?;
                captions += 1;
            }
        }
    }
    output.flush()?;
    println!(
        "bytes={bytes} caption_events={captions} streams={:?}",
        engine.caption_streams()
    );
    Ok(())
}
