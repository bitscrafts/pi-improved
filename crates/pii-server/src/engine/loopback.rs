//! Loopback transport (F2.9): the SAME `serve_connection` path as UDS — the
//! only difference is which `AsyncRead + AsyncWrite` reaches it.

use std::sync::Arc;

use crate::Engine;
use crate::engine::conn::{PeerIdentity, serve_connection};

/// Serves the engine over an in-process duplex pair; returns the client
/// half. No `if loopback` branch exists anywhere in dispatch.
#[must_use]
pub fn spawn_loopback(engine: Arc<Engine>) -> tokio::io::DuplexStream {
    let (client, server) = tokio::io::duplex(1 << 20);
    tokio::spawn(serve_connection(engine, server, PeerIdentity::InProcess));
    client
}
