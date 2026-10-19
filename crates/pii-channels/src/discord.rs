//! `DiscordChannel` (F6.3): HTTP(S) against a Discord-API-compatible endpoint.
//!
//! TLS is available now: the transport the daemon wires in (`pii-net`
//! `NetService`) handles `https://` URLs (rustls, spec 17); the default
//! `DEFAULT_API_BASE` below is the real API (`discord.com/api/v10`); `PII_DISCORD_API_BASE` may point at
//! either scheme (tests point it at a plaintext loopback fake).

use std::sync::Arc;

use pii_abi::{HttpMethod, HttpRequest, PiiError};
use pii_core::{Ctx, HttpTransport};
use serde::Deserialize;

use crate::{Channel, Incoming};

mod forum;

/// Default Discord API base: the versioned REST API on `discord.com`.
///
/// (`api.discord.com` does not
/// resolve, so the earlier default could never reach Discord.) Every request URL is
/// `{base}/channels/{id}...`: poll, plain send, the forum shape probe (which also reads the tags)
/// and the thread create. Override with `PII_DISCORD_API_BASE`; tests point it at a loopback fake.
pub const DEFAULT_API_BASE: &str = "https://discord.com/api/v10";

/// The `host:port` of [`DEFAULT_API_BASE`]: the one egress target the shipped configuration sanctions
/// without an operator entry (spec 137). A pin test keeps it equal to `url_target(DEFAULT_API_BASE)`.
pub const DEFAULT_EGRESS_HOST: &str = "discord.com:443";

/// One message as Discord's API shapes it.
#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    content: String,
    author: DiscordAuthor,
}

#[derive(Debug, Deserialize)]
struct DiscordAuthor {
    #[serde(default)]
    username: String,
}

/// A Discord channel bound to one guild channel id (F6.3).
pub struct DiscordChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    base: String,
    /// Bot token (`Authorization: Bot <token>`). Never logged (F6.8/N9).
    token: String,
    channel_id: String,
    timeout_ms: u64,
    /// Tag name a forum post carries when the caller gave none (`PII_DISCORD_FORUM_TAG`).
    forum_tag: String,
    /// Channel shapes already probed, each valid for [`forum::SHAPE_TTL`].
    shapes: forum::ShapeCache,
}

impl DiscordChannel {
    /// Builds a channel from the phase's env surface (N10):
    /// `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`, `PII_DISCORD_API_BASE`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when a required env var is missing.
    pub fn from_env(name: &str, transport: Arc<dyn HttpTransport>) -> Result<Self, PiiError> {
        Self::from_lookup(name, transport, pii_core::env_var)
    }

    /// [`Self::from_env`] over an arbitrary variable lookup (so tests need no process-global env).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when a required variable is missing.
    pub fn from_lookup(
        name: &str,
        transport: Arc<dyn HttpTransport>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, PiiError> {
        let token = env("PII_DISCORD_TOKEN")
            .ok_or_else(|| PiiError::Invalid(String::from("PII_DISCORD_TOKEN is not set")))?;
        let channel_id = env("PII_DISCORD_CHANNEL_ID")
            .ok_or_else(|| PiiError::Invalid(String::from("PII_DISCORD_CHANNEL_ID is not set")))?;
        let base = env("PII_DISCORD_API_BASE").unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            name: name.to_owned(),
            transport,
            base: base.trim_end_matches('/').to_owned(),
            token,
            channel_id,
            timeout_ms: 10_000,
            forum_tag: env("PII_DISCORD_FORUM_TAG")
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| String::from(forum::DEFAULT_FORUM_TAG)),
            shapes: forum::ShapeCache::default(),
        })
    }

    fn auth_header(&self) -> (String, String) {
        (String::from("Authorization"), format!("Bot {}", self.token))
    }

    fn messages_url(&self, after: Option<u64>) -> String {
        after.map_or_else(
            || {
                format!(
                    "{}/channels/{}/messages?limit=20",
                    self.base, self.channel_id
                )
            },
            |a| {
                format!(
                    "{}/channels/{}/messages?after={a}&limit=20",
                    self.base, self.channel_id
                )
            },
        )
    }
}

impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "discord"
    }

    fn egress_hosts(&self) -> Vec<String> {
        crate::url_target(&self.base).into_iter().collect()
    }

    /// The base can come from `PII_DISCORD_API_BASE`, so every host but the shipped default needs
    /// the configuration's sanction (spec 137).
    fn env_derived_hosts(&self) -> Vec<String> {
        let mut hosts = self.egress_hosts();
        hosts.retain(|h| h != DEFAULT_EGRESS_HOST);
        hosts
    }

    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: self.messages_url(cursor),
            headers: vec![self.auth_header()],
            body: Vec::new(),
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(self.refusal("poll", &resp));
        }
        let msgs: Vec<DiscordMessage> = serde_json::from_slice(&resp.body)
            .map_err(|e| PiiError::Unavailable(format!("discord: bad poll body: {e}")))?;
        let mut out = Vec::with_capacity(msgs.len());
        for m in msgs {
            let cursor = m.id.parse::<u64>().map_err(|e| {
                PiiError::Unavailable(format!("discord: non-numeric message id '{}': {e}", m.id))
            })?;
            out.push(Incoming {
                channel: self.name.clone(),
                chat: self.channel_id.clone(),
                user: m.author.username,
                text: m.content,
                cursor,
            });
        }
        // Discord returns newest-first; the router advances the cursor
        // monotonically, so oldest-first is the order it must see.
        out.reverse();
        Ok(out)
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        self.send_tagged(ctx, to, text, None)
    }

    fn send_tagged(
        &self,
        ctx: &Ctx,
        to: &str,
        text: &str,
        tag: Option<&str>,
    ) -> Result<(), PiiError> {
        // A forum takes threads, not messages: ask Discord which this is (spec 132 R1).
        match self.shape_of(ctx, to)? {
            forum::Shape::Text => self.send_message(ctx, to, text),
            forum::Shape::Forum(tags) => self.send_forum_post(ctx, to, text, tag, &tags),
        }
    }
}

impl DiscordChannel {
    fn json_headers(&self) -> Vec<(String, String)> {
        vec![
            self.auth_header(),
            (
                String::from("Content-Type"),
                String::from("application/json"),
            ),
        ]
    }

    /// The provider's refusal as an error: Discord says *why* in the body (`50013 Missing
    /// Permissions`); a bare status would swallow it. Bounded, and the token scrubbed.
    fn refusal(&self, what: &str, resp: &pii_abi::HttpResponse) -> PiiError {
        let body = String::from_utf8_lossy(&resp.body);
        let why: String = crate::scrub(body.trim(), &self.token)
            .chars()
            .take(300)
            .collect();
        let sep = if why.is_empty() { "" } else { ": " };
        PiiError::Unavailable(format!("discord: {what} status {}{sep}{why}", resp.status))
    }

