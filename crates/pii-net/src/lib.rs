//! `pii-net`: the one socket (F2.1). The only crate in the workspace that
//! opens a socket or resolves a name — enforced by `cargo xtask net-check`.
//! Synchronous by design: `GraphNode::run`, `ModelProvider::chat` and
//! `pii_core::HttpTransport` all execute on blocking threads.

pub mod authorize;
pub mod clock;
pub mod dial;
pub mod grant;
mod health;
pub mod http_codec;
pub mod limits;
pub mod listen;
pub mod resolve;
pub mod service;

#[cfg(feature = "testing")]
pub mod testing;

pub use authorize::{AddrClass, NetDenied, Pinned, Target};
pub use clock::{Clock, FixedClock, SystemClock};
pub use grant::{GrantStore, Grants, HostPattern, HostScope, NetGrant, StaticGrants};
pub use http_codec::Scheme;
pub use listen::{HttpHandler, ListenSpec, ListenerId, ListenerInfo, Protocol};
pub use resolve::{Resolver, StaticResolver, SystemResolver};
pub use service::{Conn, GrantInfo, NetConfig, NetService};

use pii_abi::PiiError;
use pii_core::Ctx;

/// Resolves a model-side URL target and authorizes it in one call — the
/// convenience seam `pii-model`'s provider uses through
/// [`pii_core::HttpTransport`], never a socket.
///
/// # Errors
/// See [`authorize::authorize`].
pub fn check_target(
    ctx: &Ctx,
    target: &Target,
    grants: &Grants,
    key: &str,
    timeout_ms: u64,
    clock: &dyn Clock,
) -> Result<Pinned, PiiError> {
    let principal = ctx.principal_str();
    let grant = grants
        .grant(&principal, key)
        .ok_or_else(|| PiiError::Denied(format!("net: no grant for '{key}'")))?;
    authorize::authorize(target, &[], &grant, clock.now_ms(), timeout_ms)
}
