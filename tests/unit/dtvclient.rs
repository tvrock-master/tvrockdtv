use super::*;

fn test_shared() -> Shared {
    let (stop, _) = watch::channel(false);
    Shared {
        sibling: false,
        bml_enabled: true,
        state: Mutex::new(State {
            ready: true,
            ..State::default()
        }),
        changed: Condvar::new(),
        outgoing: Notify::new(),
        stop,
    }
}

#[test]
fn sibling_notifications_are_not_cached_or_queued() {
    let mut shared = test_shared();
    shared.sibling = true;
    for ready in [false, true] {
        shared.state().ready = ready;
        for text in [
            "CH2:channels",
            "DIF:info",
            "STA:3",
            "STC:7,101",
            "CPF:caption",
            "SDT:[]",
            "EIT:[]",
            "CDT:{}",
            "TS:data",
        ] {
            assert!(shared.send(text));
        }
        let s = shared.state();
        assert!(s.cache.is_empty());
        assert!(s.outgoing.is_empty());
        assert_eq!((s.cache_bytes, s.outgoing_bytes), (0, 0));
    }
}

#[test]
fn identical_receiver_reports_are_heartbeats_even_without_packets() {
    let mut shared = test_shared();
    shared.sibling = true;
    let report = r#"RCV:{"ready":true,"bytes":0,"ageMs":18446744073709551615}"#;
    assert!(shared.send(report));
    assert!(shared.send(report));
    assert_eq!(shared.state().outgoing.len(), 2);
}

#[test]
fn stream_commands_validate_numeric_destination_and_session() {
    let shared = test_shared();
    let session = "a".repeat(64);
    let request = |address: &str, port: u32| {
        serde_json::json!({"command":"StreamStart","message":serde_json::json!({"session":session,"address":address,"port":port}).to_string()}).to_string()
    };
    for address in [
        "evil.example",
        "0.0.0.0",
        "255.255.255.255",
        "224.0.0.1",
        "1.2.3.4 & evil",
    ] {
        shared.receive(request(address, 1234).as_bytes());
        assert!(shared.state().events.is_empty());
    }
    shared.receive(request("192.168.1.2", 0).as_bytes());
    assert!(shared.state().events.is_empty());
    shared.receive(request("192.168.1.2", 1234).as_bytes());
    let event = shared.state().events.pop_front().unwrap();
    assert_eq!(event.kind, 0x20);
    assert_eq!(event.channel, 15);
    assert_eq!(event.service, 1234);
    assert_eq!(event.filename, format!("{session},192.168.1.2"));
    shared.receive(
        serde_json::json!({"command":"StreamStop","message":"bad"})
            .to_string()
            .as_bytes(),
    );
    assert!(shared.state().events.is_empty());
}

#[test]
fn pacing_commands_only_accept_session_identifiers() {
    let shared = test_shared();
    for session in ["bad".to_owned(), "x".repeat(64), "a".repeat(65)] {
        let request = serde_json::json!({"command":"StreamPace","message":serde_json::json!({"session":session}).to_string()});
        shared.receive(request.to_string().as_bytes());
        assert!(shared.state().events.is_empty());
    }
    let token = "a".repeat(64);
    let request = serde_json::json!({"command":"StreamPace","message":serde_json::json!({"session":token}).to_string()});
    shared.receive(request.to_string().as_bytes());
    let event = shared.state().events.pop_front().unwrap();
    assert_eq!(event.kind, 0x24);
    assert_eq!(event.filename, token);
    assert_eq!((event.channel, event.service), (0, 0));
}

