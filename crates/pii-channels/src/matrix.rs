//! `MatrixChannel` (spec 112 R3): the client-server API's `sync`, a second
//! protocol family (an opaque `next_batch` token, not an integer offset).
//!
//! **Honest scope: unencrypted rooms only.** An `m.room.encrypted` event needs
//! Olm/Megolm, i.e. device keys and a crypto store; a thin adapter cannot
//! decrypt it, so such events are dropped (never faked). Use an unencrypted
//! room for the bot.
//!
//! The trait cursor is `origin_server_ts` (monotone, `u64`); the opaque
//! `since` token lives in memory and is only an efficiency — after a restart
//! the first sync is full and events at or below the persisted cursor are
//! filtered out.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};
use serde_json::Value;

use crate::{Channel, Incoming, auth_rejected, scrub};

/// Default env var holding the access token.
pub const DEFAULT_TOKEN_ENV: &str = "PII_MATRIX_TOKEN";

/// A Matrix bot channel.
pub struct MatrixChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    base: Option<String>,
    token: Option<String>,
    /// The bot's own user id; its events are skipped so a reply is not
    /// consumed as a new message.
    user_id: Option<String>,
    since: Mutex<Option<String>>,
    rejected: Mutex<Option<String>>,
    txn: AtomicU64,
}

impl MatrixChannel {
    /// Builds the channel; missing `token` / `base` / `user_id` make it
    /// `Degraded` and fail every poll loudly (spec 112 R5).
    #[must_use]
    pub fn new(
        name: &str,
        transport: Arc<dyn HttpTransport>,
        token: Option<String>,
        base: Option<&str>,
        user_id: Option<&str>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            transport,
            base: base.map(|b| b.trim_end_matches('/').to_owned()),
            token,
            user_id: user_id.map(str::to_owned),
            since: Mutex::new(None),
            rejected: Mutex::new(None),
            txn: AtomicU64::new(0),
        }
    }

    fn missing(&self) -> Option<&'static str> {
        if self.token.is_none() {
            Some("token missing")
        } else if self.base.is_none() {
            Some("homeserver (base_url) missing")
        } else if self.user_id.is_none() {
            Some("user_id missing")
        } else {
            None
        }
    }

    fn ready(&self) -> Result<(&str, &str), PiiError> {
        if let Some(why) = self.missing() {
            return Err(PiiError::Invalid(format!("matrix '{}': {why}", self.name)));
        }
        match (self.token.as_deref(), self.base.as_deref()) {
            (Some(t), Some(b)) => Ok((t, b)),
            _ => Err(PiiError::Internal(String::from("matrix: unreachable"))),
        }
    }

    fn call(&self, ctx: &Ctx, mut req: HttpRequest, token: &str) -> Result<HttpResponse, PiiError> {
        req.headers
            .push((String::from("Authorization"), format!("Bearer {token}")));
        let resp = self.transport.request(ctx, &req).map_err(|e| match e {
            PiiError::Denied(m) => PiiError::Denied(scrub(&m, token)),
            other => PiiError::Unavailable(scrub(&other.to_string(), token)),
        })?;
        if matches!(resp.status, 401 | 403) {
            *self.rejected.lock() = Some(format!(
                "homeserver rejected the token (status {})",
                resp.status
            ));
            return Err(auth_rejected("matrix", resp.status));
        }
        if !resp.is_success() {
            return Err(PiiError::Unavailable(format!(
                "matrix: status {}",
                resp.status
            )));
        }
        *self.rejected.lock() = None;
        Ok(resp)
    }
}

/// Percent-encodes everything but unreserved characters (room ids contain
/// `!` and `:`).
fn enc(s: &str) -> String {
    use std::fmt::Write as _;
    s.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

impl Channel for MatrixChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "matrix"
    }

    fn egress_hosts(&self) -> Vec<String> {
        self.base
            .as_deref()
            .and_then(crate::url_target)
            .into_iter()
            .collect()
    }

    fn health_issue(&self) -> Option<String> {
        self.missing()
            .map(str::to_owned)
            .or_else(|| self.rejected.lock().clone())
    }

    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let (token, base) = self.ready()?;
        let since = self.since.lock().clone();
        let since_q = since.map_or_else(String::new, |s| format!("&since={}", enc(&s)));
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: format!("{base}/_matrix/client/v3/sync?timeout=0{since_q}"),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 15_000,
        };
        let resp = self.call(ctx, req, token)?;
        let v: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| PiiError::Unavailable(format!("matrix: bad sync body: {e}")))?;
        let floor = cursor.unwrap_or(0);
        let me = self.user_id.as_deref().unwrap_or_default();
        let mut out = Vec::new();
        if let Some(rooms) = v.pointer("/rooms/join").and_then(Value::as_object) {
            for (room, r) in rooms {
                let events = r.pointer("/timeline/events").and_then(Value::as_array);
                for e in events.into_iter().flatten() {
                    if let Some(m) = self.message(e, room, me, floor) {
                        out.push(m);
                    }
                }
            }
        }
        out.sort_by_key(|m| m.cursor);
        // Only remember the batch once it parsed: a failed sync is re-asked
        // from the previous token (at-least-once).
        if let Some(nb) = v.get("next_batch").and_then(Value::as_str) {
            *self.since.lock() = Some(nb.to_owned());
        }
        Ok(out)
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let (token, base) = self.ready()?;
        let n = self.txn.fetch_add(1, Ordering::Relaxed);
        let body = serde_json::to_vec(&serde_json::json!({ "msgtype": "m.text", "body": text }))
            .map_err(|e| PiiError::Internal(format!("matrix: encode: {e}")))?;
        let req = HttpRequest {
            method: HttpMethod::Put,
            url: format!(
                "{base}/_matrix/client/v3/rooms/{}/send/m.room.message/pii-{}-{n}",
                enc(to),
                std::process::id()
            ),
            headers: vec![(
                String::from("Content-Type"),
                String::from("application/json"),
            )],
            body,
            timeout_ms: 10_000,
        };
        self.call(ctx, req, token).map(|_| ())
    }
}

impl MatrixChannel {
    fn message(&self, e: &Value, room: &str, me: &str, floor: u64) -> Option<Incoming> {
        if e.get("type")?.as_str()? != "m.room.message" {
            return None;
        }
        let sender = e.get("sender")?.as_str()?;
        let ts = e.get("origin_server_ts")?.as_u64()?;
        if sender == me || ts <= floor {
            return None;
        }
        Some(Incoming {
            channel: self.name.clone(),
            chat: room.to_owned(),
            user: sender.to_owned(),
            text: e.pointer("/content/body")?.as_str()?.to_owned(),
            cursor: ts,
        })
    }
}
