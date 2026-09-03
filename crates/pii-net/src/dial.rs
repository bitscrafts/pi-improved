//! Outbound dialling (F2.6): the ONLY `TcpStream::connect` in the workspace.

use std::time::Duration;

use pii_abi::PiiError;

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

/// Milliseconds remaining before the shared deadline (§4.3.4).
#[must_use]
pub fn remaining_ms(deadline_ms: u64, clock: &dyn Clock) -> u64 {
    deadline_ms.saturating_sub(clock.now_ms())
}