#[test]
fn stream_option_flags_are_strict_and_normalized() {
    let shared = test_shared();
    let session = "a".repeat(64);
    for flags in [
        serde_json::json!(16),
        serde_json::json!(-1),
        serde_json::json!("7"),
        serde_json::json!(true),
    ] {
        let request = serde_json::json!({"command":"StreamStart","message":serde_json::json!({"session":session,"address":"192.168.1.2","port":1234,"flags":flags}).to_string()});
        shared.receive(request.to_string().as_bytes());
        assert!(shared.state().events.is_empty());
    }
    let request = serde_json::json!({"command":"StreamStart","message":serde_json::json!({"session":session,"address":"192.168.1.2","port":1234,"flags":8}).to_string()});
    shared.receive(request.to_string().as_bytes());
    assert_eq!(
        shared.state().events.pop_front().unwrap().filename,
        format!("{session},192.168.1.2,9")
    );
}

#[test]
fn identical_sdt_is_cached_and_queued_once() {
    let shared = test_shared();
    let message = "SDT:[{\"TableId\":66,\"Tsid\":1,\"Onid\":2,\"Version\":3,\"Section\":0,\"name\":\"test\"}]";
    assert!(shared.send(message));
    let changed = message.replace("test", "changed");
    assert!(shared.send(&changed));
    let state = shared.state();
    assert_eq!(state.cache.len(), 1);
    assert_eq!(state.outgoing.len(), 1);
}

#[test]
fn sdt_oneseg_resolution_is_sent_after_initial_unknown_classification() {
    for oneseg in [0, 1] {
        let shared = test_shared();
        let unknown = r#"SDT:[{"TableId":66,"Tsid":1,"Onid":2,"Version":3,"Section":0,"Sid":101,"OneSeg":-1,"RunningStatus":0}]"#;
        assert!(shared.send(unknown));
        {
            let mut state = shared.state();
            state.outgoing.clear();
            state.outgoing_bytes = 0;
        }
        let resolved = unknown.replace("\"OneSeg\":-1", &format!("\"OneSeg\":{oneseg}"));
        assert!(shared.send(&resolved));
        {
            let state = shared.state();
            assert_eq!(
                state.outgoing.len(),
                1,
                "resolved classification was suppressed"
            );
            assert_eq!(state.outgoing.front().unwrap().1, resolved);
            assert_eq!(state.cache.values().next().unwrap(), &resolved);
        }
        assert!(shared.send(&resolved));
        assert!(shared.send(&resolved.replace("\"RunningStatus\":0", "\"RunningStatus\":4")));
        assert_eq!(shared.state().outgoing.len(), 1);
    }
}
#[test]
fn reset_service_removes_sdt_cache_and_pending_messages() {
    let shared = test_shared();
    assert!(shared.send(
        "SDT:[{\"TableId\":66,\"Tsid\":1,\"Onid\":2,\"Version\":3,\"Section\":0,\"name\":\"test\"}]"
    ));
    assert!(shared.send("EIT:[]"));
    let client = Client {
        shared: Arc::new(shared),
        worker: Mutex::new(None),
    };
    client.reset_service();
    let state = client.shared.state();
    assert!(state.cache.is_empty());
    assert_eq!(state.outgoing.len(), 1);
    assert_eq!(state.outgoing.front().unwrap().1, "EIT:[]");
}

#[test]
fn outgoing_queue_bounds_and_coalesces_status() {
    let mut s = State::default();
    for n in 0..100 {
        assert!(enqueue(&mut s, Some("STA:".into()), format!("STA:{n}")));
    }
    assert_eq!(s.outgoing.len(), 1);
    assert_eq!(s.outgoing.front().unwrap().1, "STA:99");
    for _ in 1..MAX_QUEUE {
        assert!(enqueue(&mut s, None, "EIT:[]".into()));
    }
    assert!(!enqueue(&mut s, None, "CAP:overflow".into()));
    assert_eq!(
        s.outgoing_bytes,
        s.outgoing.iter().map(|(_, v)| v.len()).sum::<usize>()
    );
    let mut s = State::default();
    for _ in 0..8 {
        assert!(enqueue(&mut s, None, "x".repeat(MAX_MESSAGE)));
    }
    assert!(!enqueue(&mut s, None, "x".into()));
}

