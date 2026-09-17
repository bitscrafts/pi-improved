//! `NetService` (F2.5): the process-wide egress chokepoint. Clones share one
//! `Arc`, so the connection registry and byte counters never multiply.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rustls::StreamOwned;

use pii_abi::{HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};

use crate::authorize::{Pinned, Target};
use crate::clock::Clock;
use crate::dial::{dial, dial_tls};
use crate::grant::{Grants, NetGrant};
use crate::http_codec::{Scheme, parse_response, parse_url};

/// Service configuration. No `Default`: every field is load-bearing.
#[derive(Clone)]
pub struct NetConfig {
    /// Name resolver.
    pub resolver: Arc<dyn crate::resolve::Resolver>,
    /// Grant source.
    pub grants: Grants,
    /// Wall clock (grant expiry, deadlines).
    pub clock: Arc<dyn Clock>,
}

/// The byte-carrying stream behind [`Conn`]: a plain TCP stream (http) or a
/// rustls TLS stream (https, spec 17 R3).
///
/// An enum, not a trait object: two variants, no vtable, deadline logic
/// stays in `Conn`.
pub enum Stream {
    /// Plaintext HTTP transport.
    Plain(TcpStream),
    /// TLS-wrapped transport (spec 17).
    #[allow(clippy::large_enum_variant)] // reason: the TLS variant carries the
    // rustls session state by design; boxing it would put it on the heap for
    // every https conn with no layout benefit
    Tls(Box<StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl std::io::Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl std::io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

impl Stream {
    /// Arms the shared deadline on whichever transport is live (§4.3.4).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when the OS refuses the timeout.
    fn set_timeouts(&mut self, d: Duration) -> Result<(), PiiError> {
        let map = |e| PiiError::Invalid(format!("net: timeout: {e}"));
        match self {
            Self::Plain(s) => {
                s.set_read_timeout(Some(d)).map_err(map)?;
                s.set_write_timeout(Some(d)).map_err(map)
            }
            // The TLS stream forwards timeouts to the wrapped TCP stream;
            // rustls reads honour them during handshake and data phases.
            Self::Tls(s) => {
                s.sock.set_read_timeout(Some(d)).map_err(map)?;
                s.sock.set_write_timeout(Some(d)).map_err(map)
            }
        }
    }
}

/// A live connection charging its grant (F2.7).
pub struct Conn {
    stream: Stream,
    key: (String, String),
    inner: Arc<Inner>,
    deadline_ms: u64,
}

/// Shared service state.
pub(crate) struct Inner {
    pub(crate) resolver: Arc<dyn crate::resolve::Resolver>,
    pub(crate) grants: Grants,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) registry: Mutex<Registry>,
    pub(crate) listeners: Mutex<BTreeMap<u64, crate::listen::ListenerEntry>>,
}

/// Process-wide registry keyed by `(principal, key)`.
#[derive(Default)]
pub(crate) struct Registry {
    leases: BTreeMap<(String, String), u32>,
    bytes_in: BTreeMap<(String, String), u64>,
    bytes_out: BTreeMap<(String, String), u64>,
}

/// One grant's live counters (F2.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantInfo {
    /// Principal of the grant.
    pub principal: String,
    /// Grant key.
    pub key: String,
    /// Live connection leases.
    pub conns: u32,
    /// Bytes received, cumulative.
    pub bytes_in: u64,
    /// Bytes sent, cumulative.
    pub bytes_out: u64,
}

impl Drop for Conn {
    fn drop(&mut self) {
        let mut reg = self.inner.registry.lock();
        if let Some(n) = reg.leases.get_mut(&self.key) {
            *n = n.saturating_sub(1);
        }
    }
}

impl Conn {
    /// Arms the shared deadline on the socket's read/write timeouts (§4.3.4:
    /// one deadline spans dial + write + read; no per-stage budgets).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when the OS refuses the timeout.
    pub fn arm_deadline(&mut self, pinned: &Pinned, clock: &dyn Clock) -> Result<(), PiiError> {
        self.deadline_ms = pinned.deadline_ms;
        let remaining = pinned.deadline_ms.saturating_sub(clock.now_ms());
        let d = Duration::from_millis(remaining.max(1));
        self.stream.set_timeouts(d)
    }

