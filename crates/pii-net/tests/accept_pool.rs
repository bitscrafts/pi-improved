//! Accept-pool tests (spec: bounded worker pool in `accept_loop`): with a
//! small pool of N workers, more than N concurrent connections are all
//! served (later ones wait), and the worker-thread count stays capped near
//! N under a burst — no thread-per-connection growth.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::undocumented_unsafe_blocks,
    // reason: this test file exercises the listener itself, so raw client
    // sockets are the unit under test's counterpart (like testing.rs for the
    // dial side); grants do not apply to the test's own loopback listener.
    clippy::disallowed_methods,
    clippy::redundant_closure
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use pii_abi::{HttpRequest, HttpResponse};
use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_net::limits::set_accept_pool_override;
use pii_net::listen::{HttpHandler, ListenSpec, Protocol};
use pii_net::service::{NetConfig, NetService};

/// Handler that records concurrent in-flight requests and answers after a
/// configurable delay, so saturation is observable.
struct Slow {
    delay: Duration,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl Slow {
    fn new(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        })
    }
}

impl HttpHandler for Slow {
    fn handle(&self, _req: &HttpRequest) -> HttpResponse {
        let n = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(n, Ordering::SeqCst);
        thread::sleep(self.delay);
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: b"ok".to_vec(),
        }
    }
}

use pii_net::listen::ListenerId;

/// Serializes pool-size (process-global) setup across tests.
static POOL_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn net_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

fn service_with(handler: Arc<dyn HttpHandler>) -> (NetService, std::net::SocketAddr, ListenerId) {
    let service = NetService::new(NetConfig {
        resolver: Arc::new(pii_net::resolve::StaticResolver::new()),
        grants: Arc::new(pii_net::grant::StaticGrants::default()),
        clock: Arc::new(pii_net::clock::FixedClock::new()),
    });
    let id = service
        .listen(
            &net_ctx(),
            ListenSpec::Tcp {
                addr: std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            },
            Protocol::Http,
            handler,
        )
        .expect("listen must succeed");
    let addr = service.listeners()[0].addr;
    (service, addr, id)
}

fn request(mut stream: TcpStream) -> u16 {
    let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: t\r\n\r\n");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).expect("read response");
    let head = String::from_utf8_lossy(&buf[..n]);
    head.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status line")
}

/// Count live worker threads by name via /proc (Linux container).
fn count_pii_net_conn_threads() -> usize {
    std::fs::read_dir("/proc")
        .expect("/proc")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .chars()
                .all(|c| c.is_ascii_digit())
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .filter(|c| c.starts_with("pii-net-conn"))
        .count()
}

/// TDD 1: N+2 concurrent connections against a pool of N are all served.
#[test]
fn accept_pool_serves_connections_within_bound() {
    let _guard = POOL_LOCK.lock();
    let n = 2usize;
    set_accept_pool_override(n);

    let handler = Slow::new(Duration::from_millis(100));
    let (_svc, addr, _id) = service_with(handler.clone());

    // Open N+2 connections and send before any request completes, so later
    // ones must queue.
    let mut streams: Vec<TcpStream> = (0..n + 2)
        .map(|_| TcpStream::connect(addr).expect("connect"))
        .collect();
    for s in &mut streams {
        let _ = s.write_all(b"GET / HTTP/1.1\r\nHost: t\r\n\r\n");
        let _ = s.flush();
    }

    let start = Instant::now();
    let statuses: Vec<u16> = streams
        .into_iter()
        .map(|s| thread::spawn(move || request(s)).join().expect("join"))
        .collect();
    assert!(
        start.elapsed() >= Duration::from_millis(100),
        "requests must actually be served"
    );
    assert!(
        statuses.iter().all(|&s| s == 200),
        "all {n}+2 connections served, got {statuses:?}"
    );
    assert!(
        handler.max_in_flight.load(Ordering::SeqCst) <= n,
        "concurrency must not exceed pool size {n}"
    );
}

/// TDD 2: a burst of M > N connections keeps live worker threads capped
/// near N instead of spawning one thread per connection.
#[test]
fn accept_pool_does_not_spawn_unbounded_threads() {
    let _guard = POOL_LOCK.lock();
    let n = 2usize;
    set_accept_pool_override(n);

    let handler = Slow::new(Duration::from_millis(200));
    let (_svc, addr, _id) = service_with(handler);

    let m = 12usize;
    let clients = (0..m).map(|_| {
        thread::spawn(move || {
            let mut s = TcpStream::connect(addr).expect("connect");
            let _ = s.write_all(b"GET / HTTP/1.1\r\nHost: t\r\n\r\n");
            let _ = s.flush();
            request(s)
        })
    });

    // Sample the live worker-thread count repeatedly during the burst.
    let mut max_seen = 0usize;
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        max_seen = max_seen.max(count_pii_net_conn_threads());
        thread::sleep(Duration::from_millis(20));
    }

    let statuses: Vec<u16> = clients
        .into_iter()
        .map(|c| c.join().expect("join"))
        .collect();
    assert!(
        statuses.iter().all(|&s| s == 200),
        "all served: {statuses:?}"
    );
    assert!(
        max_seen <= n,
        "live worker threads ({max_seen}) must stay capped at pool size {n}"
    );
}

/// Regression guard: `close()` still returns promptly and stops accepting.
#[test]
fn close_still_prompt_and_stops_accepting() {
    let _guard = POOL_LOCK.lock();
    set_accept_pool_override(4);
    let handler = Slow::new(Duration::from_millis(1));
    let (svc, addr, id) = service_with(handler);

    let t = Instant::now();
    svc.close(id);
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "close() must remain prompt"
    );
    // After close, new connections must fail (listener socket dropped).
    let res = TcpStream::connect_timeout(&addr, Duration::from_millis(300));
    assert!(res.is_err(), "listener socket must be closed");
}
