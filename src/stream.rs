//! Rust-owned SRT/UDP senders. The input callback only queues bytes; workers filter
//! and send without holding recording, application or SI-analysis locks.
use crate::stream_filter::Filter;
use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering::SeqCst},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const BATCH: usize = 128 * 188;
const SMALL_BATCH: usize = 7 * 188;
const LIMIT: usize = 8 * 1024 * 1024;
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Status {
    pub enabled: u32,
    pub address: [u8; 16],
    pub port: u16,
    pub service: u16,
    pub flags: u32,
    pub bytes: u64,
    pub last_tick: u64,
    pub error: u32,
}
#[cfg(windows)]
mod platform {
    use super::*;
    use std::{ffi::c_void, os::windows::io::AsRawSocket};
    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn setsockopt(socket: usize, level: i32, name: i32, value: *const u8, length: i32) -> i32;
        fn WSAGetLastError() -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn GetLastError() -> u32;
        fn GetTickCount64() -> u64;
        fn CreateWaitableTimerExW(
            attributes: *const c_void,
            name: *const u16,
            flags: u32,
            access: u32,
        ) -> *mut c_void;
        fn SetWaitableTimer(
            timer: *mut c_void,
            due: *const i64,
            period: i32,
            completion: *const c_void,
            argument: *const c_void,
            resume: i32,
        ) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    }
    pub fn tick() -> u64 {
        unsafe { GetTickCount64() }
    }
    pub fn buffer(s: &UdpSocket) -> io::Result<()> {
        let bytes: i32 = 4 * 1024 * 1024;
        if unsafe {
            setsockopt(
                s.as_raw_socket() as usize,
                0xffff,
                0x1001,
                (&bytes as *const i32).cast(),
                4,
            )
        } != 0
        {
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
        }
        Ok(())
    }
    // Reuse a high-resolution timer instead of a coarse Sleep per datagram.
    pub struct PaceTimer(*mut c_void);
    impl PaceTimer {
        pub fn new() -> Self {
            let mut timer =
                unsafe { CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), 2, 0x1f0003) };
            if timer.is_null() {
                timer = unsafe {
                    CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), 0, 0x1f0003)
                }
            }
            Self(timer)
        }
        pub fn wait(&self, duration: Duration) {
            let due = -(duration.as_nanos().div_ceil(100).max(1) as i64);
            if !self.0.is_null()
                && unsafe {
                    SetWaitableTimer(self.0, &due, 0, std::ptr::null(), std::ptr::null(), 0)
                } != 0
            {
                unsafe { WaitForSingleObject(self.0, 1000) };
            } else {
                thread::sleep(duration)
            }
        }
    }
    impl Drop for PaceTimer {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }
    pub struct Reservation(usize);
    impl Reservation {
        pub fn new(address: SocketAddrV4) -> io::Result<Self> {
            let name = format!(
                "Global\\EpgDataCap_Bon_UDP_PORT_{}_{}",
                i32::from_ne_bytes(address.ip().octets()),
                address.port()
            );
            Self::named(&name)
        }
        pub fn port(port: u16) -> io::Result<Self> {
            Self::named(&format!("RecTest Network Port {port}"))
        }
        fn named(name: &str) -> io::Result<Self> {
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            let error = unsafe { GetLastError() };
            if h.is_null() {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            if error == 183 {
                unsafe { CloseHandle(h) };
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "UDP destination is reserved",
                ));
            }
            Ok(Self(h as usize))
        }
    }
    impl Drop for Reservation {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0 as *mut c_void) };
        }
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn tick() -> u64 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
    }
    pub fn buffer(_: &UdpSocket) -> io::Result<()> {
        Ok(())
    }
    pub struct PaceTimer;
    impl PaceTimer {
        pub fn new() -> Self {
            Self
        }
        pub fn wait(&self, duration: Duration) {
            thread::sleep(duration)
        }
    }
    pub struct Reservation;
    impl Reservation {
        pub fn port(_: u16) -> io::Result<Self> {
            Ok(Self)
        }
        pub fn new(_: SocketAddrV4) -> io::Result<Self> {
            Ok(Self)
        }
    }
}
struct Queue {
    bytes: Vec<u8>,
    service: u16,
    generation: u64,
}
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    stop: AtomicBool,
    enabled: AtomicBool,
    generation: AtomicU64,
    bytes: AtomicU64,
    tick: AtomicU64,
    error: AtomicU32,
    service: AtomicU16,
    incoming: AtomicU64,
    paced: AtomicBool,
}
impl Shared {
    fn set_error(&self, generation: u64, error: u32) {
        let q = lock(&self.queue);
        if q.generation == generation {
            self.error.store(error, SeqCst);
        }
    }
    fn select(&self, generation: u64, service: u16) {
        let q = lock(&self.queue);
        if q.generation == generation {
            self.service.store(service, SeqCst);
            if service != 0 {
                self.error.store(0, SeqCst);
            }
        }
    }
}
struct Sender {
    endpoint: SocketAddrV4,
    flags: u32,
    srt: Option<crate::srt::Options>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    _reservation: platform::Reservation,
    _port_reservation: Option<Arc<platform::Reservation>>,
}
impl Sender {
    fn new(
        endpoint: SocketAddrV4,
        service: u16,
        flags: u32,
        port_reservation: Option<Arc<platform::Reservation>>,
        srt: Option<crate::srt::Options>,
    ) -> io::Result<Self> {
        let reservation = platform::Reservation::new(endpoint)?;
        let socket = if let Some(options) = &srt {
            Transport::Srt(crate::srt::Socket::open(endpoint, options)?)
        } else {
            let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
            platform::buffer(&socket)?;
            socket.set_nonblocking(true)?;
            socket.connect(endpoint)?;
            Transport::Udp(socket)
        };
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                bytes: Vec::new(),
                service,
                generation: 0,
            }),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            enabled: AtomicBool::new(true),
            generation: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            error: AtomicU32::new(0),
            service: AtomicU16::new(if flags & 8 != 0 { 0 } else { service }),
            incoming: AtomicU64::new(0),
            paced: AtomicBool::new(false),
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("tvrock-udp".into())
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(socket, &state, flags)
                }))
                .is_err()
                {
                    state.error.store(8, SeqCst);
                }
                state.enabled.store(false, SeqCst);
            })?;
        Ok(Self {
            endpoint,
            flags,
            srt,
            shared,
            worker: Some(worker),
            _reservation: reservation,
            _port_reservation: port_reservation,
        })
    }
    fn reset(&self, service: u16) {
        let mut q = lock(&self.shared.queue);
        q.bytes.clear();
        q.service = service;
        q.generation = q.generation.wrapping_add(1);
        self.shared.generation.store(q.generation, SeqCst);
        self.shared
            .service
            .store(if self.flags & 8 != 0 { 0 } else { service }, SeqCst);
        self.shared.wake.notify_one();
    }
    fn feed(&self, bytes: &[u8]) -> bool {
        if !self.shared.enabled.load(SeqCst) {
            return false;
        }
        let mut q = lock(&self.shared.queue);
        if bytes.len() > LIMIT || q.bytes.len() > LIMIT - bytes.len() {
            q.bytes.clear();
            q.generation = q.generation.wrapping_add(1);
            self.shared.generation.store(q.generation, SeqCst);
            self.shared.error.store(10055, SeqCst);
            self.shared.wake.notify_one();
            return false;
        }
        self.shared.incoming.fetch_add(bytes.len() as u64, SeqCst);
        q.bytes.extend_from_slice(bytes);
        if q.bytes.len() >= BATCH {
            self.shared.wake.notify_one()
        }
        true
    }
    fn status(&self) -> Status {
        let mut out = Status {
            enabled: self.shared.enabled.load(SeqCst) as u32,
            port: self.endpoint.port(),
            service: self.shared.service.load(SeqCst),
            flags: self.flags,
            bytes: self.shared.bytes.load(SeqCst),
            last_tick: self.shared.tick.load(SeqCst),
            error: self.shared.error.load(SeqCst),
            ..Status::default()
        };
        let ip = self.endpoint.ip().to_string();
        out.address[..ip.len()].copy_from_slice(ip.as_bytes());
        out
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.shared.stop.store(true, SeqCst);
        self.shared.wake.notify_one();
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
// Measure arrival before the worker so pacing cannot limit its own estimate.
// The selected/raw ratio accounts for service and one-seg filtering.
struct Pacing {
    sample: Instant,
    incoming: u64,
    raw: u64,
    selected: u64,
    rate: f64,
    ratio: f64,
    credit: f64,
    refill: Instant,
    timer: platform::PaceTimer,
}
impl Pacing {
    fn new(incoming: u64) -> Self {
        Self {
            sample: Instant::now(),
            incoming,
            raw: 0,
            selected: 0,
            rate: 0.0,
            ratio: 1.0,
            credit: 0.0,
            refill: Instant::now(),
            timer: platform::PaceTimer::new(),
        }
    }
    fn sample(&mut self, s: &Shared) {
        let elapsed = self.sample.elapsed().as_secs_f64();
        if elapsed < 0.05 {
            return;
        }
        let incoming = s.incoming.load(SeqCst);
        let rate = incoming.saturating_sub(self.incoming) as f64 / elapsed;
        self.rate = if rate > self.rate {
            rate
        } else {
            self.rate * 0.8 + rate * 0.2
        };
        if self.raw > 0 {
            self.ratio = (self.selected as f64 / self.raw as f64).clamp(0.0, 1.0)
        }
        self.raw = 0;
        self.selected = 0;
        self.sample = Instant::now();
        self.incoming = incoming;
    }
    fn permit(&mut self, s: &Shared, generation: u64, bytes: usize, backlog: usize) -> bool {
        // Two milliseconds of credit amortize waits. Backlog adds a drain rate
        // without a fixed bitrate ceiling, keeping latency bounded on rate jumps.
        let base = self.rate * self.ratio;
        let rate = (base * 1.10 + backlog as f64 * self.ratio / 0.020).max(131600.0);
        loop {
            if s.stop.load(SeqCst) || s.generation.load(SeqCst) != generation {
                return false;
            }
            if self.rate == 0.0 {
                return true;
            }
            self.credit = (self.credit + self.refill.elapsed().as_secs_f64() * rate)
                .min((rate * 0.002).max(SMALL_BATCH as f64));
            self.refill = Instant::now();
            if self.credit >= bytes as f64 {
                self.credit -= bytes as f64;
                return true;
            }
            // Refill roughly one millisecond at a time instead of waking for
            // every small datagram at high rates. Low rates may wait up to 10 ms.
            let delay = ((bytes as f64 - self.credit) / rate).clamp(0.001, 0.010);
            self.timer.wait(Duration::from_secs_f64(delay));
        }
    }
}
enum Transport {
    Udp(UdpSocket),
    Srt(crate::srt::Socket),
}
impl Transport {
    fn send(&self, data: &[u8]) -> io::Result<usize> {
        match self {
            Self::Udp(socket) => socket.send(data),
            Self::Srt(socket) => socket.send(data),
        }
    }
}
fn flush(
    socket: &Transport,
    s: &Shared,
    batch: &mut Vec<u8>,
    generation: u64,
    pacing: &mut Pacing,
    input_backlog: usize,
) {
    pacing.sample(s);
    let paced = s.paced.load(SeqCst);
    let size = if paced || matches!(socket, Transport::Srt(_)) {
        SMALL_BATCH
    } else {
        BATCH
    };
    let backlog = if paced {
        lock(&s.queue).bytes.len() + input_backlog
    } else {
        0
    };
    for data in batch.chunks(size) {
        if s.stop.load(SeqCst) || s.generation.load(SeqCst) != generation {
            break;
        }
        if paced && !pacing.permit(s, generation, data.len(), backlog) {
            break;
        }
        let started = Instant::now();
        let result = loop {
            let result = socket.send(data);
            if matches!(socket, Transport::Srt(_))
                && result
                    .as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::WouldBlock)
                && started.elapsed() < Duration::from_millis(100)
                && !s.stop.load(SeqCst)
                && s.generation.load(SeqCst) == generation
            {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            break result;
        };
        match result {
            Ok(n) => {
                s.bytes.fetch_add(n as u64, SeqCst);
                s.tick.store(platform::tick(), SeqCst);
                s.set_error(generation, 0);
            }
            Err(e) => {
                s.set_error(generation, e.raw_os_error().unwrap_or(31) as u32);
            }
        }
    }
    batch.clear();
}
fn run(socket: Transport, s: &Shared, flags: u32) {
    let mut generation = u64::MAX;
    let mut filter = Filter::new(0, flags);
    let mut selected = 0;
    let mut input = Vec::new();
    let mut partial = Vec::new();
    let mut batch = Vec::with_capacity(BATCH);
    let mut pending = Instant::now();
    let mut selection_started = Instant::now();
    let mut pacing = Pacing::new(s.incoming.load(SeqCst));
    while !s.stop.load(SeqCst) {
        if let Transport::Srt(socket) = &socket {
            if socket.broken() {
                s.error.store(10054, SeqCst);
                break;
            }
            if !socket.ready() {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
        }
        let (service, next) = {
            let mut q = lock(&s.queue);
            if q.bytes.len() < BATCH && q.generation == generation && !s.stop.load(SeqCst) {
                q = s
                    .wake
                    .wait_timeout(q, Duration::from_millis(1))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            input.clear();
            std::mem::swap(&mut input, &mut q.bytes);
            (q.service, q.generation)
        };
        if s.stop.load(SeqCst) {
            break;
        }
        if generation != next {
            generation = next;
            filter = Filter::new(service, flags);
            pacing = Pacing::new(s.incoming.load(SeqCst));
            batch.clear();
            partial.clear();
            selected = filter.selected();
            selection_started = Instant::now();
        }
        partial.extend_from_slice(&input);
        let mut pos = 0;
        while partial.len() - pos >= 188 {
            if s.stop.load(SeqCst) || s.generation.load(SeqCst) != generation {
                break;
            }
            if partial[pos] != 0x47 {
                pos += 1;
                continue;
            }
            let packet: &[u8; 188] = partial[pos..pos + 188].try_into().unwrap();
            pos += 188;
            pacing.raw += 188;
            let output = filter.packet(packet);
            if filter.selected() != selected {
                selected = filter.selected();
                s.select(generation, selected);
                batch.clear();
            }
            if let Some(p) = output {
                if batch.is_empty() {
                    pending = Instant::now()
                }
                pacing.selected += 188;
                batch.extend_from_slice(&p);
                if batch.len() == BATCH {
                    flush(
                        &socket,
                        s,
                        &mut batch,
                        generation,
                        &mut pacing,
                        partial.len() - pos,
                    )
                }
            }
        }
        partial.drain(..pos);
        if flags & 8 != 0 && selected == 0 && selection_started.elapsed() >= Duration::from_secs(2)
        {
            s.set_error(generation, 1168);
        }
        if !batch.is_empty() && pending.elapsed() >= Duration::from_millis(10) {
            flush(&socket, s, &mut batch, generation, &mut pacing, 0)
        }
    }
}
#[derive(Default)]
struct Inner {
    service: u16,
    senders: BTreeMap<String, Sender>,
    ports: BTreeMap<u16, std::sync::Weak<platform::Reservation>>,
}
#[derive(Default)]
pub struct Streamer {
    inner: Mutex<Inner>,
    recorder_port_lock: bool,
}
impl Streamer {
    pub fn new(recorder_port_lock: bool) -> Self {
        Self {
            recorder_port_lock,
            ..Self::default()
        }
    }
    fn token(token: &str) -> bool {
        token.is_empty()
            || (token.len() == 64
                && token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    }
    pub fn start(&self, token: &str, address: &str, port: u16, flags: u32) -> bool {
        self.start_transport(token, address, port, flags, None)
    }
    pub fn start_transport(
        &self,
        token: &str,
        address: &str,
        port: u16,
        mut flags: u32,
        srt: Option<crate::srt::Options>,
    ) -> bool {
        if srt.as_ref().is_some_and(|options| !options.valid()) {
            return false;
        }
        if !Self::token(token) || port == 0 || flags & !15 != 0 {
            return false;
        }
        let Ok(ip) = address.parse::<Ipv4Addr>() else {
            return false;
        };
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip.octets()[0] == 0
            || ip.octets()[0] >= 224
        {
            return false;
        }
        if flags & 8 != 0 {
            flags |= 1
        }
        let endpoint = SocketAddrV4::new(ip, port);
        let mut inner = lock(&self.inner);
        if let Some(s) = inner.senders.get(token)
            && s.endpoint == endpoint
            && s.flags == flags
            && s.srt == srt
            && s.shared.enabled.load(SeqCst)
        {
            return true;
        }
        if inner
            .senders
            .iter()
            .any(|(key, s)| key != token && s.endpoint == endpoint)
            || (inner.senders.len() >= 32 && !inner.senders.contains_key(token))
        {
            return false;
        }
        inner.senders.remove(token);
        inner.ports.retain(|_, guard| guard.strong_count() > 0);
        let port_guard = if self.recorder_port_lock && srt.is_none() {
            let old = inner.ports.get(&port).and_then(std::sync::Weak::upgrade);
            let guard = match old {
                Some(guard) => guard,
                None => match platform::Reservation::port(port) {
                    Ok(guard) => Arc::new(guard),
                    Err(_) => return false,
                },
            };
            inner.ports.insert(port, Arc::downgrade(&guard));
            Some(guard)
        } else {
            None
        };
        match Sender::new(endpoint, inner.service, flags, port_guard, srt) {
            Ok(sender) => {
                inner.senders.insert(token.into(), sender);
                true
            }
            Err(_) => false,
        }
    }
    pub fn is_srt(&self, token: &str) -> bool {
        lock(&self.inner)
            .senders
            .get(token)
            .is_some_and(|s| s.srt.is_some())
    }
    pub fn pace(&self, token: &str) -> bool {
        if !Self::token(token) {
            return false;
        }
        let i = lock(&self.inner);
        let Some(sender) = i.senders.get(token) else {
            return false;
        };
        if sender.srt.is_some() || !sender.shared.enabled.load(SeqCst) {
            return false;
        }
        sender.shared.paced.store(true, SeqCst);
        sender.shared.wake.notify_one();
        true
    }
    pub fn is_paced(&self, token: &str) -> bool {
        Self::token(token)
            && lock(&self.inner)
                .senders
                .get(token)
                .is_some_and(|s| s.shared.paced.load(SeqCst))
    }
    pub fn stop(&self, token: &str) -> bool {
        if !Self::token(token) {
            return false;
        }
        lock(&self.inner).senders.remove(token);
        true
    }
    pub fn set_service(&self, service: u16) {
        let mut i = lock(&self.inner);
        if i.service == service {
            return;
        }
        i.service = service;
        for sender in i.senders.values() {
            sender.reset(service)
        }
    }
    pub fn reset(&self, service: u16) {
        let mut i = lock(&self.inner);
        i.service = service;
        for sender in i.senders.values() {
            sender.reset(service)
        }
    }
    pub fn feed(&self, bytes: &[u8]) -> bool {
        let i = lock(&self.inner);
        let mut ok = true;
        for sender in i.senders.values() {
            ok = sender.feed(bytes) && ok
        }
        ok
    }
    pub fn status(&self, token: &str) -> Option<Status> {
        Self::token(token).then(|| {
            lock(&self.inner)
                .senders
                .get(token)
                .map_or_else(Status::default, Sender::status)
        })
    }
}
