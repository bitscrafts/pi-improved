//! Posting into a Discord forum (spec 132).
//!
//! A forum accepts no messages (`50008`); a post is a thread (`POST /channels/{forum}/threads`)
//! and the user's forum requires a tag (`40067`). The channel asks Discord what the target is
//! (`GET /channels/{id}`), caches the answer for [`SHAPE_TTL`], and resolves the tag **by name**.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, PiiError};
use pii_core::Ctx;
use serde::Deserialize;

use super::DiscordChannel;

/// Tag a forum post carries when neither the caller nor the config names one.
pub(super) const DEFAULT_FORUM_TAG: &str = "chat";
/// How long a probed channel shape is trusted (tags are renamed on the phone, so it is short).
pub(super) const SHAPE_TTL: Duration = Duration::from_mins(5);
/// Discord's channel type for a forum.
const TYPE_FORUM: u64 = 15;
/// Longest thread name Discord accepts, in chars.
const MAX_THREAD_NAME: usize = 100;
/// Longest message content Discord accepts, in chars.
const MAX_CONTENT: usize = 2000;

/// What a channel id turned out to be.
#[derive(Debug, Clone)]
pub(super) enum Shape {
    /// Anything that takes plain messages.
    Text,
    /// A forum, with its tags as `(name, id)` and its guild id (spec 401 reads the guild's threads).
    Forum {
        tags: Vec<(String, String)>,
        guild: String,
    },
}

/// Probed shapes by channel id, each with the instant it was learned. Only successes are stored.
#[derive(Default)]
pub(super) struct ShapeCache(Mutex<BTreeMap<String, (Instant, Shape)>>);

#[derive(Deserialize)]
struct Probe {
    #[serde(rename = "type")]
    kind: u64,
    #[serde(default)]
    available_tags: Vec<ProbeTag>,
    #[serde(default)]
    guild_id: String,
}

#[derive(Deserialize)]
struct ProbeTag {
    id: String,
    name: String,
}

/// The thread title: the text's first non-empty line, at most [`MAX_THREAD_NAME`] chars.
fn thread_name(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let name: String = line
        .unwrap_or("post")
        .chars()
        .take(MAX_THREAD_NAME)
        .collect();
    name
}

impl DiscordChannel {
    fn probe(&self, ctx: &Ctx, to: &str) -> Result<Shape, PiiError> {
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: format!("{}/channels/{to}", self.base),
            headers: vec![self.auth_header()],
            body: Vec::new(),
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(self.refusal(&format!("cannot tell what channel {to} is:"), &resp));
        }
        let p: Probe = serde_json::from_slice(&resp.body).map_err(|e| {
            PiiError::Unavailable(format!("discord: cannot tell what channel {to} is: {e}"))
        })?;
        Ok(if p.kind == TYPE_FORUM {
            Shape::Forum {
                tags: p
                    .available_tags
                    .into_iter()
                    .map(|t| (t.name, t.id))
                    .collect(),
                guild: p.guild_id,
            }
        } else {
            Shape::Text
        })
    }

    /// The shape of `to`, from the cache while fresh. A failed probe is an error and is not cached.
    pub(super) fn shape_of(&self, ctx: &Ctx, to: &str) -> Result<Shape, PiiError> {
        if let Some((at, shape)) = self.shapes.0.lock().get(to)
            && at.elapsed() < SHAPE_TTL
        {
            return Ok(shape.clone());
        }
        let shape = self.probe(ctx, to)?;
        self.shapes
            .0
            .lock()
            .insert(to.to_owned(), (Instant::now(), shape.clone()));
        Ok(shape)
    }

    /// Creates a thread in forum `to` carrying the tag named `tag` (else the channel's `forum_tag`).
    /// A name that matches no tag is refused naming the forum, the tried names and the available
    /// ones; the tags are re-read once first, since they may have been renamed since the probe.
    pub(super) fn send_forum_post(
        &self,
        ctx: &Ctx,
        to: &str,
        text: &str,
        tag: Option<&str>,
        tags: &[(String, String)],
    ) -> Result<(), PiiError> {
        let wanted = tag.filter(|t| !t.is_empty()).unwrap_or(&self.forum_tag);
        let mut tags = tags.to_vec();
        let mut id = find_tag(&tags, wanted);
        if id.is_none()
            && let Shape::Forum { tags: fresh, .. } = self.probe(ctx, to)?
        {
            tags = fresh;
            id = find_tag(&tags, wanted);
        }
        let Some(id) = id else {
            let names: Vec<&str> = tags.iter().map(|(n, _)| n.as_str()).collect();
            return Err(PiiError::Invalid(format!(
                "discord: forum {to} has no tag named '{wanted}' (tried: '{wanted}'; available: {})",
                names.join(", ")
            )));
        };
        let content: String = text.chars().take(MAX_CONTENT).collect();
        let body = serde_json::to_vec(&serde_json::json!({
            "name": thread_name(text),
            "message": { "content": content },
            "applied_tags": [id],
        }))
        .map_err(|e| PiiError::Internal(format!("discord: encode: {e}")))?;
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: format!("{}/channels/{to}/threads", self.base),
            headers: self.json_headers(),
            body,
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(self.refusal("forum post", &resp));
        }
        Ok(())
    }
}

