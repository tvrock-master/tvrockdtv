//! In-process replacement for the Go dtvclient WebSocket client.
//! One worker owns the socket. Host calls only touch short-held, bounded queues.
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, Request},
        protocol::WebSocketConfig,
    },
};

const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_QUEUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_QUEUE: usize = 4096;
const MAX_EVENTS: usize = 256;
const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: u32,
    pub channel: u32,
    pub service: u32,
    pub filename: String,
}
#[derive(Deserialize)]
struct RequestMessage {
    #[serde(default)]
    #[serde(rename = "deviceid")]
    _deviceid: String,
    command: String,
    #[serde(default)]
    message: String,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
struct DSMModuleKey {
    sid: u16,
    onid: u16,
    tsid: u16,
    pid: u16,
    download_id: u32,
    module_id: u16,
    version: u8,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct DSMTransfer {
    #[serde(default)]
    sid: u16,
    #[serde(default)]
    onid: u16,
    #[serde(default)]
    tsid: u16,
    #[serde(default)]
    pid: u16,
    #[serde(default)]
    data: String,
    #[serde(default)]
    components: Vec<DSMTransferComponent>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DSMTransferComponent {
    pid: u16,
    component_tag: u8,
    #[serde(rename = "DataComponentId")]
    data_component_id: u16,
}
impl DSMTransfer {
    fn logo_candidate(&self) -> bool {
        !self.data.is_empty()
            && !self.data.starts_with("3d")
            && self.components.iter().any(|c| {
                c.pid == self.pid
                    && (c.component_tag == 255 || !matches!(c.data_component_id, 7 | 11 | 12 | 13))
            })
    }
    fn module_key(&self) -> Option<DSMModuleKey> {
        // Only DDB blocks can be suppressed. DII announcements must still
        // describe new module versions and carousel changes.
        let raw: Vec<u8> = self
            .data
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .take(280)
            .map(|p| {
                std::str::from_utf8(p)
                    .ok()
                    .and_then(|p| u8::from_str_radix(p, 16).ok())
            })
            .collect::<Option<_>>()?;
        if raw.len() < 26
            || raw[0] != 0x3c
            || raw[8] != 0x11
            || raw[9] != 3
            || raw[10..12] != [0x10, 3]
        {
            return None;
        }
        let body = 20 + raw[17] as usize;
        if raw.len() < body + 6 {
            return None;
        }
        Some(DSMModuleKey {
            sid: self.sid,
            onid: self.onid,
            tsid: self.tsid,
            pid: self.pid,
            download_id: u32::from_be_bytes(raw[12..16].try_into().ok()?),
            module_id: u16::from_be_bytes(raw[body..body + 2].try_into().ok()?),
            version: raw[body + 2],
        })
    }
}

#[derive(Default)]
struct State {
    connected: bool,
    ready: bool,
    stopped: bool,
    generation: u64,
    tuner: Option<u32>,
    services: HashSet<u16>,
    events: VecDeque<Event>,
    outgoing: VecDeque<(Option<String>, String)>,
    outgoing_bytes: usize,
    cache: BTreeMap<String, String>,
    cache_bytes: usize,
    last_error: String,
    dropped: u64,
    poweroff: bool,
    playback_generation: u32,
    playback_active: bool,
    received_dsm: HashSet<DSMModuleKey>,
}
struct Shared {
    sibling: bool,
    bml_enabled: bool,
    state: Mutex<State>,
    changed: Condvar,
    outgoing: Notify,
    stop: watch::Sender<bool>,
}
impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn error(&self, message: &str) {
        let mut s = self.state();
        if s.last_error != message {
            s.last_error = message.into();
            crate::windows::debug(message);
        }
    }
    fn disconnected(&self) {
        let mut s = self.state();
        s.playback_active = false;
        s.playback_generation = 0;
        s.connected = false;
        s.ready = false;
        s.tuner = None;
        s.services.clear();
        s.outgoing.clear();
        s.outgoing_bytes = 0;
        self.changed.notify_all();
    }
    fn send(&self, text: &str) -> bool {
        if text.len() > MAX_MESSAGE || text.contains('\0') {
            return false;
        }
        let dsm = text
            .strip_prefix("DSM:")
            .and_then(|text| serde_json::from_str::<DSMTransfer>(text).ok());
        let mut s = self.state();
        if s.stopped {
            return false;
        }
        if !self.bml_enabled
            && text.starts_with("DSM:")
            && !dsm.as_ref().is_some_and(DSMTransfer::logo_candidate)
        {
            return true;
        }
        if let Some(key) = dsm.as_ref().and_then(DSMTransfer::module_key) {
            // Keep only the current acknowledged version of a module family,
            // so version-number rollover cannot suppress a later update.
            s.received_dsm.retain(|old| {
                old.sid != key.sid
                    || old.onid != key.onid
                    || old.tsid != key.tsid
                    || old.pid != key.pid
                    || old.module_id != key.module_id
                    || old == &key
            });
            if s.received_dsm.contains(&key) {
                return true;
            }
        }
        // Playback captions have their own non-persistent protocol. A receiver
        // cannot enable it: only an authenticated server control can do so.
        let transformed;
        let text = if self.sibling && text.starts_with("CAP:") && s.playback_active {
            let Ok(caption) = serde_json::from_str::<serde_json::Value>(&text[4..]) else {
                return false;
            };
            transformed = format!(
                "PBC:{}",
                serde_json::json!({"generation":s.playback_generation,"caption":caption})
            );
            transformed.as_str()
        } else {
            text
        };
        if self.sibling && text.starts_with("PBA:") {
            let Ok(ack) = serde_json::from_str::<serde_json::Value>(&text[4..]) else {
                return false;
            };
            if s.playback_generation == 0
                || ack["generation"].as_u64() != Some(s.playback_generation as u64)
            {
                return false;
            }
            s.playback_active =
                ack["success"].as_bool().unwrap_or(true) && ack["action"].as_str() != Some("idle");
        }
        if self.sibling
            && !text.starts_with("RCV:")
            && !text.starts_with("PBA:")
            && !(text.starts_with("PBC:") && s.playback_active)
        {
            return true;
        }
        let key = cache_key(text);
        if let Some(key) = &key {
            // Discard cached sections belonging to an older SDT version of this TS.
            if key.starts_with("SDT:") {
                let parts = key.split(':').collect::<Vec<_>>();
                if parts.len() == 6 {
                    let prefix = parts[..4].join(":") + ":";
                    let version = parts[..5].join(":") + ":";
                    s.cache
                        .retain(|k, _| !k.starts_with(&prefix) || k.starts_with(&version));
                }
            }
            if key.starts_with("SDT:") {
                // RunningStatus changes do not announce a new section, but OneSeg is
                // derived from NIT and may resolve after the initial SDT was sent.
                if s.cache.get(key).is_some_and(|old| {
                    sdt_service_classification(old) == sdt_service_classification(text)
                }) {
                    return true;
                }
            } else if s.cache.get(key).is_some_and(|old_text| old_text == text) {
                return true;
            }
            let old = s.cache.get(key).map_or(0, String::len);
            s.cache_bytes = s.cache.values().map(String::len).sum();
            if s.cache_bytes - old + text.len() > MAX_CACHE_BYTES
                || (s.cache.len() >= 518 && !s.cache.contains_key(key))
            {
                s.dropped += 1;
                return false;
            }
            s.cache.insert(key.clone(), text.into());
            s.cache_bytes = s.cache_bytes - old + text.len();
        }
        // CDT cache is a durable pending queue, drained by the worker only once.
        if key.as_ref().is_some_and(|k| k.starts_with("CDT:")) {
            drop(s);
            self.outgoing.notify_one();
            return true;
        }
        if !s.ready && !(s.connected && text.starts_with("CH2:")) {
            return key.is_some();
        }
        let accepted = enqueue(&mut s, key, text.to_owned());
        drop(s);
        if accepted {
            self.outgoing.notify_one()
        }
        accepted
    }
    fn receive(&self, bytes: &[u8]) -> bool {
        let Ok(r) = serde_json::from_slice::<RequestMessage>(bytes) else {
            self.error("Invalid WebSocket command JSON");
            return true;
        };
        // The server routes this connection by RegistryCode; deviceid is its separate display ID.
        if self.sibling
            && !matches!(
                r.command.as_str(),
                "PowerOff" | "Capture" | "ReceiverService" | "PlaybackControl"
            )
        {
            return true;
        }
        if r.command == "DSMModuleReceived" {
            if let Ok(key) = serde_json::from_str::<DSMModuleKey>(&r.message)
                && key.sid != 0
                && key.pid < 8191
            {
                let mut s = self.state();
                if s.received_dsm.len() < 4096 || s.received_dsm.contains(&key) {
                    s.received_dsm.insert(key);
                }
            }
            return true;
        }
        if r.command == "PlaybackControl" {
            if !self.sibling {
                return true;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&r.message) else {
                return true;
            };
            let Some(generation) = value["generation"]
                .as_u64()
                .filter(|g| *g > 0 && *g <= u32::MAX as u64)
            else {
                return true;
            };
            let action = match value["action"].as_str() {
                Some("reset") => 0,
                Some("pause") => 1,
                Some("pause-direct") => 2,
                Some("resume-direct") => 3,
                Some("idle") => 4,
                _ => return true,
            };
            let mut s = self.state();
            s.playback_active = false;
            s.playback_generation = generation as u32;
            s.outgoing
                .retain(|(_, text)| !text.starts_with("PBC:") && !text.starts_with("PBA:"));
            s.outgoing_bytes = s.outgoing.iter().map(|(_, text)| text.len()).sum();
            if s.events.len() >= MAX_EVENTS {
                return false;
            }
            s.events.push_back(Event {
                kind: 0x26,
                channel: generation as u32,
                service: action,
                ..Event::default()
            });
            self.changed.notify_all();
            return true;
        }
        match r.command.as_str() {
            "Ready" => {
                let values = split_uints(&r.message);
                let Some(&tuner) = values.first() else {
                    self.error("Invalid Ready message");
                    return true;
                };
                let mut s = self.state();
                s.tuner = Some(tuner);
                s.services = values[1..]
                    .iter()
                    .filter_map(|v| u16::try_from(*v).ok())
                    .collect();
                s.ready = true;
                s.generation = s.generation.saturating_add(1);
                let mut replay = Vec::new();
                for prefix in ["DIF:", "STA:", "STC:", "CPF:", "SDT:"] {
                    replay.extend(
                        s.cache
                            .iter()
                            .filter(|(k, _)| k.starts_with(prefix))
                            .map(|(k, v)| (Some(k.clone()), v.clone())),
                    );
                }
                for (k, v) in replay {
                    enqueue(&mut s, k, v);
                }
                drop(s);
                self.changed.notify_all();
                self.outgoing.notify_one();
            }
            "LoopBack" => {
                let message = if r.message.starts_with("LTC:") {
                    format!("{},{}", r.message, env!("TVROCKDTV_VERSION"))
                } else {
                    r.message
                };
                let mut state = self.state();
                enqueue(&mut state, None, message);
                drop(state);
                self.outgoing.notify_one();
            }
            command => {
                let kind = match command {
                    "RecordStart" => 0x10,
                    "RecordStop" => 0x11,
                    "QuickRecord" | "RecordStartQuick" => 0x12,
                    "PowerOff" => 0x13,
                    "StreamChange" => 0x14,
                    "Capture" => 0x15,
                    "OnOption" => 0x17,
                    "StreamStart" => 0x20,
                    "StreamStartSRT" => 0x25,
                    "StreamStop" => 0x21,
                    "GetStreamStatus" => 0x22,
                    "ReceiverService" => 0x23,
                    "StreamPace" => 0x24,
                    _ => return true,
                };
                let mut ev = Event {
                    kind,
                    ..Event::default()
                };
                if matches!(ev.kind, 0x10 | 0x12) {
                    if r.message.is_empty()
                        || r.message.contains('\0')
                        || r.message.encode_utf16().count() >= 32768
                    {
                        self.error("Invalid recording filename");
                        return true;
                    }
                    ev.filename = r.message.clone();
                }
                if ev.kind == 0x23 {
                    let Some(sid) = parse_uint(&r.message).filter(|v| *v > 0 && *v <= 65535) else {
                        return true;
                    };
                    ev.service = sid;
                }
                if ev.kind == 0x14 {
                    let Some(value) = parse_uint(&r.message) else {
                        self.error("Invalid StreamChange message");
                        return true;
                    };
                    ev.channel = value >> 16;
                    ev.service = value & 65535;
                }
                if matches!(ev.kind, 0x20..=0x22 | 0x24 | 0x25) {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.message) else {
                        self.error("Invalid stream request");
                        return true;
                    };
                    let Some(session) = v["session"]
                        .as_str()
                        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                    else {
                        self.error("Invalid stream session");
                        return true;
                    };
                    ev.filename = session.to_owned();
                    if matches!(ev.kind, 0x20 | 0x25) {
                        let Some(ip) = v["address"]
                            .as_str()
                            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
                        else {
                            self.error("Invalid stream address");
                            return true;
                        };
                        let Some(port) = v["port"].as_u64().filter(|p| (1..=65535).contains(p))
                        else {
                            self.error("Invalid stream port");
                            return true;
                        };
                        if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                            self.error("Invalid unicast stream address");
                            return true;
                        }
                        ev.filename.push(',');
                        ev.filename.push_str(&ip.to_string());
                        if let Some(value) = v.get("flags") {
                            let Some(mut flags) = value.as_u64().filter(|n| *n <= 15) else {
                                self.error("Invalid stream flags");
                                return true;
                            };
                            if flags & 8 != 0 {
                                flags |= 1;
                            }
                            ev.filename.push(',');
                            ev.filename.push_str(&flags.to_string());
                        }
                        ev.service = port as u32;
                        let lease = v.get("leaseSeconds").map_or(Some(15), |value| {
                            value.as_u64().filter(|n| (1..=86400).contains(n))
                        });
                        let Some(lease) = lease else {
                            self.error("Invalid stream lease");
                            return true;
                        };
                        ev.channel = lease as u32;
                        if ev.kind == 0x25 {
                            let Some(latency) = v["latencyMs"].as_u64().filter(|n| *n <= 60000)
                            else {
                                self.error("Invalid SRT latency");
                                return true;
                            };
                            let Some(secret) = v["passphrase"].as_str() else {
                                self.error("Invalid SRT passphrase");
                                return true;
                            };
                            if !(crate::srt::Options {
                                latency_ms: latency as u32,
                                passphrase: secret.to_owned(),
                            })
                            .valid()
                            {
                                self.error("Invalid SRT options");
                                return true;
                            }
                            // Hex encodes the secret so delimiters cannot alter the event layout.
                            let encoded: String =
                                secret.bytes().map(|b| format!("{b:02x}")).collect();
                            ev.filename = format!(
                                "{session},{ip},{},{latency},{encoded}",
                                v.get("flags").and_then(|f| f.as_u64()).unwrap_or(7)
                            );
                        }
                    }
                }
                let poweroff = ev.kind == 0x13;
                let mut s = self.state();
                if s.events.len() == MAX_EVENTS {
                    drop(s);
                    self.error("Remote command queue is full; reconnecting");
                    return false;
                }
                s.poweroff |= poweroff;
                s.events.push_back(ev);
                drop(s);
                self.changed.notify_all();
                if poweroff {
                    return false;
                }
            }
        }
        true
    }
}
fn enqueue(s: &mut State, key: Option<String>, text: String) -> bool {
    if let Some(k) = &key
        && let Some(pos) = s
            .outgoing
            .iter()
            .position(|(old, _)| old.as_ref() == Some(k))
    {
        let (_, old) = s.outgoing.remove(pos).unwrap();
        s.outgoing_bytes -= old.len();
    }
    if s.outgoing.len() >= MAX_QUEUE || s.outgoing_bytes + text.len() > MAX_QUEUE_BYTES {
        s.dropped += 1;
        return false;
    }
    s.outgoing_bytes += text.len();
    s.outgoing.push_back((key, text));
    true
}
fn sdt_service_classification(text: &str) -> Option<Vec<(u64, i64)>> {
    let services: serde_json::Value = serde_json::from_str(text.strip_prefix("SDT:")?).ok()?;
    services
        .as_array()?
        .iter()
        .map(|service| Some((service["Sid"].as_u64()?, service["OneSeg"].as_i64()?)))
        .collect()
}
fn cache_key(text: &str) -> Option<String> {
    if let Some(json) = text.strip_prefix("CDT:") {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        return Some(format!(
            "CDT:{}:{}:{}",
            v["Onid"].as_u64()?,
            v["Sid"].as_u64()?,
            v["LogoType"].as_u64()?
        ));
    }
    for prefix in ["CH2:", "DIF:", "STA:", "CPF:", "STC:"] {
        if text.starts_with(prefix) {
            return Some(prefix.into());
        }
    }
    if let Some(json) = text.strip_prefix("SDT:") {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let first = v.as_array()?.first()?;
        let mut key = "SDT".to_string();
        for f in ["TableId", "Tsid", "Onid", "Version", "Section"] {
            key.push(':');
            key.push_str(&first.get(f)?.as_u64()?.to_string())
        }
        return Some(key);
    }
    None
}
fn parse_uint(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}
pub fn split_uints(s: &str) -> Vec<u32> {
    s.split(',').filter_map(parse_uint).collect()
}
pub fn websocket_url(server: &str) -> Result<url::Url, String> {
    let input = if server.contains("://") {
        server.to_owned()
    } else {
        format!("ws://{server}")
    };
    let mut url = url::Url::parse(&input).map_err(|_| "Invalid device server URL")?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        _ => return Err("Unsupported device server URL scheme".into()),
    };
    url.set_scheme(scheme).map_err(|_| "Invalid URL scheme")?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err("Device server must have a host and no URL credentials".into());
    }
    url.set_path("/dev");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}
