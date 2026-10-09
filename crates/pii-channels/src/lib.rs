//! `pii-channels` (F6.1): inbound polling + outbound reply, no sockets.
//!
//! Every channel goes through `Arc<dyn HttpTransport>` (from `pii-core`),
//! which is what keeps this crate testable with a fake and keeps
//! `cargo xtask net-check` clean — `pii-net` never appears in this crate's
//! dependency graph.

#![forbid(unsafe_code)]

pub mod cli;
pub mod discord;
pub mod observe;

use pii_abi::PiiError;
use pii_core::Ctx;

/// One inbound message from a channel (F6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    /// Channel name.
    pub channel: String,
    /// Chat/conversation id to reply into.
    pub chat: String,
    /// Sending user.
    pub user: String,
    /// Message text.
    pub text: String,
    /// Monotonic cursor for this message (channel-defined ordering).
    pub cursor: u64,
}

/// A pollable, reply-capable channel (F6.2). Polling, not listening: inbound
/// webhooks need TLS and a public port, both cut from this phase.
pub trait Channel: Send + Sync {
    /// The channel's configured name (from `[[channel]]`).
    fn name(&self) -> &str;
    /// The channel kind (`"discord"`, `"cli"`, ...).
    fn kind(&self) -> &str;
    /// Fetches messages newer than `cursor` (`None` = from the beginning).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] on a transient failure (bad status,
    /// unparseable body) — the router retries next tick and never treats
    /// this as fatal.
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError>;
    /// Sends `text` to `to` (channel-defined recipient: chat id, file, ...).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] on a transient failure.
    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError>;
}
