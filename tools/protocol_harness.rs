// tungstenite fixes the handshake callback error type.
#![allow(clippy::result_large_err)]
//! Local-only protocol peer used to exercise the actual C++ DLL host executable.
use std::{
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{
    Error, Message, accept_hdr,
    handshake::server::{Request, Response},
};
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let executable = args.get(1).expect("host executable");
    let self_delete = args.get(2).is_some_and(|s| s == "self-delete");
    let sibling = args.iter().skip(2).any(|s| s == "sibling");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let server = thread::spawn(move || {
        let mut connections = 0;
        while !stop.load(Ordering::Acquire) {
            let (stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            // Winsock accepted sockets inherit the nonblocking listener mode.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = accept_hdr(stream, |r: &Request, response: Response| {
                assert_eq!(r.uri().path(), "/dev");
                assert_eq!(r.headers()["Authorization"], "Bearer test-token");
                assert_eq!(
                    r.headers()["User-Agent"],
                    if sibling {
                        concat!("TvRockDTV-Sib/", env!("TVROCKDTV_VERSION"))
                    } else {
                        concat!("TvRockDTV/", env!("TVROCKDTV_VERSION"))
                    }
                );
                if sibling {
                    assert_eq!(r.headers()["X-TvRockDTV-Parent"], "parent-token");
                } else {
                    assert!(!r.headers().contains_key("X-TvRockDTV-Parent"));
                }
                Ok(response)
            })
            .unwrap();
            connections += 1;
            if sibling {
                // No CH2/LoopBack/Ready handshake or TS/status messages in sibling mode.
                for (name, message) in [
                    ("Ready", "0,101"),
                    ("LoopBack", "LTC:probe"),
                    ("RecordStart", "__test_第一.ts"),
                    ("QuickRecord", "__test_第二.ts"),
                    ("RecordStartQuick", "__test_第二.ts"),
                    ("StreamChange", "16"),
                    ("RecordStop", ""),
                    ("OnOption", ""),
                    ("Capture", ""),
                ] {
                    ws.send(Message::text(
                        serde_json::json!({"command":name,"message":message,"deviceid":"REC01"})
                            .to_string(),
                    ))
                    .unwrap();
                }
                if self_delete {
                    ws.send(Message::text(r#"{"command":"PowerOff"}"#)).unwrap();
                }
                loop {
                    match ws.read() {
                        Ok(Message::Text(text)) => {
                            assert!(
                                text.starts_with("RCV:"),
                                "sibling transmitted recorder data: {text}"
                            );
                            let status: serde_json::Value =
                                serde_json::from_str(&text[4..]).unwrap();
                            assert!(status["ready"].is_boolean());
                        }
                        Ok(Message::Binary(_)) => panic!("sibling transmitted binary data"),
                        Ok(Message::Close(_)) => {
                            ws.flush().ok();
                            break;
                        }
                        Err(Error::ConnectionClosed | Error::AlreadyClosed) => break,
                        other => panic!("unexpected sibling frame: {other:?}"),
                    }
                }
                continue;
            }
            ws.send(Message::text(
                serde_json::json!({"command":"LoopBack","message":"LTC:probe","deviceid":"REC01"})
                    .to_string(),
            ))
            .unwrap();
            let mut got_ch2 = false;
            let mut got_loopback = false;
            while !got_ch2 || !got_loopback {
                match ws.read() {
                    Ok(Message::Text(text)) => {
                        got_ch2 |= text.starts_with("CH2:");
                        got_loopback |= text == concat!("LTC:probe,", env!("TVROCKDTV_VERSION"));
                    }
                    other => panic!(
                        "initial handshake CH2={got_ch2}, LoopBack={got_loopback}: {other:?}"
                    ),
                }
            }
            ws.send(Message::text(
                serde_json::json!({"command":"Ready","message":"0,101","deviceid":"REC01"})
                    .to_string(),
            ))
            .unwrap();
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut sent = false;
            let mut srt_seen = false;
            while !stop.load(Ordering::Acquire) {
                match ws.read() {
                    Ok(Message::Text(text)) => {
                        if let Some(json) = text.strip_prefix("STR:") {
                            let value: serde_json::Value = serde_json::from_str(json).unwrap();
                            assert!(
                                !text.contains("integration-test-key"),
                                "secret leaked into status"
                            );
                            srt_seen |= value["srtSupported"] == true
                                && value["transport"] == "srt"
                                && value["success"] == true;
                        }
                        if text == "STC:10,101" && !sent {
                            sent = true;
                            let commands = if self_delete {
                                vec![("PowerOff", "")]
                            } else {
                                vec![
                                    ("RecordStart", "__test_第一.ts"),
                                    ("RecordStart", "__test_第二.ts"),
                                    ("StreamChange", "16"),
                                    ("Capture", ""),
                                    ("RecordStop", ""),
                                ]
                            };
                            if !self_delete && !sibling {
                                let session = "e".repeat(64);
                                let message = serde_json::json!({"session":session,"address":"127.0.0.1","port":9,"flags":6,"latencyMs":120,"passphrase":"integration-test-key"}).to_string();
                                ws.send(Message::text(serde_json::json!({"command":"StreamStartSRT","message":message}).to_string())).unwrap();
                                ws.send(Message::text(serde_json::json!({"command":"StreamStop","message":serde_json::json!({"session":session}).to_string()}).to_string())).unwrap();
                            }
                            for (name, message) in commands {
                                ws.send(Message::text(serde_json::json!({"command":name,"message":message,"deviceid":"REC01"}).to_string())).unwrap();
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        ws.flush().ok();
                        break;
                    }
                    Err(Error::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                    _ => (),
                }
            }
            if !self_delete && !sibling {
                assert!(srt_seen, "DLL did not acknowledge SRT transport/capability");
            }
        }
        connections
    });
    let mut command = Command::new(executable);
    if self_delete {
        command.arg("self-delete");
    }
    command.args(["/host", &address.to_string(), "/pid", "test-token"]);
    if sibling {
        command.args(["/parent", "parent-token"]);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            panic!("DLL test timeout")
        }
        thread::sleep(Duration::from_millis(20));
    };
    stopped.store(true, Ordering::Release);
    let count = server.join().unwrap();
    assert!(status.success(), "DLL host failed: {status}");
    assert_eq!(count, if self_delete { 1 } else { 3 });
}