    fn send_message(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let body = serde_json::to_vec(&serde_json::json!({ "content": text }))
            .map_err(|e| PiiError::Internal(format!("discord: encode: {e}")))?;
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: format!("{}/channels/{to}/messages", self.base),
            headers: self.json_headers(),
            body,
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(self.refusal("send", &resp));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use parking_lot::Mutex;
    use std::sync::Arc;

    use pii_abi::{HttpRequest, HttpResponse, PiiError};
    use pii_core::{Budget, CapSet, Ctx};

    use super::DiscordChannel;
    use crate::Channel;

    /// Fake transport: returns a canned response and records the last request.
    struct FakeTransport {
        resp: Result<HttpResponse, PiiError>,
        last: Mutex<Option<HttpRequest>>,
    }

    impl FakeTransport {
        fn ok(body: Vec<u8>) -> Self {
            Self {
                resp: Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body,
                }),
                last: Mutex::new(None),
            }
        }
        fn status(status: u16) -> Self {
            Self::status_body(status, Vec::new())
        }
        fn status_body(status: u16, body: Vec<u8>) -> Self {
            Self {
                resp: Ok(HttpResponse {
                    status,
                    headers: Vec::new(),
                    body,
                }),
                last: Mutex::new(None),
            }
        }
    }

    impl pii_core::HttpTransport for FakeTransport {
        fn request(&self, _ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
            // The shape probe (`GET /channels/{id}`) answers "text channel".
            if req.method == pii_abi::HttpMethod::Get && !req.url.contains("/messages") {
                return Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: br#"{"type":0}"#.to_vec(),
                });
            }
            *self.last.lock() = Some(req.clone());
            self.resp.clone()
        }
    }

    fn ctx() -> Ctx {
        Ctx::root(CapSet::empty(), Budget::unlimited())
    }

    /// Builds a channel directly (private fields are visible in this
    /// module); avoids process-global env mutation, so no `set_var`.
    fn channel(fake: FakeTransport) -> DiscordChannel {
        DiscordChannel {
            name: String::from("discord1"),
            transport: Arc::new(fake),
            base: String::from("http://fake.test"),
            token: String::from("tok"),
            channel_id: String::from("ch1"),
            timeout_ms: 10_000,
            forum_tag: String::from("chat"),
            shapes: super::forum::ShapeCache::default(),
        }
    }

    fn body_of(msgs: &[(&str, &str, &str)]) -> Vec<u8> {
        let arr: Vec<serde_json::Value> = msgs
            .iter()
            .map(|(id, content, user)| {
                serde_json::json!({ "id": id, "content": content, "author": { "username": user } })
            })
            .collect();
        serde_json::to_vec(&arr).unwrap()
    }

    /// Pin (spec 132): the old default, `api.discord.com`, does not resolve, so no URL built from it
    /// ever reached Discord while every fake-base test passed.
    #[test]
    fn the_default_base_is_the_real_discord_api() {
        assert_eq!(super::DEFAULT_API_BASE, "https://discord.com/api/v10");
        let mut ch = channel(FakeTransport::ok(Vec::new()));
        ch.base = super::DEFAULT_API_BASE.to_owned();
        let url = ch.messages_url(None);
        assert!(
            url.starts_with("https://discord.com/api/v10/channels/"),
            "{url}"
        );
    }

    #[test]
    fn the_default_egress_host_is_the_default_base_s_target() {
        assert_eq!(
            crate::url_target(super::DEFAULT_API_BASE).as_deref(),
            Some(super::DEFAULT_EGRESS_HOST)
        );
    }

    #[test]
    fn only_a_non_default_base_is_env_derived() {
        let mut ch = channel(FakeTransport::ok(Vec::new()));
        ch.base = super::DEFAULT_API_BASE.to_owned();
        assert!(ch.env_derived_hosts().is_empty());
        ch.base = String::from("https://evil.example");
        assert_eq!(
            ch.env_derived_hosts(),
            vec![String::from("evil.example:443")]
        );
    }

    #[test]
    fn egress_follows_the_configured_base() {
        let mut ch = channel(FakeTransport::ok(Vec::new()));
        ch.base = super::DEFAULT_API_BASE.to_owned();
        assert_eq!(ch.egress_hosts(), vec![String::from("discord.com:443")]);
        ch.base = String::from("http://127.0.0.1:9000");
        assert_eq!(ch.egress_hosts(), vec![String::from("127.0.0.1:9000")]);
    }

    #[test]
    fn poll_maps_and_reverses_discord_order() {
        let ch = channel(FakeTransport::ok(body_of(&[
            ("30", "newest", "carol"),
            ("20", "middle", "bob"),
            ("10", "oldest", "alice"),
        ])));
        let got = ch.poll(&ctx(), None).unwrap();
        let cursors: Vec<u64> = got.iter().map(|m| m.cursor).collect();
        assert_eq!(cursors, vec![10, 20, 30], "oldest first");
        let oldest = got.first().unwrap();
        assert_eq!(oldest.user, "alice");
        assert_eq!(oldest.text, "oldest");
        assert_eq!(oldest.chat, "ch1");
        assert_eq!(oldest.channel, "discord1");
    }

    #[test]
    fn poll_non_success_status_is_unavailable() {
        let ch = channel(FakeTransport::status(500));
        assert!(matches!(
            ch.poll(&ctx(), None),
            Err(PiiError::Unavailable(_))
        ));
    }

    #[test]
    fn poll_malformed_body_is_unavailable() {
        let ch = channel(FakeTransport::ok(b"not json".to_vec()));
        assert!(matches!(
            ch.poll(&ctx(), None),
            Err(PiiError::Unavailable(_))
        ));
    }

    #[test]
    fn poll_non_numeric_id_is_unavailable() {
        let ch = channel(FakeTransport::ok(body_of(&[("not-a-number", "hi", "u")])));
        assert!(matches!(
            ch.poll(&ctx(), None),
            Err(PiiError::Unavailable(_))
        ));
    }

    #[test]
    fn send_success() {
        let ch = channel(FakeTransport::ok(Vec::new()));
        ch.send(&ctx(), "ch1", "hello").unwrap();
    }

    #[test]
    fn send_non_success_status_is_unavailable() {
        let ch = channel(FakeTransport::status(403));
        assert!(matches!(
            ch.send(&ctx(), "ch1", "hello"),
            Err(PiiError::Unavailable(_))
        ));
    }

    #[test]
    fn send_refusal_keeps_the_provider_reason_bounded_and_scrubbed() {
        let body = br#"{"message": "Missing Permissions", "code": 50013}"#.to_vec();
        let ch = channel(FakeTransport::status_body(403, body));
        let Err(PiiError::Unavailable(m)) = ch.send(&ctx(), "ch1", "hello") else {
            panic!("expected Unavailable")
        };
        assert!(
            m.contains("403") && m.contains("50013") && m.contains("Missing Permissions"),
            "{m}"
        );
        let long = format!("tok {}", "y".repeat(2000)).into_bytes();
        let ch = channel(FakeTransport::status_body(500, long));
        let Err(PiiError::Unavailable(m)) = ch.send(&ctx(), "ch1", "hello") else {
            panic!("expected Unavailable")
        };
        assert!(!m.contains("tok y"), "token scrubbed: {m}");
        assert!(m.len() < 400, "bounded: {}", m.len());
    }
}
