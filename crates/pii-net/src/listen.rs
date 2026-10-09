//! Listener management (F2.10): one accept thread per listener, one thread
//! per connection, hard-capped, `close()` prompt via non-blocking accept.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pii_abi::PiiError;
use pii_core::Ctx;

use crate::http_codec;
use crate::service::{GrantInfo, NetService};

/// What to bind (F2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenSpec {
    /// Bind a TCP socket; port 0 = ephemeral.
    Tcp {
        /// Address to bind.
        addr: SocketAddr,
    },
}

/// Listener protocol (F2.10); HTTP is the only one in Phase 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Plaintext HTTP/1.1.
    Http,
}

/// A served listener handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(pub(crate) u64);

/// What a listener reports (F2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerInfo {
    /// Stable id.
    pub id: u64,
    /// The bound address (resolved port for port 0).
    pub addr: SocketAddr,
    /// Wire protocol.
    pub proto: Protocol,
}

/// Request handler (F2.10): must not panic; an error yields 500.
pub trait HttpHandler: Send + Sync {
    /// Handles one request.
    fn handle(&self, req: &pii_abi::HttpRequest) -> pii_abi::HttpResponse;

    /// Streaming variant (spec 4.1c): serves the same response but writes
    /// the body in the order given by `chunks` (raw bytes, not the
    /// `HttpResponse.body`), with a flush between each so a client reading
    /// incrementally sees the chunks separately. The default renders the
    /// single-body response unchanged.
    fn handle_stream(
        &self,
        req: &pii_abi::HttpRequest,
        _write: &mut dyn FnMut(&[u8]),
    ) -> pii_abi::HttpResponse {
        self.handle(req)
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl NetService {
    /// Binds a listener (F2.10). Port 0 binds ephemeral; the resolved port
    /// is reported so tests need no fixed ports.
    ///
    /// # Errors
    /// [`PiiError::Denied`] without the `net` capability; [`PiiError::Internal`]
    /// on bind failure.
    #[allow(clippy::disallowed_methods)]
    // reason: listen.rs is a sanctioned
    // socket file (spec F2.13/F1.6); TcpListener::bind is denied elsewhere.
    #[allow(clippy::needless_pass_by_value)] // reason: spec F2.10 API takes
    // the handler by value; it is cloned into the accept thread.
    pub fn listen(
        &self,
        ctx: &Ctx,
        spec: ListenSpec,
        _proto: Protocol,
        handler: Arc<dyn HttpHandler>,
    ) -> Result<ListenerId, PiiError> {
        let ListenSpec::Tcp { addr } = spec;
        let _ = ctx.net()?;
        let listener =
            TcpListener::bind(addr).map_err(|e| PiiError::Internal(format!("listen: {e}")))?;
        let bound = listener
            .local_addr()
            .map_err(|e| PiiError::Internal(format!("local_addr: {e}")))?;
        let id = ListenerId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        self.0.listeners.lock().insert(
            id.0,
            ListenerEntry {
                info: ListenerInfo {
                    id: id.0,
                    addr: bound,
                    proto: Protocol::Http,
                },
                stop: stop_tx,
                join: None,
            },
        );
        let h = Arc::clone(&handler);
        let service = self.clone();
        let handle = std::thread::Builder::new()
            .name("pii-net-accept".into())
            .spawn(move || accept_loop(service, listener, h, stop_rx))
            .map_err(|e| PiiError::Internal(format!("spawn accept: {e}")))?;
        if let Some(e) = self.0.listeners.lock().get_mut(&id.0) {
            e.join = Some(handle);
        }
        Ok(id)
    }

    /// Closes a listener: stops the accept loop and joins it.
    pub fn close(&self, id: ListenerId) {
        let entry = self.0.listeners.lock().remove(&id.0);
        if let Some(mut e) = entry {
            let _ = e.stop.send(());
            if let Some(j) = e.join.take() {
                let _ = j.join();
            }
        }
    }

    /// Every live listener (F2.5).
    #[must_use]
    pub fn listeners(&self) -> Vec<ListenerInfo> {
        self.0
            .listeners
            .lock()
            .values()
            .map(|e| e.info.clone())
            .collect()
    }

    /// Registers an externally-bound listener (e.g. the control UDS) so it
    /// appears in `sys.status` without moving its bind (F2.5).
    pub fn register_external_listener(&self, info: ListenerInfo) {
        let (tx, _rx) = std::sync::mpsc::channel();
        self.0.listeners.lock().insert(
            info.id,
            ListenerEntry {
                info,
                stop: tx,
                join: None,
            },
        );
    }

    /// Grant counters passthrough (F2.5).
    #[must_use]
    pub fn grants(&self) -> Vec<GrantInfo> {
        self.grant_info()
    }
}

pub(crate) struct ListenerEntry {
    pub(crate) info: ListenerInfo,
    pub(crate) stop: std::sync::mpsc::Sender<()>,
    pub(crate) join: Option<std::thread::JoinHandle<()>>,
}

#[allow(clippy::needless_pass_by_value)] // reason: moved into the accept
// thread; used across the loop.
#[allow(clippy::print_stderr)] // reason: accept-loop startup failure is a
// daemon-level diagnostic; no logger exists in pii-net.
fn accept_loop(
    service: NetService,
    listener: TcpListener,
    handler: Arc<dyn HttpHandler>,
    stop: std::sync::mpsc::Receiver<()>,
) {
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("pii-net: set_nonblocking: {e}");
        return;
    }
    // Bounded worker pool: spawns a fixed number of connection-serving
    // threads that drain a bounded queue of accepted streams, instead of
    // one thread per connection. Excess connections wait in the queue
    // (backpressure via the OS listen backlog) rather than spawning more
    // threads. Workers exit when the queue closes and drains (i.e. after
    // the accept loop returns on stop or listener error).
    let pool_size = crate::limits::accept_pool_size();
    let (conn_tx, conn_rx) = std::sync::mpsc::sync_channel::<TcpStream>(pool_size);
    let conn_rx = Arc::new(parking_lot::Mutex::new(conn_rx));
    let mut workers = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let rx = Arc::clone(&conn_rx);
        let h = Arc::clone(&handler);
        let name = format!("pii-net-conn-{i}");
        match std::thread::Builder::new().name(name).spawn(move || {
            loop {
                let stream = {
                    let guard = rx.lock();
                    guard.recv().ok()
                };
                let Some(stream) = stream else { return };
                serve_conn(stream, Arc::clone(&h));
            }
        }) {
            Ok(handle) => workers.push(handle),
            Err(e) => {
                eprintln!("pii-net: spawn conn worker {i}: {e}");
                break;
            }
        }
    }
    loop {
        if stop.try_recv().is_ok() {
            return;
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
                // Bounded, non-blocking send: if the queue is full (all
                // workers busy) we keep the stream and retry after the poll
                // sleep, so stop detection is never starved. The connection
                // has been accepted from the backlog already; holding it
                // here is the backpressure mechanism.
                let mut pending = Some(stream);
                loop {
                    if stop.try_recv().is_ok() {
                        drop(pending); // closing the socket resets the client
                        return;
                    }
                    match conn_tx.try_send(match pending.take() {
                        Some(s) => s,
                        // Should be impossible: we only loop when try_send
                        // failed, which consumed the stream.
                        None => break,
                    }) {
                        Ok(()) => break,
                        Err(std::sync::mpsc::TrySendError::Full(s)) => {
                            pending = Some(s);
                        }
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                            // All workers gone; nothing can serve conns.
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                service.observe_accept_failure(&e);
                break;
            }
        }
    }
    drop(conn_tx); // close the queue; workers drain remaining conns then exit
    for w in workers {
        let _ = w.join();
    }
}

#[allow(clippy::needless_pass_by_value)] // reason: moved into the per-conn
// thread; consumed by the read loop.
fn serve_conn(stream: TcpStream, handler: Arc<dyn HttpHandler>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];

