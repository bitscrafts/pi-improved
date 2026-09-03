//! `NetService` (F2.5): the process-wide egress chokepoint. Clones share one
//! `Arc`, so the connection registry and byte counters never multiply.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use pii_abi::{HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};

use crate::authorize::{Pinned, Target};
use crate::clock::Clock;
use crate::dial::dial;
use crate::grant::{Grants, NetGrant};
use crate::http_codec::{parse_response, parse_url};

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

/// A live connection charging its grant (F2.7).
pub struct Conn {
    stream: TcpStream,
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
        self.stream
            .set_read_timeout(Some(d))
            .map_err(|e| PiiError::Invalid(format!("net: read timeout: {e}")))?;
        self.stream
            .set_write_timeout(Some(d))
            .map_err(|e| PiiError::Invalid(format!("net: write timeout: {e}")))
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
        match dial(ctx, &pinned, &target.host, self.0.clock.as_ref()) {
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
        let (_scheme, target, path) = parse_url(&req.url)?;
        let host_header = format!("{}:{}", target.host, target.port);
        let key = format!("http:{}", target.host);
        // connect() resolves exactly once and authorizes against the SAME
        // pinned set — never re-resolve here (§4.3.2).
        let mut conn = self.connect(ctx, &target, &key, req.timeout_ms)?;
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
