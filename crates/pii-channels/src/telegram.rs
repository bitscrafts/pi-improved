//! `TelegramChannel` (spec 112 R2): the Bot API over the injected transport.
//!
//! This file is the reference adapter that `writing-a-channel.md` tells a
//! third party to copy: token from env/file, an `offset` cursor, the long-poll
//! window, 401/403 mapped to a named error and a `Degraded` health state, and
//! a mock-transport test (see `tests/adapters.rs`).

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};
use serde::Deserialize;

use crate::{Channel, Incoming, auth_rejected, scrub};

/// Default Bot API base.
pub const DEFAULT_API_BASE: &str = "https://api.telegram.org";
/// Default env var holding the bot token.
pub const DEFAULT_TOKEN_ENV: &str = "PII_TELEGRAM_TOKEN";
/// Server-side long-poll window, seconds. Kept below `timeout_ms` so the
/// transport deadline never fires first.
const LONG_POLL_S: u64 = 25;

#[derive(Debug, Deserialize)]
struct Updates {
    #[serde(default)]
    result: Vec<Update>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: u64,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    chat: Chat,
    from: Option<From>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct From {
    #[serde(default)]
    username: String,
}

/// A Telegram bot channel.
pub struct TelegramChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    base: String,
    /// Never logged, never in an error (see [`scrub`]).
    token: Option<String>,
    timeout_ms: u64,
    /// Set by a 401/403, cleared by the next success.
    rejected: Mutex<Option<String>>,
}

impl TelegramChannel {
    /// Builds the channel. A `None` token is allowed on purpose: the channel
    /// then reports `Degraded` and every poll fails loudly, instead of the
    /// daemon refusing to start (spec 112 R5).
    #[must_use]
    pub fn new(
        name: &str,
        transport: Arc<dyn HttpTransport>,
        token: Option<String>,
        base: Option<&str>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            transport,
            base: base
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_owned(),
            token,
            timeout_ms: (LONG_POLL_S + 10) * 1000,
            rejected: Mutex::new(None),
        }
    }

    fn token(&self) -> Result<&str, PiiError> {
        self.token.as_deref().ok_or_else(|| {
            PiiError::Invalid(format!(
                "telegram '{}': no token (set token_env or token_file)",
                self.name
            ))
        })
    }

    fn call(&self, ctx: &Ctx, req: &HttpRequest, token: &str) -> Result<HttpResponse, PiiError> {
        let resp = self
            .transport
            .request(ctx, req)
            .map_err(|e| redact(e, token))?;
        if matches!(resp.status, 401 | 403) {
            *self.rejected.lock() =
                Some(format!("api rejected the token (status {})", resp.status));
            return Err(auth_rejected("telegram", resp.status));
        }
        if !resp.is_success() {
            return Err(PiiError::Unavailable(format!(
                "telegram: status {}",
                resp.status
            )));
        }
        *self.rejected.lock() = None;
        Ok(resp)
    }
}

/// Keeps the error variant (so `Denied` stays `Denied`), drops the token.
fn redact(e: PiiError, token: &str) -> PiiError {
    match e {
        PiiError::Denied(m) => PiiError::Denied(scrub(&m, token)),
        PiiError::Invalid(m) => PiiError::Invalid(scrub(&m, token)),
        other => PiiError::Unavailable(scrub(&other.to_string(), token)),
    }
}

impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "telegram"
    }

    fn egress_hosts(&self) -> Vec<String> {
        crate::url_target(&self.base).into_iter().collect()
    }

    fn health_issue(&self) -> Option<String> {
        if self.token.is_none() {
            return Some(String::from("token missing"));
        }
        self.rejected.lock().clone()
    }

    /// `cursor` is the last handled `update_id`; Telegram wants `offset =
    /// update_id + 1` to acknowledge it.
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let token = self.token()?;
        let offset = cursor.map_or_else(String::new, |c| format!("offset={}&", c + 1));
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: format!(
                "{}/bot{token}/getUpdates?{offset}timeout={LONG_POLL_S}",
                self.base
            ),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: self.timeout_ms,
        };
        let resp = self.call(ctx, &req, token)?;
        let updates: Updates = serde_json::from_slice(&resp.body)
            .map_err(|e| PiiError::Unavailable(format!("telegram: bad poll body: {e}")))?;
        Ok(updates
            .result
            .into_iter()
            .filter_map(|u| {
                let m = u.message?;
                Some(Incoming {
                    channel: self.name.clone(),
                    chat: m.chat.id.to_string(),
                    user: m.from.map(|f| f.username).unwrap_or_default(),
                    text: m.text?,
                    cursor: u.update_id,
                })
            })
            .collect())
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let token = self.token()?;
        let body = serde_json::to_vec(&serde_json::json!({ "chat_id": to, "text": text }))
            .map_err(|e| PiiError::Internal(format!("telegram: encode: {e}")))?;
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: format!("{}/bot{token}/sendMessage", self.base),
            headers: vec![(
                String::from("Content-Type"),
                String::from("application/json"),
            )],
            body,
            timeout_ms: 10_000,
        };
        self.call(ctx, &req, token).map(|_| ())
    }
}