    // Read until end of headers (and body if Content-Length present).
    let header_end = loop {
        match std::io::Read::read(&mut &stream, &mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                buf.extend_from_slice(chunk.get(..n).unwrap_or(&chunk));
                if buf.len() > crate::limits::MAX_SERVER_BODY {
                    let _ = std::io::Write::write_all(
                        &mut &stream,
                        &http_codec::render_response(&pii_abi::HttpResponse {
                            status: 413,
                            headers: Vec::new(),
                            body: Vec::new(),
                        }),
                    );
                    return;
                }
                if let Some(i) = find_header_end(&buf) {
                    break i;
                }
            }
        }
    };
    let head = String::from_utf8_lossy(buf.get(..header_end).unwrap_or(&buf)).into_owned();
    let body_len = content_length(&head).unwrap_or(0);
    if body_len > crate::limits::MAX_SERVER_BODY {
        let _ = std::io::Write::write_all(
            &mut &stream,
            &http_codec::render_response(&pii_abi::HttpResponse {
                status: 413,
                headers: Vec::new(),
                body: Vec::new(),
            }),
        );
        return;
    }
    while buf.len() < header_end + body_len {
        match std::io::Read::read(&mut &stream, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or(&chunk)),
        }
    }
    let req = parse_request(&head, buf.get(header_end..).unwrap_or(&[]));
    // Streaming handlers (spec 4.1c) write chunks through `stream_write`
    // before the final rendered head; buffered handlers answer unchanged.
    let mut stream_write = |bytes: &[u8]| {
        // Best-effort: the trait's sink cannot report failure; a write error
        // leaves the chunk undelivered and the final rendered head still goes
        // out below (the connection is torn down right after either way).
        let _ = std::io::Write::write_all(&mut &stream, bytes);
        let _ = std::io::Write::flush(&mut &stream);
    };
    let resp = req.map_or_else(
        |_| pii_abi::HttpResponse {
            status: 400,
            headers: Vec::new(),
            body: Vec::new(),
        },
        |r| {
            // The handler must never take the process down (F2.10).
            let h = Arc::clone(&handler);
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                h.handle_stream(&r, &mut stream_write)
            }))
            .unwrap_or_else(|_| pii_abi::HttpResponse {
                status: 500,
                headers: Vec::new(),
                body: Vec::new(),
            })
        },
    );
    let _ = std::io::Write::write_all(&mut &stream, &http_codec::render_response(&resp));
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("content-length")
            .then_some(())?;
        v.trim().parse().ok()
    })
}

fn parse_request(head: &str, body: &[u8]) -> Result<pii_abi::HttpRequest, PiiError> {
    let mut lines = head.lines();
    let status = lines
        .next()
        .ok_or_else(|| PiiError::Invalid(String::from("http: empty request line")))?;
    let mut parts = status.split_whitespace();
    let method = match parts.next() {
        Some("GET") => pii_abi::HttpMethod::Get,
        Some("POST") => pii_abi::HttpMethod::Post,
        Some("PUT") => pii_abi::HttpMethod::Put,
        Some("DELETE") => pii_abi::HttpMethod::Delete,
        _ => return Err(PiiError::Invalid(String::from("http: bad method"))),
    };
    let path = parts
        .next()
        .ok_or_else(|| PiiError::Invalid(String::from("http: missing path")))?
        .to_owned();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Ok(pii_abi::HttpRequest {
        method,
        url: path,
        headers,
        body: body.to_vec(),
        timeout_ms: 0,
    })
}
