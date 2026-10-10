//! Shared HTTP plumbing for the hosted provider kinds (`openai`, `anthropic`,
//! `gemini`): one POST helper that maps every failure to the existing
//! [`PiiError`] shape, feeds it through `pii_abi::failover::classify`, and emits
//! the named `model.provider.*` events. Request headers and credentials never
//! reach an event or an error: a denial or auth failure names the HOST only.

// reason: crate-internal helpers in a private module; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::sync::Arc;

use pii_abi::{
    FailoverReason, HttpMethod, HttpRequest, HttpResponse, Origin, PiiError, ProviderFailure,
    classify_provider,
};
use pii_core::{Ctx, HttpTransport};
use pii_observe::{Event, Level, Observable, Observed, Observer, names};

/// Per-request wall-clock budget, as for the Ollama provider.
const TIMEOUT_MS: u64 = 120_000;
/// Provider error bodies are cut to this many characters before they travel.
const MAX_BODY_CHARS: usize = 512;

/// The transport, observer and credential a hosted provider shares.
pub(crate) struct Wire {
    pub transport: Arc<dyn HttpTransport>,
    pub observer: Option<Arc<dyn Observer>>,
    pub provider: &'static str,
    base: String,
    secret: Option<String>,
}

impl Observable for Wire {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

/// The host of `url` (no scheme, userinfo, port or path) — what the
/// `http:<host>` egress grant is keyed on, and the only part of a URL that is
/// ever reported.
pub(crate) fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(stripped) = host_port.strip_prefix('[') {
        return stripped.split(']').next().unwrap_or("").to_owned();
    }
    host_port.split(':').next().unwrap_or("").to_owned()
}

impl Wire {
    pub(crate) fn new(
        transport: Arc<dyn HttpTransport>,
        provider: &'static str,
        base: &str,
        secret: Option<String>,
    ) -> Self {
        Self {
            transport,
            observer: None,
            provider,
            base: base.trim_end_matches('/').to_owned(),
            secret: secret.filter(|s| !s.is_empty()),
        }
    }

    /// `{base}{path}`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn scrub(&self, body: &[u8]) -> String {
        let mut s: String = String::from_utf8_lossy(body)
            .chars()
            .take(MAX_BODY_CHARS)
            .collect();
        if let Some(k) = &self.secret {
            s = s.replace(k.as_str(), "[REDACTED]");
        }
        s
    }

    pub(crate) fn bad_body(&self, detail: impl std::fmt::Display) -> PiiError {
        self.emit(
            &Event::new("model", Level::Error, names::MODEL_PROVIDER_BAD_BODY)
                .field("provider", self.provider)
                .field("detail", detail.to_string()),
        );
        PiiError::Invalid(format!("{}: response body: {detail}", self.provider))
    }

    fn request(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        body: &serde_json::Value,
    ) -> Result<HttpRequest, PiiError> {
        Ok(HttpRequest {
            method: HttpMethod::Post,
            url: url.to_owned(),
            headers,
            body: serde_json::to_vec(body)
                .map_err(|e| PiiError::Internal(format!("{}: encode: {e}", self.provider)))?,
            timeout_ms: TIMEOUT_MS,
        })
    }

    fn transport_err(&self, url: &str, e: PiiError) -> PiiError {
        if matches!(e, PiiError::Denied(_)) {
            self.emit(
                &Event::new("model", Level::Error, names::MODEL_PROVIDER_EGRESS_DENIED)
                    .field("provider", self.provider)
                    .field("host", host_of(url)),
            );
        }
        e
    }

    /// POSTs `body` as JSON and returns the 2xx response.
    pub(crate) fn post(
        &self,
        ctx: &Ctx,
        url: &str,
        headers: Vec<(String, String)>,
        body: &serde_json::Value,
    ) -> Result<HttpResponse, PiiError> {
        let http = self.request(url, headers, body)?;
        let resp = self
            .transport
            .request(ctx, &http)
            .map_err(|e| self.transport_err(url, e))?;
        self.check(url, resp)
    }

    /// Like [`Self::post`], forwarding each body chunk to `on_chunk`.
    pub(crate) fn post_streaming(
        &self,
        ctx: &Ctx,
        url: &str,
        headers: Vec<(String, String)>,
        body: &serde_json::Value,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> {
        let http = self.request(url, headers, body)?;
        let resp = self
            .transport
            .request_streaming(ctx, &http, on_chunk)
            .map_err(|e| self.transport_err(url, e))?;
        self.check(url, resp)
    }

    /// Maps a non-2xx response to [`PiiError::Provider`] (status + scrubbed
    /// body, so `classify` keeps the one retryability verdict) and emits the
    /// matching named event.
    fn check(&self, url: &str, resp: HttpResponse) -> Result<HttpResponse, PiiError> {
        if (200..300).contains(&resp.status) {
            return Ok(resp);
        }
        let failure = ProviderFailure {
            status: Some(resp.status),
            origin: Origin::Provider,
            provider: self.provider.to_owned(),
            message: self.scrub(&resp.body),
        };
        let kind = match classify_provider(&failure) {
            FailoverReason::Auth => Some(names::MODEL_PROVIDER_AUTH_FAILED),
            FailoverReason::RateLimit | FailoverReason::UpstreamRateLimit => {
                Some(names::MODEL_PROVIDER_RATE_LIMITED)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            self.emit(
                &Event::new("model", Level::Warn, kind)
                    .field("provider", self.provider)
                    .field("host", host_of(url))
                    .field("status", u64::from(resp.status)),
            );
        }
        Err(PiiError::Provider(failure))
    }

    /// One partial-token event.
    pub(crate) fn delta(&self, text: &str) {
        self.emit(
            &Event::new("model", Level::Trace, names::MODEL_PROVIDER_STREAM_DELTA)
                .field("provider", self.provider)
                .field("text", text),
        );
    }
}

/// Line-oriented server-sent-events reader: feed raw chunks, get the `data:`
/// payloads of every complete line. A chunk may split a line anywhere; the
/// residual is carried to the next call.
#[derive(Default)]
pub(crate) struct SseReader {
    buf: Vec<u8>,
}

impl SseReader {
    pub(crate) fn feed(&mut self, bytes: &[u8], mut on_data: impl FnMut(&str)) {
        self.buf.extend_from_slice(bytes);
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(d) = line.trim_end().strip_prefix("data:") {
                on_data(d.trim_start());
            }
        }
    }
}
