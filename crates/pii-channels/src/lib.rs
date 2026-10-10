//! `pii-channels` (F6.1): inbound polling + outbound reply, no sockets of its
//! own (the webhook channel's listener is bound by the daemon through
//! `NetService`; this crate only holds its request handler).
//!
//! Every channel goes through `Arc<dyn HttpTransport>` (from `pii-core`),
//! which is what keeps this crate testable with a fake and keeps
//! `cargo xtask net-check` clean — `pii-net` never appears in this crate's
//! dependency graph.

#![forbid(unsafe_code)]

pub mod cli;
pub mod discord;
pub mod matrix;
pub mod observe;
pub mod secret;
pub mod telegram;
pub mod webhook;

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

/// A pollable, reply-capable channel (F6.2). The trait is the extension point
/// (spec 112, `writing-a-channel.md`); inbound push (webhook) is a queue that
/// `poll` drains, so the router stays poll-shaped.
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
    /// Why this channel cannot work right now (missing token, credential
    /// rejected, ...), or `None` when healthy. A `Some` is reported as
    /// `Degraded` by the daemon's health (spec 112 R6). Never contains a secret.
    fn health_issue(&self) -> Option<String> {
        None
    }
}

/// Message prefix that marks a credential rejection (see [`auth_rejected`]).
const AUTH_REJECTED: &str = "auth rejected";

/// The error for an upstream 401/403: not transient, not an egress denial.
#[must_use]
pub fn auth_rejected(adapter: &str, status: u16) -> PiiError {
    PiiError::Invalid(format!("{adapter}: {AUTH_REJECTED} (status {status})"))
}

/// True when `e` came from [`auth_rejected`].
#[must_use]
pub fn is_auth_rejected(e: &PiiError) -> bool {
    matches!(e, PiiError::Invalid(m) if m.contains(AUTH_REJECTED))
}

/// Replaces every occurrence of `secret` in `text` (a token embedded in a URL
/// that a transport error echoed back must not reach logs).
#[must_use]
pub fn scrub(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "***")
    }
}