fn request(server: &str, device: &str, parent: Option<&str>) -> Result<Request<()>, String> {
    let url = websocket_url(server)?;
    let mut r = url
        .as_str()
        .into_client_request()
        .map_err(|_| "Invalid WebSocket request")?;
    r.headers_mut().insert(
        "Content-Type",
        HeaderValue::from_static("application/json; charset=utf8"),
    );
    let agent = if let Some(parent) = parent {
        if parent.is_empty() {
            return Err("Parent registration code must not be empty".into());
        }
        r.headers_mut().insert(
            "X-TvRockDTV-Parent",
            HeaderValue::from_str(parent).map_err(|_| "Invalid parent registration code")?,
        );
        concat!("TvRockDTV-Sib/", env!("TVROCKDTV_VERSION"))
    } else {
        concat!("TvRockDTV/", env!("TVROCKDTV_VERSION"))
    };
    r.headers_mut().insert(
        "User-Agent",
        HeaderValue::from_str(agent).map_err(|_| "Invalid client version")?,
    );
    r.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {device}"))
            .map_err(|_| "Invalid device identifier")?,
    );
    Ok(r)
}

pub struct Client {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Client {
    pub fn start(device: &str, server: &str) -> Result<Self, String> {
        Self::start_with_parent(device, server, None)
    }
    pub fn start_with_parent(
        device: &str,
        server: &str,
        parent: Option<&str>,
    ) -> Result<Self, String> {
        let request = request(server, device, parent)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .map_err(|e| e.to_string())?;
        let (stop, rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            sibling: parent.is_some(),
            bml_enabled: std::env::args().any(|arg| arg.eq_ignore_ascii_case("/bml")),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            outgoing: Notify::new(),
            stop,
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("tvrock-network".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.block_on(run(worker_shared.clone(), rx, request))
                }));
                worker_shared.disconnected();
                worker_shared.state().stopped = true;
                worker_shared.changed.notify_all();
                // Drop the runtime here: DNS/background work is joined before code can be unloaded.
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }
    pub fn stop(&self) {
        let mut s = self.shared.state();
        s.stopped = true;
        s.events.clear();
        drop(s);
        self.shared.stop.send_replace(true);
        self.shared.changed.notify_all();
    }
    pub fn send(&self, text: &str) -> bool {
        self.shared.send(text)
    }
    pub fn reset_service(&self) {
        let mut s = self.shared.state();
        s.cache.retain(|key, _| !key.starts_with("SDT:"));
        s.cache_bytes = s.cache.values().map(String::len).sum();
        s.outgoing
            .retain(|(key, _)| !key.as_ref().is_some_and(|key| key.starts_with("SDT:")));
        s.outgoing_bytes = s.outgoing.iter().map(|(_, text)| text.len()).sum();
    }
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let s = self.shared.state();
        let (s, _) = self
            .shared
            .changed
            .wait_timeout_while(s, timeout, |s| !s.ready && !s.stopped)
            .unwrap_or_else(|e| e.into_inner());
        s.ready && !s.stopped
    }
    pub fn next_event(&self, timeout: Duration) -> Option<Event> {
        let s = self.shared.state();
        let (mut s, _) = self
            .shared
            .changed
            .wait_timeout_while(s, timeout, |s| s.events.is_empty() && !s.stopped)
            .unwrap_or_else(|e| e.into_inner());
        s.events.pop_front()
    }
    pub fn stopped(&self) -> bool {
        self.shared.state().stopped
    }
    pub fn tuner_type(&self) -> u32 {
        self.shared.state().tuner.unwrap_or(255)
    }
    pub fn valid_service(&self, sid: u16) -> bool {
        let s = self.shared.state();
        s.ready && s.services.contains(&sid)
    }
    pub fn generation(&self) -> u64 {
        let s = self.shared.state();
        if s.ready { s.generation } else { 0 }
    }
    pub fn last_error(&self) -> String {
        self.shared.state().last_error.clone()
    }
    pub fn dropped_messages(&self) -> u64 {
        self.shared.state().dropped
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}

