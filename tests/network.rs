// tungstenite fixes the handshake callback error type.
#![allow(clippy::result_large_err)]
use std::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{
    self, Message, WebSocket,
    handshake::server::{Request, Response},
};
use tvrockdtv::dtvclient::{Client, split_uints, websocket_url};
fn listener() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    (l, url)
}
fn accept(l: &TcpListener) -> WebSocket<TcpStream> {
    let (s, _) = l.accept().unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    tungstenite::accept_hdr(s, |r: &Request, response: Response| {
        assert_eq!(r.uri().path(), "/dev");
        assert_eq!(r.headers()["Authorization"], "Bearer test-token");
        assert!(!r.headers().contains_key("X-TvRockDTV-Parent"));
        assert_eq!(
            r.headers()["User-Agent"],
            concat!("TvRockDTV/", env!("TVROCKDTV_VERSION"))
        );
        assert_eq!(
            r.headers()["Content-Type"],
            "application/json; charset=utf8"
        );
        Ok(response)
    })
    .unwrap()
}
fn command(ws: &mut WebSocket<TcpStream>, name: &str, message: &str) {
    ws.send(Message::text(
        serde_json::json!({"deviceid":"REC01","command":name,"message":message}).to_string(),
    ))
    .unwrap();
}
fn text(ws: &mut WebSocket<TcpStream>) -> String {
    loop {
        match ws.read().unwrap() {
            Message::Text(t) => return t.to_string(),
            Message::Ping(_) => ws.flush().unwrap(),
            m => panic!("unexpected frame: {m:?}"),
        }
    }
}