    /// Sends raw bytes, charging `max_bytes_out`.
    ///
    /// # Errors
    /// [`PiiError::Denied`] past the byte budget; I/O errors are
    /// [`PiiError::Unavailable`].
    #[allow(clippy::significant_drop_tightening)] // reason: the MutexGuard must
    // stay held across the read-modify-write of the byte counter.
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), PiiError> {
        let grant = self.grant();
        {
            let mut reg = self.inner.registry.lock();
            let used = reg.bytes_out.entry(self.key.clone()).or_insert(0);
            *used = used.saturating_add(bytes.len() as u64);
            if *used > grant.max_bytes_out {
                return Err(PiiError::Denied(String::from("net: byte budget exhausted")));
            }
        }
        std::io::Write::write_all(&mut self.stream, bytes)
            .map_err(|e| PiiError::Unavailable(format!("net: write: {e}")))
    }

    /// Reads up to `buf.len()` bytes, charging `max_bytes_in`.
    ///
    /// # Errors
    /// [`PiiError::Denied`] past the byte budget.
    #[allow(clippy::significant_drop_tightening)] // reason: the MutexGuard must
    // stay held across the read-modify-write of the byte counter.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PiiError> {
        let grant = self.grant();
        let n = std::io::Read::read(&mut self.stream, buf)
            .map_err(|e| PiiError::Unavailable(format!("net: read: {e}")))?;
        {
            let mut reg = self.inner.registry.lock();
            let used = reg.bytes_in.entry(self.key.clone()).or_insert(0);
            *used = used.saturating_add(n as u64);
            if *used > grant.max_bytes_in {
                return Err(PiiError::Denied(String::from("net: byte budget exhausted")));
            }
        }
        Ok(n)
    }

    fn grant(&self) -> NetGrant {
        self.inner
            .grants
            .grant(&self.key.0, &self.key.1)
            .or_else(|| self.inner.grants.grant("*", &self.key.1))
            .unwrap_or_else(|| NetGrant {
                connect: crate::grant::HostScope::default(),
                allow_private: false,
                max_conns: 0,
                max_bytes_out: 0,
                max_bytes_in: 0,
                expires_at_ms: Some(0),
            })
    }
}

impl Clone for NetService {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

/// The egress service (F2.5).
pub struct NetService(pub(crate) Arc<Inner>);

impl NetService {
    /// Builds the service; clones share the registry (§4.3.5).
    #[must_use]
    pub fn new(cfg: NetConfig) -> Self {
        Self(Arc::new(Inner {
            resolver: cfg.resolver,
            grants: cfg.grants,
            clock: cfg.clock,
            registry: Mutex::new(Registry::default()),
            listeners: Mutex::new(BTreeMap::new()),
        }))
    }

    /// Resolves once, authorizes once, dials only the pinned set (F2.6).
    ///
    /// # Errors
    /// See [`crate::authorize::authorize`]; dial failure is
    /// [`PiiError::Unavailable`].
    #[allow(clippy::significant_drop_tightening)] // reason: the MutexGuard must
    // stay held across the read-modify-write of the lease counter.
    pub fn connect(
        &self,
        ctx: &Ctx,
        target: &Target,
        key: &str,
        timeout_ms: u64,
        scheme: Scheme,
    ) -> Result<Conn, PiiError> {
        let principal = ctx.principal_str();
        // The principal's own grant first; a "*" wildcard grant is the
        // process-wide default (StaticGrants semantics).
        let grant = self
            .0
            .grants
            .grant(&principal, key)
            .or_else(|| self.0.grants.grant("*", key))
            .ok_or_else(|| PiiError::Denied(format!("net: no grant for '{key}'")))?;
        let resolved: Vec<IpAddr> = self
            .0
            .resolver
            .resolve(&target.host, target.port)?
            .iter()
            .map(SocketAddr::ip)
            .collect();
        let pinned = crate::authorize::authorize(
            target,
            &resolved,
            &grant,
            self.0.clock.now_ms(),
            timeout_ms,
        )?;
        let k = (principal, String::from(key));
        #[allow(clippy::significant_drop_tightening)] // reason: the MutexGuard
        // must stay held across the read-modify-write of the lease counter.
        {
            let mut reg = self.0.registry.lock();
            let n = reg.leases.entry(k.clone()).or_insert(0);
            if *n >= grant.max_conns {
                return Err(PiiError::Denied(format!(
                    "net: connection lease cap {} reached",
                    grant.max_conns
                )));
            }
            *n += 1;
        }
        // Single exit path (D1): dial AND the TLS wrap that follows it are
        // folded into one `Result` before the match, so ANY failure after
        // the lease increment above — dial failure or TLS handshake
        // failure — takes the same `Err` arm and decrements the lease. A
        // `?` on the TLS step here would bypass the decrement and leak the
        // lease; that was the bug this comment guards against.
        let dial_result: Result<Stream, PiiError> =
            dial(ctx, &pinned, &target.host, self.0.clock.as_ref()).and_then(|stream| {
                // Spec 17 R3/R4: TLS wraps the dialed stream AFTER dial,
                // inside pii-net — the authorize→pin→grant→dial chokepoint
                // is unchanged.
                if scheme == Scheme::Https {
                    Ok(Stream::Tls(Box::new(dial_tls(stream, &target.host)?)))
                } else {
                    Ok(Stream::Plain(stream))
                }
            });
        match dial_result {
            Ok(stream) => {
                let mut conn = Conn {
                    stream,
                    key: k,
                    inner: Arc::clone(&self.0),
                    deadline_ms: pinned.deadline_ms,
                };
                let _ = conn.arm_deadline(&pinned, self.0.clock.as_ref());
                Ok(conn)
            }
            Err(e) => {
                let mut reg = self.0.registry.lock();
                if let Some(n) = reg.leases.get_mut(&k) {
                    *n = n.saturating_sub(1);
                }
                Err(e)
            }
        }
    }

