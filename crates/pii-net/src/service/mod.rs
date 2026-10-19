//! `NetService` (F2.5): the process-wide egress chokepoint. Clones share one
//! `Arc`, so the connection registry and byte counters never multiply.

mod conn;
#[cfg(test)]
mod conn_close_tests;
mod dechunk;
mod http;
mod observe;

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use parking_lot::Mutex;

use pii_abi::PiiError;
use pii_core::Ctx;

use crate::authorize::{Target, authorize};
use crate::clock::Clock;
use crate::dial::{dial, dial_tls};
use crate::grant::Grants;
use crate::http_codec::Scheme;

pub use conn::{Conn, GrantInfo};

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

/// Shared service state (defined in [`conn`]).
use conn::{Inner, Registry};

/// The egress service (F2.5).
pub struct NetService(pub(crate) Arc<Inner>);

impl Clone for NetService {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

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
            observer: std::sync::OnceLock::new(),
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
        let result = self.connect_inner(ctx, target, key, timeout_ms, scheme);
        if let Err(e) = &result {
            self.observe_connect_failure(target, key, e);
        }
        result
    }

    #[allow(clippy::significant_drop_tightening)] // reason: see `connect`.
    fn connect_inner(
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
        let pinned = authorize(target, &resolved, &grant, self.0.clock.now_ms(), timeout_ms)?;
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
        let dial_result: Result<conn::Stream, PiiError> =
            dial(ctx, &pinned, &target.host, self.0.clock.as_ref()).and_then(|stream| {
                // Spec 17 R3/R4: TLS wraps the dialed stream AFTER dial,
                // inside pii-net — the authorize→pin→grant→dial chokepoint
                // is unchanged.
                if scheme == Scheme::Https {
                    Ok(conn::Stream::Tls(Box::new(dial_tls(stream, &target.host)?)))
                } else {
                    Ok(conn::Stream::Plain(stream))
                }
            });
        match dial_result {
            Ok(stream) => {
                let mut conn = Conn {
                    stream,
                    key: k,
                    inner: Arc::clone(&self.0),
                    deadline_ms: pinned.deadline_ms,
                    sent: 0,
                    received: 0,
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
        // the OpenAI provider's SSE reader, reassembles with a residual buffer). So the
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