#[test]
fn original_go_url_and_integer_tests_plus_validation() {
    for (input, want) in [
        ("localhost:8080", "ws://localhost:8080/dev"),
        ("http://localhost:8080", "ws://localhost:8080/dev"),
        ("https://example.com", "wss://example.com/dev"),
        ("wss://example.com/path?q=1", "wss://example.com/dev"),
        ("ws://[::1]:8080", "ws://[::1]:8080/dev"),
    ] {
        assert_eq!(websocket_url(input).unwrap().as_str(), want)
    }
    for bad in ["", "ftp://example.com", "http://user:password@example.com"] {
        assert!(websocket_url(bad).is_err())
    }
    assert_eq!(split_uints("1,2,invalid,65535"), [1, 2, 65535]);
    assert_eq!(split_uints("-1,4294967296, 2,3"), [3]);
    assert!(Client::start("bad\r\nheader", "localhost:1").is_err());
    for parent in ["", "bad\r\nheader"] {
        assert!(Client::start_with_parent("test-token", "localhost:1", Some(parent)).is_err());
    }
}
#[test]
fn sibling_headers_are_preserved_on_reconnect() {
    let (l, url) = listener();
    let (connected_tx, connected_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        for attempt in 0..2 {
            let (s, _) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept_hdr(s, |r: &Request, response: Response| {
                assert_eq!(r.uri().path(), "/dev");
                assert_eq!(r.headers()["Authorization"], "Bearer test-token");
                assert_eq!(r.headers()["X-TvRockDTV-Parent"], "parent-token");
                assert_eq!(
                    r.headers()["User-Agent"],
                    concat!("TvRockDTV-Sib/", env!("TVROCKDTV_VERSION"))
                );
                Ok(response)
            })
            .unwrap();
            if attempt == 1 {
                connected_tx.send(()).unwrap();
                done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            ws.close(None).ok();
        }
    });
    let c = Client::start_with_parent("test-token", &url, Some("parent-token")).unwrap();
    connected_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(c.wait_ready(Duration::from_secs(4)));
    done_tx.send(()).unwrap();
    c.stop();
    server.join().unwrap();
}
#[test]
fn sibling_sends_no_application_data_and_accepts_only_capture_and_poweroff() {
    let (l, url) = listener();
    let (capture_tx, capture_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut ws = tungstenite::accept(s).unwrap();
        for (name, message) in [
            ("Ready", "1,101"),
            ("LoopBack", "LTC:probe"),
            ("RecordStart", "ignored.ts"),
            ("QuickRecord", "ignored.ts"),
            ("RecordStartQuick", "ignored.ts"),
            ("RecordStop", ""),
            ("StreamChange", "16"),
            ("OnOption", ""),
        ] {
            command(&mut ws, name, message);
        }
        ws.send(Message::Ping(vec![1, 2, 3].into())).unwrap();
        assert_eq!(ws.read().unwrap(), Message::Pong(vec![1, 2, 3].into()));
        ws.send(Message::Binary(br#"{"command":"Capture"}"#.to_vec().into()))
            .unwrap();
        capture_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        command(&mut ws, "PowerOff", "");
        assert!(matches!(ws.read().unwrap(), Message::Close(_)));
        ws.flush().ok();
    });
    let c = Client::start_with_parent("test-token", &url, Some("parent-token")).unwrap();
    // A sibling is ready after HTTP upgrade, without receiving Ready.
    for text in [
        "CH2:channels",
        "STA:3",
        "STC:7,101",
        "EIT:[]",
        "TS:data",
        "CDT:{}",
    ] {
        assert!(c.send(text));
    }
    assert!(c.wait_ready(Duration::from_secs(4)));
    assert_eq!(c.next_event(Duration::from_secs(4)).unwrap().kind, 0x15);
    assert!(c.next_event(Duration::from_millis(100)).is_none());
    assert_eq!(c.tuner_type(), 255);
    assert!(!c.valid_service(101));
    capture_tx.send(()).unwrap();
    assert_eq!(c.next_event(Duration::from_secs(4)).unwrap().kind, 0x13);
    assert!(c.next_event(Duration::from_millis(100)).is_none());
    server.join().unwrap();
}
#[test]
fn handshake_ready_outgoing_and_typed_commands() {
    let (l, url) = listener();
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "1,101,102,65535");
        assert_eq!(text(&mut ws), "STC:7,101");
        command(&mut ws, "RecordStart", "C:\\録画\\第一.ts");
        command(&mut ws, "RecordStart", "C:\\録画\\第二.ts");
        command(&mut ws, "StreamChange", "16");
        command(&mut ws, "StreamChange", "4294901861");
        command(&mut ws, "RecordStop", "");
        command(&mut ws, "Capture", "");
        command(&mut ws, "QuickRecord", "quick.ts");
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        ws.close(None).ok();
    });
    let c = Client::start("test-token", &url).unwrap();
    assert!(c.wait_ready(Duration::from_secs(4)));
    assert_eq!(c.tuner_type(), 1);
    assert!(!c.valid_service(1));
    assert!(c.valid_service(101));
    assert!(c.send("STC:7,101"));
    let first = c.next_event(Duration::from_secs(4)).unwrap();
    let second = c.next_event(Duration::from_secs(4)).unwrap();
    assert_eq!(first.filename, "C:\\録画\\第一.ts");
    assert_eq!(second.filename, "C:\\録画\\第二.ts");
    let change = c.next_event(Duration::from_secs(4)).unwrap();
    assert_eq!((change.kind, change.channel, change.service), (0x14, 0, 16));
    let change = c.next_event(Duration::from_secs(4)).unwrap();
    assert_eq!(
        (change.kind, change.channel, change.service),
        (0x14, 65535, 101)
    );
    for kind in [0x11, 0x15, 0x12] {
        assert_eq!(c.next_event(Duration::from_secs(4)).unwrap().kind, kind)
    }
    done_tx.send(()).unwrap();
    c.stop();
    server.join().unwrap();
}
#[test]
fn malformed_messages_are_ignored() {
    let (l, url) = listener();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "0,101");
        ws.send(Message::text("not json")).unwrap();
        ws.send(Message::text(r#"{"deviceid":"REC01","command":"Notify"}"#))
            .unwrap();
        command(&mut ws, "StreamChange", "-1");
        command(&mut ws, "StreamChange", "4294967296");
        command(&mut ws, "RecordStart", "bad\0path");
        command(&mut ws, "Capture", "");
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let c = Client::start("test-token", &url).unwrap();
    assert!(c.wait_ready(Duration::from_secs(4)));
    assert_eq!(c.next_event(Duration::from_secs(4)).unwrap().kind, 0x15);
    assert!(c.next_event(Duration::from_millis(100)).is_none());
    tx.send(()).unwrap();
    c.stop();
    server.join().unwrap();
}
#[test]
fn reconnect_replays_all_sdt_sections_and_disconnected_cache() {
    let (l, url) = listener();
    let (connected_tx, connected_rx) = mpsc::channel();
    let (close_tx, close_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "0,101");
        connected_tx.send(()).unwrap();
        close_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(ws);
        let mut ws = accept(&l);
        assert_eq!(text(&mut ws), "CH2:test");
        command(&mut ws, "Ready", "0,101");
        let messages = (0..4).map(|_| text(&mut ws)).collect::<Vec<_>>();
        assert_eq!(messages[0], "DIF:new");
        assert_eq!(messages[1], "STC:9,101");
        assert!(messages[2].contains("\"Section\":0"));
        assert!(messages[3].contains("\"Section\":1"));
        connected_tx.send(()).unwrap();
        finish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let c = Client::start("test-token", &url).unwrap();
    assert!(c.wait_ready(Duration::from_secs(4)));
    connected_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    let first = c.generation();
    assert!(c.send("CH2:test"));
    assert!(c.send("DIF:old"));
    assert!(c.send("STC:9,101"));
    for section in 0..2 {
        assert!(c.send(&format!(
            "SDT:[{{\"TableId\":66,\"Tsid\":1,\"Onid\":2,\"Version\":0,\"Section\":{section}}}]"
        )));
    }
    close_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while c.generation() != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(c.generation(), 0);
    assert!(c.send("DIF:new"));
    assert!(!c.send("CAP:stale"));
    assert!(c.wait_ready(Duration::from_secs(4)));
    assert!(c.generation() > first);
    connected_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    finish_tx.send(()).unwrap();
    c.stop();
    server.join().unwrap();
}
#[test]
fn stop_cancels_idle_event_wait_and_network_read() {
    let (l, url) = listener();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "0,101");
        loop {
            match ws.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => (),
            }
        }
    });
    let c = std::sync::Arc::new(Client::start("test-token", &url).unwrap());
    assert!(c.wait_ready(Duration::from_secs(4)));
    let waiter = c.clone();
    let t = thread::spawn(move || assert!(waiter.next_event(Duration::from_secs(30)).is_none()));
    let start = Instant::now();
    c.stop();
    t.join().unwrap();
    drop(c);
    assert!(start.elapsed() < Duration::from_secs(2));
    server.join().unwrap();
}
#[test]
fn stop_cancels_a_stalled_http_upgrade() {
    let (l, url) = listener();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (_socket, _) = l.accept().unwrap();
        tx.send(()).unwrap();
        rx_wait();
    });
    fn rx_wait() {
        thread::sleep(Duration::from_millis(500));
    }
    let c = Client::start("test-token", &url).unwrap();
    rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = Instant::now();
    drop(c);
    assert!(start.elapsed() < Duration::from_secs(2));
    server.join().unwrap();
}
#[test]
fn loopback_and_poweroff_stop_reconnecting() {
    let (l, url) = listener();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "0,101");
        command(&mut ws, "LoopBack", "LTC:1791504000");
        assert_eq!(
            text(&mut ws),
            concat!("LTC:1791504000,", env!("TVROCKDTV_VERSION"))
        );
        command(&mut ws, "LoopBack", "echo");
        assert_eq!(text(&mut ws), "echo");
        command(&mut ws, "PowerOff", "");
    });
    let c = Client::start("test-token", &url).unwrap();
    let event = c.next_event(Duration::from_secs(4)).unwrap();
    assert_eq!(event.kind, 0x13);
    server.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !c.stopped() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5))
    }
    assert!(c.stopped());
}