    /// Live counters per grant (F2.7).
    #[must_use]
    pub fn grant_info(&self) -> Vec<GrantInfo> {
        let reg = self.0.registry.lock();
        self.0
            .grants
            .all()
            .into_iter()
            .map(|(p, k, _)| {
                let key = (p.clone(), k.clone());
                GrantInfo {
                    principal: p,
                    key: k,
                    conns: reg.leases.get(&key).copied().unwrap_or(0),
                    bytes_in: reg.bytes_in.get(&key).copied().unwrap_or(0),
                    bytes_out: reg.bytes_out.get(&key).copied().unwrap_or(0),
                }
            })
            .collect()
    }
}

impl HttpTransport for NetService {
    fn request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
        let (scheme, target, path) = parse_url(&req.url)?;
        let host_header = format!("{}:{}", target.host, target.port);
        let key = format!("http:{}", target.host);
        // connect() resolves exactly once and authorizes against the SAME
        // pinned set — never re-resolve here (§4.3.2).
        let mut conn = self.connect(ctx, &target, &key, req.timeout_ms, scheme)?;
        let pinned = Pinned {
            addrs: Vec::new(),
            deadline_ms: self.0.clock.now_ms().saturating_add(req.timeout_ms),
        };
        conn.arm_deadline(&pinned, self.0.clock.as_ref())?;
        let wire = crate::http_codec::render_request(req, &host_header, &path);
        conn.write_all(&wire)?;
        let raw = read_to_deadline(&mut conn)?;
        parse_response(&raw)
    }

    /// Streaming client path (spec 4.1c): same single connect + write as
    /// [`NetService::request`], but each socket read is forwarded through
    /// `on_chunk` as it arrives. The returned [`HttpResponse`] is assembled
    /// from the same bytes, so non-streaming consumers see no difference.
    /// Reuses `Conn::read` — no new socket path, byte accounting intact.
    #[allow(clippy::too_many_lines)] // reason: dechunk framing + deadline
    // accounting is one cohesive wire loop; splitting it would hide the
    // byte-budget invariants this function exists to enforce
    fn request_streaming(
        &self,
        ctx: &Ctx,
        req: &HttpRequest,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> {
        let (scheme, target, path) = parse_url(&req.url)?;
        let host_header = format!("{}:{}", target.host, target.port);
        let key = format!("http:{}", target.host);
        let mut conn = self.connect(ctx, &target, &key, req.timeout_ms, scheme)?;
        let pinned = Pinned {
            addrs: Vec::new(),
            deadline_ms: self.0.clock.now_ms().saturating_add(req.timeout_ms),
        };
        conn.arm_deadline(&pinned, self.0.clock.as_ref())?;
        let wire = crate::http_codec::render_request(req, &host_header, &path);
        conn.write_all(&wire)?;
        let mut head: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let head_end = loop {
            match conn.read(&mut buf) {
                Ok(0) => return Err(PiiError::Invalid("http: truncated response".into())),
                Ok(n) => {
                    head.extend_from_slice(buf.get(..n).unwrap_or(&buf));
                    if head.len() > crate::limits::MAX_HTTP_BODY {
                        return Err(PiiError::Invalid("http: body too large".into()));
                    }
                    if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                }
                Err(e) => return Err(e),
            }
        };
        let (status, headers, _body_start) = crate::http_codec::parse_response_head(&head)?;
        // R3 (spec 22 A3): when the response is chunked, `on_chunk` must
        // receive DECHUNKED body bytes — not the hex size lines. Reuses
        // `read_chunked`'s incremental state machine; the returned
        // HttpResponse.body is identical to the buffered `request` path.
        let chunked = headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
        });
        // Stream everything after the head (plus what arrived early).
        let mut pending: Vec<u8> = head.get(head_end..).unwrap_or(&[]).to_vec();
        let mut total: Vec<u8> = Vec::new();
        let mut consumed: usize = 0; // bytes of `pending` consumed by chunk framing
        if chunked {
            // Incremental dechunk: parse as many complete chunks as the
            // bytes seen so far allow, forwarding decoded bytes only.
            dechunk_step(&pending, &mut consumed, &mut |d| {
                total.extend_from_slice(d);
                on_chunk(d);
            })?;
            pending.drain(..consumed);
            consumed = 0;
        } else {
            total.extend_from_slice(&pending);
            on_chunk(&pending);
            pending.clear();
        }
        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let sl = buf.get(..n).unwrap_or(&buf);
                    pending.extend_from_slice(sl);
                    if pending.len() > crate::limits::MAX_HTTP_BODY {
                        return Err(PiiError::Invalid("http: body too large".into()));
                    }
                    if chunked {
                        dechunk_step(&pending, &mut consumed, &mut |d| {
                            total.extend_from_slice(d);
                            on_chunk(d);
                        })?;
                        pending.drain(..consumed);
                        consumed = 0;
                    } else {
                        total.extend_from_slice(sl);
                        on_chunk(sl);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        if chunked {
            // The final 0-size chunk may still be unprocessed.
            dechunk_step(&pending, &mut consumed, &mut |d| {
                total.extend_from_slice(d);
                on_chunk(d);
            })?;
        }
        Ok(HttpResponse {
            status,
            headers,
            body: total,
        })
    }
}

