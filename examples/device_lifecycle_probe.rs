//! A simulated recorder for TvRockGO loopback lifecycle regression tests.
//! No DLL, tuner, recording file, or host application is opened.
use std::{
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};
use tvrockdtv::dtvclient::Client;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        3,
        "usage: device_lifecycle_probe <loopback-url> <test-device-code>"
    );
    let server = url::Url::parse(&args[1]).expect("server URL");
    assert!(
        matches!(
            server.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        ),
        "loopback only"
    );
    let client = Client::start(&args[2], &args[1]).expect("client start");
    assert!(client.send("CH2:;#SPACE(0,BS)"));
    assert!(client.send("STA:2"));
    assert!(client.send("STC:131072,101"));
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut generation = 0;
    while Instant::now() < deadline {
        let current = client.generation();
        if current != 0 && current != generation {
            generation = current;
            println!("ready {generation}");
            io::stdout().flush().unwrap();
        }
        if let Some(event) = client.next_event(Duration::from_millis(50)) {
            println!("event {} {} {}", event.kind, event.channel, event.service);
            io::stdout().flush().unwrap();
            match event.kind {
                0x10 => {
                    assert!(client.send("STA:3"));
                }
                0x11 => {
                    assert!(client.send("STA:2"));
                }
                0x14 => {
                    assert!(client.send(&format!("STC:{},{}", event.channel << 16, event.service)));
                }
                0x13 => {
                    let stop_deadline = Instant::now() + Duration::from_secs(3);
                    while !client.stopped() && Instant::now() < stop_deadline {
                        thread::sleep(Duration::from_millis(10));
                    }
                    assert!(client.stopped(), "PowerOff must stop reconnecting");
                    thread::sleep(Duration::from_millis(1200));
                    assert_eq!(client.generation(), 0);
                    println!("poweroff-stopped");
                    return;
                }
                _ => panic!("unexpected event: {event:?}"),
            }
        }
    }
    panic!("probe timed out: {}", client.last_error());
}