#[test]
fn real_server_requires_ch2_and_loopback_before_ready() {
    let (l, url) = listener();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        command(&mut ws, "LoopBack", "LTC:probe");
        let messages = [text(&mut ws), text(&mut ws)];
        assert!(messages.iter().any(|s| s == "CH2:file not found"));
        assert!(
            messages
                .iter()
                .any(|s| s == concat!("LTC:probe,", env!("TVROCKDTV_VERSION")))
        );
        command(&mut ws, "Ready", "0,101");
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let c = Client::start("test-token", &url).unwrap();
    assert!(c.send("CH2:file not found"));
    assert!(c.wait_ready(Duration::from_secs(4)), "{}", c.last_error());
    tx.send(()).unwrap();
    c.stop();
    server.join().unwrap();
}

#[test]
fn cdt_waits_for_ready_and_is_not_replayed_after_delivery() {
    let (l, url) = listener();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut ws = accept(&l);
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        command(&mut ws, "Ready", "0,101");
        assert!(text(&mut ws).starts_with("CDT:"));
        ws.close(None).ok();
        drop(ws);
        let mut ws = accept(&l);
        command(&mut ws, "Ready", "0,101");
        command(&mut ws, "LoopBack", "LTC:after-cdt");
        assert_eq!(
            text(&mut ws),
            concat!("LTC:after-cdt,", env!("TVROCKDTV_VERSION"))
        );
        ws.get_mut()
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        assert!(ws.read().is_err(), "CDT must not be replayed");
        done_tx.send(()).unwrap();
    });
    let c = Client::start("test-token", &url).unwrap();
    assert!(c.send(r#"CDT:{"Onid":4,"Sid":101,"LogoType":5,"Data":"00"}"#));
    ready_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(8)).unwrap();
    c.stop();
    server.join().unwrap();
}