/// The id of the tag called `name` (case-insensitive: the user types these on a phone).
fn find_tag<'a>(tags: &'a [(String, String)], name: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, id)| id.as_str())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::sync::Arc;

    use parking_lot::Mutex;
    use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
    use pii_core::{Budget, CapSet, Ctx, HttpTransport};
    use serde_json::{Value, json};

    use super::{DiscordChannel, ShapeCache};
    use crate::Channel;
    use crate::discord::inbound::Inbound;
    use crate::post::{ChannelHub, PostOutcome, PostRequest};

    /// Routes by method + path suffix; records every request.
    struct Routes {
        kind: u64,
        create: (u16, Vec<u8>),
        seen: Mutex<Vec<HttpRequest>>,
    }

    impl Routes {
        fn new(kind: u64) -> Arc<Self> {
            Self::with_create(kind, 200, b"{}".to_vec())
        }
        fn with_create(kind: u64, status: u16, body: Vec<u8>) -> Arc<Self> {
            Arc::new(Self {
                kind,
                create: (status, body),
                seen: Mutex::new(Vec::new()),
            })
        }
        fn posts(&self) -> Vec<HttpRequest> {
            self.seen
                .lock()
                .iter()
                .filter(|r| r.method == HttpMethod::Post)
                .cloned()
                .collect()
        }
    }

    impl HttpTransport for Routes {
        fn request(&self, _: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
            self.seen.lock().push(req.clone());
            let reply = |status, body: Vec<u8>| {
                Ok(HttpResponse {
                    status,
                    headers: Vec::new(),
                    body,
                })
            };
            if req.method == HttpMethod::Get {
                let tags = json!([{"id": "11", "name": "chat"}, {"id": "22", "name": "status"},
                                  {"id": "33", "name": "alert"}]);
                let tags = if self.kind == 15 { tags } else { json!([]) };
                return reply(
                    200,
                    json!({"type": self.kind, "available_tags": tags})
                        .to_string()
                        .into_bytes(),
                );
            }
            if req.url.ends_with("/threads") {
                return reply(self.create.0, self.create.1.clone());
            }
            reply(200, b"{}".to_vec())
        }
    }

    fn channel(t: &Arc<Routes>, forum_tag: &str) -> DiscordChannel {
        DiscordChannel {
            name: String::from("discord1"),
            transport: Arc::clone(t) as Arc<dyn HttpTransport>,
            base: String::from("http://fake.test"),
            token: String::from("tok-secret"),
            channel_id: String::from("ch1"),
            timeout_ms: 10_000,
            forum_tag: forum_tag.to_owned(),
            shapes: ShapeCache::default(),
            inbound: Inbound::default(),
        }
    }

    fn ctx() -> Ctx {
        Ctx::root(CapSet::empty(), Budget::unlimited())
    }

    fn body(r: &HttpRequest) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn a_text_channel_still_posts_a_plain_message() {
        let t = Routes::new(0);
        channel(&t, "chat")
            .send_tagged(&ctx(), "c9", "hi", Some("alert"))
            .unwrap();
        let p = t.posts();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].url, "http://fake.test/channels/c9/messages");
        assert_eq!(body(&p[0]), json!({"content": "hi"}));
    }

    #[test]
    fn a_forum_post_creates_a_thread_with_the_configured_tag() {
        let t = Routes::new(15);
        channel(&t, "status")
            .send_tagged(&ctx(), "f1", "Digest\nbody line", None)
            .unwrap();
        let p = t.posts();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].url, "http://fake.test/channels/f1/threads");
        let b = body(&p[0]);
        assert_eq!(b["applied_tags"], json!(["22"]));
        assert_eq!(b["name"], "Digest");
        assert_eq!(b["message"]["content"], "Digest\nbody line");
    }

    #[test]
    fn a_forum_post_uses_the_per_post_tag() {
        let t = Routes::new(15);
        channel(&t, "status")
            .send_tagged(&ctx(), "f1", "x", Some("alert"))
            .unwrap();
        assert_eq!(body(&t.posts()[0])["applied_tags"], json!(["33"]));
    }

    #[test]
    fn a_forum_with_no_resolvable_tag_is_refused_and_counts() {
        let t = Routes::new(15);
        let hub = ChannelHub::default();
        hub.insert(Arc::new(channel(&t, "nope")), ctx());
        let e = hub
            .post(&PostRequest {
                channel: "discord1",
                to: "f1",
                text: "x",
                key: None,
                tag: None,
                timeout_ms: 2000,
            })
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("f1") && e.contains("nope") && e.contains("chat") && e.contains("status"),
            "{e}"
        );
        assert!(t.posts().is_empty(), "nothing was created");
        assert_eq!(hub.counts().refused, 1);
    }

    #[test]
    fn the_providers_refusal_is_preserved_and_scrubbed() {
        let b = br#"{"message": "A tag is required", "code": 40067, "echo": "tok-secret"}"#;
        let t = Routes::with_create(15, 400, b.to_vec());
        let Err(PiiError::Unavailable(m)) =
            channel(&t, "chat").send_tagged(&ctx(), "f1", "x", None)
        else {
            panic!("expected Unavailable")
        };
        assert!(
            m.contains("40067") && m.contains("A tag is required"),
            "{m}"
        );
        assert!(!m.contains("tok-secret"), "scrubbed: {m}");
        assert!(m.len() < 400, "bounded");
    }

    #[test]
    fn two_sends_with_one_key_create_one_thread() {
        let t = Routes::new(15);
        let hub = ChannelHub::default();
        hub.insert(Arc::new(channel(&t, "chat")), ctx());
        let req = PostRequest {
            channel: "discord1",
            to: "f1",
            text: "x",
            key: Some("k"),
            tag: None,
            timeout_ms: 2000,
        };
        assert_eq!(hub.post(&req).unwrap(), PostOutcome::Posted);
        assert_eq!(hub.post(&req).unwrap(), PostOutcome::Duplicate);
        assert_eq!(t.posts().len(), 1);
    }

    #[test]
    fn a_failed_probe_is_a_refusal_not_a_plain_message_and_is_not_cached() {
        struct Down;
        impl HttpTransport for Down {
            fn request(&self, _: &Ctx, r: &HttpRequest) -> Result<HttpResponse, PiiError> {
                assert_eq!(r.method, HttpMethod::Get, "no POST after a failed probe");
                Ok(HttpResponse {
                    status: 500,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }
        let t = Routes::new(0);
        let mut ch = channel(&t, "chat");
        ch.transport = Arc::new(Down);
        assert!(ch.send(&ctx(), "f1", "x").is_err());
        assert!(ch.shapes.0.lock().is_empty());
    }

    #[test]
    fn thread_names_are_bounded_to_the_first_line() {
        assert_eq!(super::thread_name("\n  first \nsecond"), "first");
        assert_eq!(super::thread_name(&"é".repeat(300)).chars().count(), 100);
    }
}