/// Incremental chunked-encoding decoder: consumes as many COMPLETE chunks
/// as `raw[..*pos]` contains, forwarding decoded body bytes through `out`.
/// State is fully derived from `raw` + `*pos`, so it can be called again
/// after more bytes arrive (resumable across socket reads).
fn dechunk_step(raw: &[u8], pos: &mut usize, out: &mut dyn FnMut(&[u8])) -> Result<(), PiiError> {
    loop {
        // Need a full size line first.
        let Some(rel) = find_crlf(raw.get(*pos..).unwrap_or(&[])) else {
            return Ok(());
        };
        let size_line = std::str::from_utf8(raw.get(*pos..*pos + rel).unwrap_or(&[]))
            .map_err(|_| PiiError::Invalid("http: bad chunk size".into()))?;
        let size_str = size_line.split(';').next().unwrap_or("0");
        let size = usize::try_from(
            u64::from_str_radix(size_str.trim(), 16)
                .map_err(|_| PiiError::Invalid("http: bad chunk size".into()))?,
        )
        .map_err(|_| PiiError::Invalid("http: body too large".into()))?;
        let data_start = *pos + rel + 2;
        if size == 0 {
            // Terminating chunk: consume it and any trailing CRLF/trailers.
            *pos = data_start;
            while let Some(r) = find_crlf(raw.get(*pos..).unwrap_or(&[])) {
                let Some(trailer) = raw.get(*pos..*pos + r) else {
                    break;
                };
                *pos += r + 2;
                if trailer.is_empty() {
                    break;
                }
            }
            return Ok(());
        }
        if total_len(*pos, size)? > raw.len() {
            return Ok(()); // incomplete chunk: wait for more bytes
        }
        let chunk = raw
            .get(data_start..data_start + size)
            .ok_or_else(|| PiiError::Invalid("http: truncated chunk".into()))?;
        out(chunk);
        *pos = data_start + size;
        // Consume the CRLF after the chunk data.
        if raw.get(*pos) == Some(&b'\r') {
            *pos += 2;
        }
    }
}

/// Hard size gate shared with the chunked reader (`MAX_BODY` + framing).
fn total_len(pos: usize, size: usize) -> Result<usize, PiiError> {
    pos.checked_add(size)
        .and_then(|end| end.checked_add(2))
        .filter(|end| *end <= crate::limits::MAX_BODY + 4)
        .ok_or_else(|| PiiError::Invalid("http: body too large".into()))
}

/// Finds the offset of the first CRLF in `s`.
fn find_crlf(s: &[u8]) -> Option<usize> {
    s.windows(2).position(|w| w == b"\r\n")
}