async fn run(shared: Arc<Shared>, mut stop: watch::Receiver<bool>, request: Request<()>) {
    while !*stop.borrow() {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let connection = tokio::select! {biased;_=stop.changed()=>break,r=tokio::time::timeout(IO_TIMEOUT,connect_async_with_config(request.clone(),Some(config),false))=>r};
        match connection {
            Ok(Ok((mut socket, _))) => {
                {
                    let mut s = shared.state();
                    s.connected = true;
                    if shared.sibling {
                        // Siblings do not send CH2 or wait for the normal Ready exchange.
                        s.ready = true;
                        s.generation = s.generation.saturating_add(1);
                    }
                    if let Some(ch2) = s.cache.get("CH2:").cloned() {
                        enqueue(&mut s, Some("CH2:".into()), ch2);
                    }
                }
                shared.changed.notify_all();
                let mut last_received = Instant::now();
                let mut ping = tokio::time::interval(Duration::from_secs(30));
                ping.tick().await;
                let mut poweroff = false;
                loop {
                    // Register wake-up before looking at the queue to avoid missing a producer.
                    let notify = shared.outgoing.notified();
                    let message = {
                        let mut s = shared.state();
                        let m = s.outgoing.pop_front();
                        if let Some((_, text)) = &m {
                            s.outgoing_bytes -= text.len()
                        }
                        m.or_else(|| {
                            if !s.ready {
                                return None;
                            }
                            s.cache
                                .iter()
                                .find(|(k, _)| k.starts_with("CDT:"))
                                .map(|(k, v)| (Some(k.clone()), v.clone()))
                        })
                    };
                    if let Some((key, text)) = message {
                        let sent = tokio::select! {biased;_=stop.changed()=>false,r=tokio::time::timeout(IO_TIMEOUT,socket.send(Message::Text(text.into())))=>matches!(r,Ok(Ok(())))};
                        if !sent {
                            shared.error("WebSocket write failed or timed out");
                            break;
                        }
                        if let Some(key) = key.filter(|k| k.starts_with("CDT:")) {
                            let mut s = shared.state();
                            if let Some(old) = s.cache.remove(&key) {
                                s.cache_bytes -= old.len();
                            }
                        }
                    }
                    // Yield to inbound commands after each write, even under continuous TS traffic.
                    let has_outgoing = {
                        let s = shared.state();
                        !s.outgoing.is_empty()
                            || (s.ready && s.cache.keys().any(|k| k.starts_with("CDT:")))
                    };
                    tokio::select! {biased;
                        _=stop.changed()=>break,
                        inbound=socket.next()=>{match inbound{
                            Some(Ok(Message::Text(text)))=>{last_received=Instant::now();if !shared.receive(text.as_bytes()){poweroff=shared.state().poweroff;break}},
                            Some(Ok(Message::Binary(bytes)))=>{last_received=Instant::now();if !shared.receive(&bytes){poweroff=shared.state().poweroff;break}},
                            Some(Ok(Message::Ping(_)))=>{last_received=Instant::now();let r=tokio::select!{biased;_=stop.changed()=>break,r=tokio::time::timeout(IO_TIMEOUT,socket.flush())=>r};if !matches!(r,Ok(Ok(()))){break}},
                            Some(Ok(Message::Pong(_)))=>last_received=Instant::now(),
                            Some(Ok(Message::Close(_)))|None=>break,
                            Some(Err(_))=>{shared.error("WebSocket receive failed");break},_=>()
                        }},
                        _=ping.tick()=>{if last_received.elapsed()>Duration::from_secs(90){shared.error("WebSocket peer timed out");break}let r=tokio::select!{biased;_=stop.changed()=>break,r=tokio::time::timeout(IO_TIMEOUT,socket.send(Message::Ping(Vec::new().into())))=>r};if !matches!(r,Ok(Ok(()))){break}},
                        _=notify=>(),
                        _=tokio::task::yield_now(),if has_outgoing=>(),
                    }
                    if *stop.borrow() {
                        break;
                    }
                }
                let _ = tokio::time::timeout(Duration::from_millis(250), socket.close(None)).await;
                shared.disconnected();
                if poweroff {
                    break;
                }
            }
            _ => shared.error("Device server connection failed or timed out"),
        }
        if *stop.borrow() {
            break;
        }
        tokio::select! {biased;_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>()}
    }
}

#[cfg(test)]
#[path = "../tests/unit/dtvclient.rs"]
mod tests;
