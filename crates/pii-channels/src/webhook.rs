//! `WebhookChannel` (spec 112 R1): reachable with no vendor SDK.
//!
//! **Inbound** — `POST /channel/<name>` with a JSON body
//! `{"text": "...", "chat": "...", "user": "..."}` (`text` required). The
//! daemon binds the socket through `NetService` (this crate has none) and calls
//! [`WebhookChannel::handle_inbound`] per request; accepted messages wait in a
//! bounded queue that [`Channel::poll`] serves, so the router stays poll-shaped.
//!
//! **Bounds** — loopback by default; on a wider bind the shared secret
//! (`X-Pii-Secret`) is required and a request without it is a `401` plus a
//! `channel.auth.rejected` event. Bodies over 64 KiB are `413`, a full queue is
//! `503`. **Outbound** — `send` POSTs `{to, text}` through the engine's
//! transport, so the `http:<host>` allowlist governs it.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_core::{Clock as _, Ctx, HttpTransport};
use pii_observe::{Event, Level, Observer, names};

use crate::{Channel, Incoming};

/// The shared-secret header, inbound and outbound.
pub const SECRET_HEADER: &str = "X-Pii-Secret";
const MAX_BODY: usize = 64 * 1024;
const MAX_QUEUE: usize = 1024;

/// Static settings of a webhook channel.
pub struct WebhookConfig {
    /// Where `send` POSTs `{to, text}`; `None` = inbound only.
    pub out_url: Option<String>,
    /// The shared secret (inbound check, outbound header).
    pub secret: Option<String>,
    /// True when the listener is bound to a loopback address; false makes the
    /// secret mandatory.
    pub loopback: bool,
}

/// A webhook channel: a request handler plus a queue.
pub struct WebhookChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    cfg: WebhookConfig,
    observer: Arc<dyn Observer>,
    queue: Mutex<VecDeque<Incoming>>,
    /// Seeded from the wall clock so cursors stay monotone across restarts.
    next: AtomicU64,
}

impl WebhookChannel {
    /// Builds the channel; `observer` receives the auth-rejection events.
    #[must_use]
    pub fn new(
        name: &str,
        transport: Arc<dyn HttpTransport>,
        cfg: WebhookConfig,
        observer: Arc<dyn Observer>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            transport,
            cfg,
            observer,
            queue: Mutex::new(VecDeque::new()),
            next: AtomicU64::new(pii_core::SystemClock.now_ms()),
        }
    }

    /// The URL path this channel answers on.
    #[must_use]
    pub fn path(&self) -> String {
        format!("/channel/{}", self.name)
    }

    fn reject(&self, reason: &'static str) -> HttpResponse {
        self.observer.event(
            &Event::new("channel", Level::Warn, names::CHANNEL_AUTH_REJECTED)
                .field("channel", self.name.as_str())
                .field("kind", "webhook")
                .field("reason", reason),
        );
        reply(401, "unauthorized")
    }

    fn authorized(&self, req: &HttpRequest) -> Result<(), &'static str> {
        let offered = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(SECRET_HEADER))
            .map(|(_, v)| v.as_str());
        match (&self.cfg.secret, offered) {
            (Some(want), Some(got)) if same(want, got) => Ok(()),
            (Some(_), Some(_)) => Err("wrong secret"),
            (Some(_), None) => Err("missing secret"),
            // Loopback with no secret configured: open by design.
            (None, _) if self.cfg.loopback => Ok(()),
            (None, _) => Err("secret required on a non-loopback bind but none configured"),
        }
    }

    /// Serves one inbound request (called by the daemon's listener).
    #[must_use]
    pub fn handle_inbound(&self, req: &HttpRequest) -> HttpResponse {
        if req.url.split('?').next() != Some(self.path().as_str()) {
            return reply(404, "not found");
        }
        if req.method != HttpMethod::Post {
            return reply(405, "method not allowed");
        }
        if let Err(reason) = self.authorized(req) {
            return self.reject(reason);
        }
        if req.body.len() > MAX_BODY {
            return reply(413, "body too large");
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body) else {
            return reply(400, "body is not JSON");
        };
        let Some(text) = v.get("text").and_then(|t| t.as_str()) else {
            return reply(400, "missing \"text\"");
        };
        let field = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or("webhook")
                .to_owned()
        };
        let msg = Incoming {
            channel: self.name.clone(),
            chat: field("chat"),
            user: field("user"),
            text: text.to_owned(),
            cursor: self.next.fetch_add(1, Ordering::Relaxed),
        };
        let mut q = self.queue.lock();
        if q.len() >= MAX_QUEUE {
            return reply(503, "queue full");
        }
        q.push_back(msg);
        drop(q);
        reply(202, "accepted")
    }
}

fn reply(status: u16, msg: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: vec![(
            String::from("Content-Type"),
            String::from("application/json"),
        )],
        body: serde_json::json!({ "status": msg })
            .to_string()
            .into_bytes(),
    }
}

/// Length-independent-time comparison of two secrets.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

impl Channel for WebhookChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "webhook"
    }

    fn egress_hosts(&self) -> Vec<String> {
        self.cfg
            .out_url
            .as_deref()
            .and_then(crate::url_target)
            .into_iter()
            .collect()
    }

    fn health_issue(&self) -> Option<String> {
        (!self.cfg.loopback && self.cfg.secret.is_none()).then(|| {
            String::from("secret missing on a non-loopback bind; every request is refused")
        })
    }

    /// Acknowledges (drops) queued messages at or below `cursor`, returns the
    /// rest — so a message whose run never started is delivered again.
    fn poll(&self, _ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let mut q = self.queue.lock();
        if let Some(c) = cursor {
            q.retain(|m| m.cursor > c);
        }
        Ok(q.iter().cloned().collect())
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let Some(url) = &self.cfg.out_url else {
            return Err(PiiError::Invalid(format!(
                "webhook '{}': no outbound url configured",
                self.name
            )));
        };
        let body = serde_json::to_vec(&serde_json::json!({ "to": to, "text": text }))
            .map_err(|e| PiiError::Internal(format!("webhook: encode: {e}")))?;
        let mut headers = vec![(
            String::from("Content-Type"),
            String::from("application/json"),
        )];
        if let Some(s) = &self.cfg.secret {
            headers.push((String::from(SECRET_HEADER), s.clone()));
        }
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: url.clone(),
            headers,
            body,
            timeout_ms: 10_000,
        };
        let resp = self.transport.request(ctx, &req)?;
        if resp.is_success() {
            Ok(())
        } else {
            Err(PiiError::Unavailable(format!(
                "webhook: send status {}",
                resp.status
            )))
        }
    }
}