/// Reads until EOF or the armed timeout, up to 8 MiB (HTTP client side).
fn read_to_deadline(conn: &mut Conn) -> Result<Vec<u8>, PiiError> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match conn.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                out.extend_from_slice(buf.get(..n).unwrap_or(&buf));
                if out.len() > crate::limits::MAX_HTTP_BODY {
                    return Err(PiiError::Invalid("http: body too large".into()));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod streaming_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::significant_drop_tightening,
        clippy::disallowed_methods // reason: the loopback fake server binds
        // TcpListener inside pii-net's own test module (sanctioned file).
    )]

    use std::sync::Arc;

    use super::NetService;
    use crate::NetConfig;
    use pii_abi::{HttpMethod, HttpRequest};
    use pii_core::HttpTransport as _;
    use pii_core::{Budget, CapSet, Ctx};

    fn ctx() -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = pii_core::CapKind::NET;
        Ctx::root(caps, Budget::unlimited())
    }

    #[allow(dead_code)] // reason: reserved for non-loopback variants
    fn _unused() {}

    /// A loopback server that emits a chunk-framed NDJSON body, split at a
    /// MID-LINE boundary (R3 acceptance shape).
    struct ChunkedServer {
        payload: &'static [u8],
    }

    impl ChunkedServer {
        fn serve(&self) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
            use std::fmt::Write as _;
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let payload = self.payload;
            let handle = std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 8192];
                // Read until we've seen the request headers + body (the
                // client POST has a small body). Do NOT wait for EOF: the
                // client keeps the write side open while it reads the
                // response head, so an EOF-waiting read would deadlock and
                // the client would time out.
                let mut got = 0usize;
                while got < 4096 {
                    match s.read(&mut buf[got..]) {
                        Ok(n) if n > 0 => {
                            got += n;
                            if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                // Two chunks; the split falls mid-NDJSON-line.
                let (a, b) = payload.split_at(payload.len() / 2);
                let mut resp =
                    String::from("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
                let _ = write!(resp, "{:x}\r\n", a.len());
                resp.push_str(std::str::from_utf8(a).unwrap());
                resp.push_str("\r\n");
                let _ = write!(resp, "{:x}\r\n", b.len());
                resp.push_str(std::str::from_utf8(b).unwrap());
                resp.push_str("\r\n0\r\n\r\n");
                let _ = s.write_all(resp.as_bytes());
                // Deterministic FIN: the client's read loop ends on EOF.
                let _ = s.shutdown(std::net::Shutdown::Both);
            });
            (addr, handle)
        }
    }

    /// R3 (spec 22): the streaming path dechunks — `on_chunk` receives the
    /// dechunked NDJSON lines (never hex size lines), split mid-line reads
    /// are reassembled, and the returned body is byte-identical to the
    /// buffered path.
    #[test]
    fn request_streaming_dechunks() {
        let payload: &'static [u8] = br#"{"message":{"content":"Hel"},"done":false}
{"message":{"content":"lo"},"done":false}
{"done":true,"prompt_eval_count":5,"eval_count":3}
"#;
        // Server binds FIRST: the resolver pins the ephemeral addr.
        let (addr, handle) = ChunkedServer { payload }.serve();
        let mut resolver = crate::resolve::StaticResolver::new();
        resolver.insert("127.0.0.1", addr.port(), vec![addr]);
        let mut grants = crate::grant::StaticGrants::default();
        grants.insert("*", "http:127.0.0.1", crate::grant::NetGrant::open_local());
        let svc = NetService::new(NetConfig {
            resolver: Arc::new(resolver),
            grants: Arc::new(grants),
            clock: Arc::new(crate::clock::SystemClock),
        });
        let lines: parking_lot::Mutex<Vec<String>> = parking_lot::Mutex::new(Vec::new());
        let r = HttpRequest {
            method: HttpMethod::Post,
            url: format!("http://{addr}/api"),
            headers: Vec::new(),
            body: b"{}".to_vec(),
            timeout_ms: 5_000,
        };
        let resp = svc
            .request_streaming(&ctx(), &r, &mut |bytes| {
                let s = String::from_utf8_lossy(bytes);
                for l in s.lines() {
                    lines.lock().push(l.to_owned());
                }
            })
            .expect("streaming request must succeed");
        handle.join().unwrap();
        // R3: `on_chunk` is BYTE-level — it receives dechunked bytes that may
        // still split a JSON line across two reads (the downstream consumer,
        // ollama's `drain_lines`, reassembles with a residual buffer). So the
        // contract here is: no hex-size lines leak through, and the
        // CONCATENATED dechunked body is byte-identical to the buffered parse.
        for l in lines.lock().iter() {
            assert!(
                !l.trim().chars().all(|c| c.is_ascii_hexdigit()),
                "on_chunk leaked a hex size line: {l:?}"
            );
        }
        // The dechunked body is exactly the payload (no hex framing).
        assert_eq!(resp.body, payload.to_vec());
        assert!(resp.body.starts_with(b"{\"message\""));
    }
}