#[test]
fn srt_commands_validate_options_and_preserve_delimiters() {
    let shared = test_shared();
    let session = "a".repeat(64);
    let request = |latency: serde_json::Value, secret: &str| {
        serde_json::json!({"command":"StreamStartSRT","message":serde_json::json!({"session":session,"address":"127.0.0.1","port":3230,"flags":7,"latencyMs":latency,"passphrase":secret}).to_string()}).to_string()
    };
    for latency in [
        serde_json::json!(-1),
        serde_json::json!(60001),
        serde_json::json!("120"),
    ] {
        shared.receive(request(latency, "").as_bytes());
        assert!(shared.state().events.is_empty());
    }
    for secret in ["short", "with-newline\n", "日本語のパスフレーズ"] {
        shared.receive(request(serde_json::json!(120), secret).as_bytes());
        assert!(shared.state().events.is_empty());
    }
    let secret = "commas,quotes\"and spaces";
    shared.receive(request(serde_json::json!(0), secret).as_bytes());
    let event = shared.state().events.pop_front().unwrap();
    assert_eq!((event.kind, event.service, event.channel), (0x25, 3230, 15));
    let encoded: String = secret.bytes().map(|b| format!("{b:02x}")).collect();
    assert_eq!(event.filename, format!("{session},127.0.0.1,7,0,{encoded}"));
}

#[test]
fn playback_uses_separate_caption_protocol_and_requires_control_ack() {
    let mut shared = test_shared();
    shared.sibling = true;
    assert!(shared.send("CAP:{\"Text\":\"before\"}"));
    assert!(shared.state().outgoing.is_empty());
    let command = serde_json::json!({"command":"PlaybackControl","message":serde_json::json!({"generation":3,"action":"reset"}).to_string()});
    assert!(shared.receive(command.to_string().as_bytes()));
    assert_eq!(shared.state().events.back().unwrap().kind, 0x26);
    assert!(shared.send("CAP:{\"Text\":\"pending\"}"));
    assert!(shared.state().outgoing.is_empty());
    assert!(shared.send("PBA:{\"generation\":3}"));
    assert!(shared.send("CAP:{\"Text\":\"再生字幕\"}"));
    assert!(shared.send("EIT:[]"));
    let state = shared.state();
    let text = &state.outgoing.back().unwrap().1;
    assert!(text.starts_with("PBC:"));
    let value: serde_json::Value = serde_json::from_str(&text[4..]).unwrap();
    assert_eq!(value["generation"], 3);
    assert_eq!(value["caption"]["Text"], "再生字幕");
    assert!(state.cache.is_empty());
    assert_eq!(state.outgoing.len(), 2);
}

