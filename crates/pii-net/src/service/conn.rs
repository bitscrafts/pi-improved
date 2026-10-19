//! `Conn`: the live, grant-charging connection and its byte-carrying
//! `Stream` (plain TCP or rustls TLS), plus `GrantInfo` counters.

// `Inner`/`Registry` are `pub(crate)` for use by sibling modules
// (`service::mod`), not this module's own callers — genuinely
// crate-scoped, not redundant, despite `mod conn` itself being private.
#![allow(clippy::redundant_pub_crate)]

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rustls::StreamOwned;

use pii_abi::PiiError;

use crate::clock::Clock;
use crate::grant::NetGrant;

/// The byte-carrying stream behind [`Conn`]: a plain TCP stream (http) or a
/// rustls TLS stream (https, spec 17 R3).
///
/// An enum, not a trait object: two variants, no vtable, deadline logic
/// stays in `Conn`.
pub(super) enum Stream {
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
    pub(super) fn set_timeouts(&mut self, d: Duration) -> Result<(), PiiError> {
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
    pub(super) stream: Stream,
    pub(super) key: (String, String),
    pub(super) inner: Arc<Inner>,
    pub(super) deadline_ms: u64,
    /// Bytes this connection has sent: the window `max_bytes_out` bounds.
    pub(super) sent: u64,
    /// Bytes this connection has received: the window `max_bytes_in` bounds.
    pub(super) received: u64,
}

/// Shared service state.
pub(crate) struct Inner {
    pub(crate) resolver: Arc<dyn crate::resolve::Resolver>,
    pub(crate) grants: crate::grant::Grants,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) registry: Mutex<Registry>,
    pub(crate) listeners: Mutex<BTreeMap<u64, crate::listen::ListenerEntry>>,
    /// Set once by [`super::NetService::set_observer`].
    pub(crate) observer: std::sync::OnceLock<Arc<dyn pii_observe::Observer>>,
}

/// Process-wide registry keyed by `(principal, key)`.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) leases: BTreeMap<(String, String), u32>,
    pub(crate) bytes_in: BTreeMap<(String, String), u64>,
    pub(crate) bytes_out: BTreeMap<(String, String), u64>,
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
    /// Bytes received, cumulative (a report; the budget is per connection).
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
    pub fn arm_deadline(
        &mut self,
        pinned: &crate::authorize::Pinned,
        clock: &dyn Clock,
    ) -> Result<(), PiiError> {
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
    #[allow(clippy::significant_drop_tightening)] // reason: the registry guard is held across the
    // read-modify-write of the cumulative byte report.
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), PiiError> {
        let grant = self.grant();
        self.sent = self.sent.saturating_add(bytes.len() as u64);
        {
            let mut reg = self.inner.registry.lock();
            let total = reg.bytes_out.entry(self.key.clone()).or_insert(0);
            *total = total.saturating_add(bytes.len() as u64);
        }
        if self.sent > grant.max_bytes_out {
            return Err(PiiError::Denied(String::from("net: byte budget exhausted")));
        }
        std::io::Write::write_all(&mut self.stream, bytes)
            .map_err(|e| PiiError::Unavailable(format!("net: write: {e}")))
    }

    /// Reads up to `buf.len()` bytes, charging `max_bytes_in`.
    ///
    /// # Errors
    /// [`PiiError::Denied`] past the byte budget.
    #[allow(clippy::significant_drop_tightening)] // reason: the registry guard is held across the
    // read-modify-write of the cumulative byte report.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PiiError> {
        let grant = self.grant();
        // A peer that closes without a TLS close_notify is END OF STREAM, not a failure.
        // rustls surfaces that as an io error reading "peer closed connection without sending TLS close_notify", and hosted
        // providers do it routinely after the last frame — measured 2026-10-18 against ollama-cloud: a complete answer was
        // truncated and reported as "unavailable: net: read: ..." because a normal close was read as a fault.
        // A close that arrives BEFORE any byte of this read is still an error: silence is not an answer.
        // (2026-10-19, measured: under a guard-disabled mutation the error reaching this arm is io
        // ErrorKind::UnexpectedEof, text "peer closed connection without sending TLS close_notify" — exactly the
        // shape the arm maps; the arm above is the measured match, not a guess. The diagnostic
        // that proved this was removed after the measurement.)
        let n = match std::io::Read::read(&mut self.stream, buf) {
            Ok(n) => n,
            Err(e)
                if e.kind() == std::io::ErrorKind::UnexpectedEof
                    || e.to_string().contains("close_notify") =>
            {
                0
            }
            // Any other read error (timeout, reset, budget-independent faults)
            // stays an error: only a peer close becomes end-of-stream.
            Err(e) => return Err(PiiError::Unavailable(format!("net: read: {e}"))),
        };
        self.received = self.received.saturating_add(n as u64);
        {
            let mut reg = self.inner.registry.lock();
            let total = reg.bytes_in.entry(self.key.clone()).or_insert(0);
            *total = total.saturating_add(n as u64);
        }
        if self.received > grant.max_bytes_in {
            return Err(PiiError::Denied(String::from("net: byte budget exhausted")));
        }
        Ok(n)
    }

    pub(super) fn grant(&self) -> NetGrant {
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
