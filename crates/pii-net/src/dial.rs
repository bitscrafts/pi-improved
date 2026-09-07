//! Outbound dialling (F2.6): the ONLY `TcpStream::connect` in the workspace.

use std::sync::Arc;
use std::time::Duration;

use pii_abi::PiiError;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::authorize::Pinned;
use crate::clock::Clock;

/// Dials each pinned address in order under one deadline; the first success
/// wins. Requires `CapKind::NET` before anything else (F2.6).
///
/// # Errors
/// [`PiiError::Denied`] when the context lacks the `net` capability;
/// [`PiiError::Unavailable`] when no address is reachable before the deadline.
#[allow(clippy::disallowed_methods)] // reason: dial.rs is the ONE sanctioned
// TcpStream::connect site (spec F2.6/F2.13); every byte out passes
// NetService::authorize before reaching this file.
pub fn dial(
    ctx: &pii_core::Ctx,
    pinned: &Pinned,
    host: &str,
    clock: &dyn Clock,
) -> Result<std::net::TcpStream, PiiError> {
    let _ = ctx.net()?;
    for addr in &pinned.addrs {
        let remaining = pinned.deadline_ms.saturating_sub(clock.now_ms());
        if remaining == 0 {
            break;
        }
        if let Ok(s) = std::net::TcpStream::connect_timeout(addr, Duration::from_millis(remaining))
        {
            return Ok(s);
        }
    }
    Err(PiiError::Unavailable(format!(
        "net: no address reachable for {host}"
    )))
}

/// Wraps a dialed TCP stream in a rustls client TLS connection (spec 17 R2).
///
/// Server-cert verification ON, webpki-roots trust store, SNI from the
/// parsed host, no client certs, no custom verifier (G8). The handshake
/// completes lazily on first read/write — errors surface as
/// `PiiError::Unavailable` through `Conn`.
///
/// # Errors
/// [`PiiError::Invalid`] when `host` is not a valid TLS server name;
/// [`PiiError::Unavailable`] when the TLS client setup fails.
pub fn dial_tls(
    stream: std::net::TcpStream,
    host: &str,
) -> Result<StreamOwned<ClientConnection, std::net::TcpStream>, PiiError> {
    static CONFIG: std::sync::OnceLock<Arc<ClientConfig>> = std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    });
    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| PiiError::Invalid(format!("net: tls server name '{host}': {e}")))?;
    let conn = ClientConnection::new(Arc::clone(config), name)
        .map_err(|e| PiiError::Unavailable(format!("net: tls handshake setup: {e}")))?;
    Ok(StreamOwned::new(conn, stream))
}

/// Milliseconds remaining before the shared deadline (§4.3.4).
#[must_use]
pub fn remaining_ms(deadline_ms: u64, clock: &dyn Clock) -> u64 {
    deadline_ms.saturating_sub(clock.now_ms())
}