#[test]
fn playback_direct_controls_report_failure_without_enabling_captions() {
    let mut shared = test_shared();
    shared.sibling = true;
    for (action, service) in [("pause-direct", 2), ("resume-direct", 3)] {
        let command = serde_json::json!({"command":"PlaybackControl","message":serde_json::json!({"generation":4,"action":action}).to_string()});
        assert!(shared.receive(command.to_string().as_bytes()));
        let state = shared.state();
        let event = state.events.back().unwrap();
        assert_eq!((event.kind, event.service), (0x26, service));
    }
    assert!(shared.send(r#"PBA:{"generation":4,"action":"resume-direct","success":false}"#));
    assert!(shared.send(r#"CAP:{"Text":"must stay blocked"}"#));
    let state = shared.state();
    assert!(
        state
            .outgoing
            .iter()
            .any(|(_, text)| text.starts_with("PBA:"))
    );
    assert!(
        !state
            .outgoing
            .iter()
            .any(|(_, text)| text.starts_with("PBC:"))
    );
    drop(state);
    assert!(shared.send(r#"PBA:{"generation":4,"action":"resume-direct","success":true}"#));
    assert!(shared.send(r#"CAP:{"Text":"resumed"}"#));
    assert!(
        shared
            .state()
            .outgoing
            .back()
            .unwrap()
            .1
            .starts_with("PBC:")
    );
}

#[test]
fn playback_idle_stops_caption_forwarding_until_reset() {
    let mut shared = test_shared();
    shared.sibling = true;
    let command = serde_json::json!({"command":"PlaybackControl","message":serde_json::json!({"generation":5,"action":"idle"}).to_string()});
    assert!(shared.receive(command.to_string().as_bytes()));
    assert_eq!(shared.state().events.back().unwrap().service, 4);
    assert!(shared.send(r#"PBA:{"generation":5,"action":"idle","success":true}"#));
    assert!(!shared.state().playback_active);
    assert!(shared.send(r#"CAP:{"Text":"discard while idle"}"#));
    assert!(
        !shared
            .state()
            .outgoing
            .iter()
            .any(|(_, text)| text.starts_with("PBC:"))
    );
    let reset = serde_json::json!({"command":"PlaybackControl","message":serde_json::json!({"generation":6,"action":"reset"}).to_string()});
    assert!(shared.receive(reset.to_string().as_bytes()));
    assert!(shared.send(r#"PBA:{"generation":6}"#));
    assert!(shared.state().playback_active);
}

fn dsm_block(version: u8, component: u16) -> String {
    let bytes = [
        0x3c, 0xb0, 27, 0, 0, 0xc1, 0, 0, 0x11, 3, 0x10, 3, 0x10, 0, 0, 1, 0xff, 0, 0, 10, 0, 7,
        version, 0xff, 0, 0, 1, 2, 3, 4,
    ];
    format!(
        "DSM:{}",
        serde_json::json!({"Sid":101,"Onid":4,"Tsid":1,"Pid":256,"Components":[{"Pid":256,"ComponentTag":64,"DataComponentId":component}],"Data":crate::dsm::hex(&bytes)})
    )
}

#[test]
fn bml_transfer_flag_keeps_logo_and_caption_transport() {
    let mut shared = test_shared();
    shared.bml_enabled = false;
    assert!(shared.send(&dsm_block(1, 7)));
    assert!(shared.send("DSM:{\"PCRBase\":123}"));
    assert!(shared.state().outgoing.is_empty());
    assert!(shared.send(&dsm_block(1, 9)));
    assert!(shared.send("CAP:{\"Text\":\"caption\"}"));
    assert_eq!(shared.state().outgoing.len(), 2);
    shared.bml_enabled = true;
    assert!(shared.send(&dsm_block(1, 7)));
    assert_eq!(shared.state().outgoing.len(), 3);
}

#[test]
fn dsm_logo_receipt_stops_only_acknowledged_module_version() {
    let shared = test_shared();
    let receipt = serde_json::json!({"sid":101,"onid":4,"tsid":1,"pid":256,"downloadId":0x10000001u32,"moduleId":7,"version":1});
    let command = serde_json::json!({"command":"DSMModuleReceived","message":receipt.to_string()})
        .to_string();
    assert!(shared.send(&dsm_block(1, 9)));
    shared.state().outgoing.clear();
    shared.state().outgoing_bytes = 0;
    assert!(shared.receive(command.as_bytes()));
    assert!(shared.send(&dsm_block(1, 9)));
    assert!(shared.state().outgoing.is_empty());
    assert!(shared.send(&dsm_block(2, 9)));
    assert_eq!(shared.state().outgoing.len(), 1);
    // An old version number used again after a new version must be delivered.
    assert!(shared.send(&dsm_block(1, 9)));
    assert_eq!(shared.state().outgoing.len(), 2);
    // Siblings cannot forge receipt information.
    let mut sibling = test_shared();
    sibling.sibling = true;
    assert!(sibling.receive(command.as_bytes()));
    assert!(sibling.state().received_dsm.is_empty());
}
