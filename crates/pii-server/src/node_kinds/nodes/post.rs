//! `post` node kind (spec 128): the egress to a channel, through the daemon's
//! [`ChannelHub`] and `Channel::send` — never a Discord client of its own.
//!
//! Config: `channel` (a `[[channel]]` name), `to` (destination id/topic; literal or a
//! `$.`-expression), and exactly one of `text` (literal or `$.`-expression) or `template`
//! (`${path}` substitution, paths as in `$.` expressions plus `${run}`). Optional `key`
//! (idempotence: a retried step that already delivered posts nothing; may use `${…}`),
//! `tag` (a forum channel's tag, by name; ignored by other channels), `timeout_ms` (default 5000, max 30000) and `max_bytes` (default 1900, max 2000: longer text
//! is cut and says so). The node needs the `post` capability on the graph **and** the node.
//!
//! No `err` port: a refusal or provider failure fails the node with the channel's own
//! message (retryable ones are retried by the engine; the key keeps a retry from double-posting).
//! `ok` value: `{channel, to, bytes, duplicate, truncated, key}` (`key` is null without one).

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_channels::post::{ChannelHub, PostOutcome, PostRequest};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::{eval, eval_or_literal};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};
use serde_json::{Value, json};

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const MAX_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_BYTES: usize = 1900;
const HARD_MAX_BYTES: usize = 2000;
const MAX_KEY_BYTES: usize = 200;

struct PostNode {
    hub: Arc<ChannelHub>,
    log: Arc<dyn Observer>,
    channel: String,
    to: Value,
    text: Option<Value>,
    template: Option<String>,
    key: Option<String>,
    tag: Option<String>,
    timeout_ms: u64,
    max_bytes: usize,
}

/// Registered by [`crate::Engine::new`]; every `post` node shares the hub the daemon fills.
pub struct PostFactory {
    /// The daemon's channels.
    pub hub: Arc<ChannelHub>,
    /// Receives `channel.posted` / `channel.post_refused`.
    pub log: Arc<dyn Observer>,
}

/// Renders `${path}` placeholders against `ec` (`input`, `payload`, `nodes`, `run`).
fn render(tpl: &str, ec: &Value) -> Result<String, PiiError> {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find("${") {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i + 2..).unwrap_or_default();
        let end = after
            .find('}')
            .ok_or_else(|| PiiError::Invalid(format!("post: unterminated '${{' in '{tpl}'")))?;
        let path = after.get(..end).unwrap_or_default().trim();
        match eval(ec, &format!("$.{path}"))? {
            Value::Null => {}
            Value::String(s) => out.push_str(&s),
            other => out.push_str(&other.to_string()),
        }
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// Cuts `s` to at most `max` bytes on a char boundary; true when it was cut.
fn bound(s: &mut String, max: usize) -> bool {
    if s.len() <= max {
        return false;
    }
    let mut end = max.saturating_sub(3);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("...");
    true
}

impl PostNode {
    fn text(&self, ec: &Value) -> Result<String, PiiError> {
        let raw = match (&self.template, &self.text) {
            (Some(t), _) => render(t, ec)?,
            (None, Some(v)) => match eval_or_literal(ec, v)? {
                Value::String(s) => s,
                Value::Null => String::new(),
                other => other.to_string(),
            },
            (None, None) => String::new(),
        };
        Ok(raw)
    }

    fn post(&self, ctx: &Ctx, input: &NodeInput) -> Result<Value, PiiError> {
        if !ctx.caps().kinds.contains(CapKind::POST) {
            self.hub.note_refused();
            return Err(PiiError::Denied(format!(
                "post: channel {} not granted (the graph and the node need the post capability)",
                self.channel
            )));
        }
        let ec = json!({
            "input": input.input, "payload": input.payload, "nodes": input.nodes,
            "run": ctx.trace().run.to_string(),
        });
        let to = match eval_or_literal(&ec, &self.to)? {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            other => {
                return Err(PiiError::Invalid(format!(
                    "post: 'to' must resolve to a string, got {other}"
                )));
            }
        };
        let mut text = self.text(&ec)?;
        let truncated = bound(&mut text, self.max_bytes);
        let key = self.key.as_deref().map(|k| render(k, &ec)).transpose()?;
        if key.as_ref().is_some_and(|k| k.len() > MAX_KEY_BYTES) {
            return Err(PiiError::Invalid(format!(
                "post: 'key' is longer than {MAX_KEY_BYTES} bytes"
            )));
        }
        let outcome = self.hub.post(&PostRequest {
            channel: &self.channel,
            to: &to,
            text: &text,
            key: key.as_deref(),
            tag: self.tag.as_deref(),
            timeout_ms: self.timeout_ms,
        })?;
        let duplicate = outcome == PostOutcome::Duplicate;
        if !duplicate {
            self.log.event(
                &Event::new("channel", Level::Info, names::CHANNEL_POSTED)
                    .field("channel", self.channel.as_str())
                    .field("destination", to.as_str())
                    .field("bytes", text.len() as u64)
                    .in_run(pii_observe::RunId(ctx.trace().run.to_string())),
            );
        }
        Ok(json!({
            "channel": self.channel, "to": to, "bytes": text.len(),
            "duplicate": duplicate, "truncated": truncated, "key": key,
        }))
    }
}

impl GraphNode for PostNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        match self.post(ctx, input) {
            Ok(value) => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value,
            }),
            Err(e) => {
                self.log.event(
                    &Event::new("channel", Level::Warn, names::CHANNEL_POST_REFUSED)
                        .field("channel", self.channel.as_str())
                        .field("reason", e.to_string())
                        .in_run(pii_observe::RunId(ctx.trace().run.to_string())),
                );
                Err(e)
            }
        }
    }
}

impl NodeFactory for PostFactory {
    fn kind(&self) -> &'static str {
        "post"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::POST
    }

    fn ports(&self, _cfg: &Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, cfg: &Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let str_of = |k: &str| {
            cfg.get(k)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| PiiError::Invalid(format!("post: '{k}' must be a string")))
                })
                .transpose()
        };
        let channel = str_of("channel")?
            .filter(|c| !c.is_empty())
            .ok_or_else(|| PiiError::Invalid(String::from("post node requires 'channel'")))?;
        let to = cfg
            .get("to")
            .cloned()
            .ok_or_else(|| PiiError::Invalid(String::from("post node requires 'to'")))?;
        let (text, template) = (cfg.get("text").cloned(), str_of("template")?);
        if text.is_some() == template.is_some() {
            return Err(PiiError::Invalid(String::from(
                "post: give exactly one of 'text' and 'template'",
            )));
        }
        // A malformed template is a build error, not a run-time surprise.
        let key = str_of("key")?;
        for t in [template.as_deref(), key.as_deref()].into_iter().flatten() {
            render(t, &json!({}))?;
        }
        let timeout_ms = cfg
            .get("timeout_ms")
            .map_or(Some(DEFAULT_TIMEOUT_MS), Value::as_u64)
            .filter(|t| (1..=MAX_TIMEOUT_MS).contains(t))
            .ok_or_else(|| {
                PiiError::Invalid(format!("post: 'timeout_ms' must be 1..={MAX_TIMEOUT_MS}"))
            })?;
        let max_bytes = cfg
            .get("max_bytes")
            .map_or(Some(DEFAULT_MAX_BYTES as u64), Value::as_u64)
            .and_then(|b| usize::try_from(b).ok())
            .filter(|b| (16..=HARD_MAX_BYTES).contains(b))
            .ok_or_else(|| {
                PiiError::Invalid(format!("post: 'max_bytes' must be 16..={HARD_MAX_BYTES}"))
            })?;
        Ok(Box::new(PostNode {
            hub: Arc::clone(&self.hub),
            log: Arc::clone(&self.log),
            channel,
            to,
            text,
            template,
            key,
            tag: str_of("tag")?,
            timeout_ms,
            max_bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::sync::atomic::{AtomicU64, Ordering};

    use parking_lot::Mutex;
    use pii_channels::{Channel, Incoming};
    use pii_core::{Budget, CapSet};
    use pii_observe::Recorder;

    use super::*;

    struct Mock {
        delivered: Mutex<Vec<(String, String)>>,
        tags: Mutex<Vec<Option<String>>>,
        fail: AtomicU64,
    }

    impl Channel for Mock {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn kind(&self) -> &'static str {
            "mock"
        }
        fn poll(&self, _: &Ctx, _: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
            Ok(Vec::new())
        }
        fn send_tagged(
            &self,
            c: &Ctx,
            to: &str,
            text: &str,
            tag: Option<&str>,
        ) -> Result<(), PiiError> {
            self.tags.lock().push(tag.map(str::to_owned));
            self.send(c, to, text)
        }
        fn send(&self, _: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
            if self.fail.load(Ordering::SeqCst) > 0 {
                self.fail.fetch_sub(1, Ordering::SeqCst);
                return Err(PiiError::Unavailable(String::from(
                    r#"discord: send status 403: {"message": "Missing Permissions", "code": 50013}"#,
                )));
            }
            self.delivered.lock().push((to.to_owned(), text.to_owned()));
            Ok(())
        }
    }

    struct Rig {
        node: Box<dyn GraphNode>,
        mock: Arc<Mock>,
        hub: Arc<ChannelHub>,
        log: Arc<Recorder>,
    }

    fn rig(cfg: &Value, fail: u64) -> Rig {
        let mock = Arc::new(Mock {
            delivered: Mutex::new(Vec::new()),
            tags: Mutex::new(Vec::new()),
            fail: AtomicU64::new(fail),
        });
        let hub = Arc::new(ChannelHub::default());
        hub.insert(
            Arc::clone(&mock) as Arc<dyn Channel>,
            Ctx::root(CapSet::empty(), Budget::unlimited()),
        );
        let log = Arc::new(Recorder::new());
        let node = PostFactory {
            hub: Arc::clone(&hub),
            log: Arc::clone(&log) as Arc<dyn Observer>,
        }
        .build(cfg)
        .unwrap();
        Rig {
            node,
            mock,
            hub,
            log,
        }
    }

    fn cfg() -> Value {
        json!({"channel": "mock", "to": "$.input.chat", "template": "[${payload.class}] ${input.text}"})
    }

    fn ctx(kinds: CapKind) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = kinds;
        Ctx::root(caps, Budget::unlimited())
    }

    fn input() -> NodeInput {
        NodeInput {
            input: json!({"chat": "thread-9", "text": "is it up?"}),
            payload: json!({"class": "question"}),
            nodes: json!({}),
        }
    }

    fn ok_value(o: Outcome) -> Value {
        match o {
            Outcome::Next { port, value } => {
                assert_eq!(port.as_str(), "ok");
                value
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn posts_the_templated_text_and_journals_the_destination() {
        let r = rig(&cfg(), 0);
        let v = ok_value(r.node.run(&ctx(CapKind::POST), &input()).unwrap());
        assert_eq!(
            *r.mock.delivered.lock(),
            vec![("thread-9".into(), "[question] is it up?".into())]
        );
        assert_eq!(v["to"], "thread-9");
        assert_eq!(v["bytes"], 20);
        assert_eq!(v["duplicate"], false);
        let ev = r.log.events_named(names::CHANNEL_POSTED);
        assert_eq!(ev.len(), 1);
        assert!(
            ev[0]
                .fields
                .get("destination")
                .unwrap()
                .to_string()
                .contains("thread-9")
        );
        assert_eq!(r.hub.counts().posted, 1);
    }

    #[test]
    fn refused_without_the_post_capability_by_name_and_nothing_is_sent() {
        let r = rig(&cfg(), 0);
        let e = r.node.run(&ctx(CapKind::NET), &input()).unwrap_err();
        assert_eq!(
            e.to_string(),
            "capability denied: post: channel mock not granted (the graph and the node need the post capability)"
        );
        assert!(r.mock.delivered.lock().is_empty());
        assert_eq!(r.log.events_named(names::CHANNEL_POST_REFUSED).len(), 1);
        assert_eq!(r.hub.counts().refused, 1);
    }

    #[test]
    fn a_provider_refusal_fails_the_node_with_the_message_intact() {
        let r = rig(&cfg(), 1);
        let e = r.node.run(&ctx(CapKind::POST), &input()).unwrap_err();
        assert!(
            e.to_string().contains("50013") && e.to_string().contains("Missing Permissions"),
            "{e}"
        );
        assert!(e.is_retryable(), "the engine's retry policy still applies");
        let ev = r.log.events_named(names::CHANNEL_POST_REFUSED);
        assert!(
            ev[0]
                .fields
                .get("reason")
                .unwrap()
                .to_string()
                .contains("50013")
        );
    }

    #[test]
    fn the_post_node_forwards_its_tag() {
        let mut c = cfg();
        c["tag"] = json!("status");
        let r = rig(&c, 0);
        r.node.run(&ctx(CapKind::POST), &input()).unwrap();
        assert_eq!(*r.mock.tags.lock(), vec![Some(String::from("status"))]);
        let r = rig(&cfg(), 0);
        r.node.run(&ctx(CapKind::POST), &input()).unwrap();
        assert_eq!(*r.mock.tags.lock(), vec![None]);
    }

    #[test]
    fn a_keyed_step_run_twice_posts_once() {
        let mut c = cfg();
        c["key"] = json!("triage-${run}");
        let r = rig(&c, 0);
        let x = ctx(CapKind::POST);
        assert_eq!(
            ok_value(r.node.run(&x, &input()).unwrap())["duplicate"],
            false
        );
        let v = ok_value(r.node.run(&x, &input()).unwrap());
        assert_eq!(v["duplicate"], true);
        assert_eq!(r.mock.delivered.lock().len(), 1);
        assert_eq!(r.log.events_named(names::CHANNEL_POSTED).len(), 1);
    }

    #[test]
    fn a_failed_attempt_then_a_retry_posts_once() {
        let mut c = cfg();
        c["key"] = json!("k");
        let r = rig(&c, 1);
        let x = ctx(CapKind::POST);
        assert!(r.node.run(&x, &input()).is_err());
        assert_eq!(
            ok_value(r.node.run(&x, &input()).unwrap())["duplicate"],
            false
        );
        assert_eq!(r.mock.delivered.lock().len(), 1);
    }

    #[test]
    fn long_text_is_cut_and_says_so() {
        let r = rig(
            &json!({"channel": "mock", "to": "t", "text": "$.input.text", "max_bytes": 20}),
            0,
        );
        let mut i = input();
        i.input["text"] = json!("é".repeat(40));
        let v = ok_value(r.node.run(&ctx(CapKind::POST), &i).unwrap());
        assert_eq!(v["truncated"], true);
        assert!(v["bytes"].as_u64().unwrap() <= 20);
    }

    #[test]
    fn a_bad_config_is_refused_at_build() {
        let f = PostFactory {
            hub: Arc::new(ChannelHub::default()),
            log: Arc::new(Recorder::new()),
        };
        for bad in [
            json!({"to": "t", "text": "x"}),
            json!({"channel": "c", "text": "x"}),
            json!({"channel": "c", "to": "t"}),
            json!({"channel": "c", "to": "t", "text": "x", "template": "y"}),
            json!({"channel": "c", "to": "t", "template": "${oops"}),
            json!({"channel": "c", "to": "t", "text": "x", "timeout_ms": 0}),
            json!({"channel": "c", "to": "t", "text": "x", "max_bytes": 5000}),
        ] {
            assert!(f.build(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_unconfigured_channel_is_a_named_failure() {
        let r = rig(&json!({"channel": "ghost", "to": "t", "text": "x"}), 0);
        let e = r.node.run(&ctx(CapKind::POST), &input()).unwrap_err();
        assert!(
            e.to_string().contains("ghost") && e.to_string().contains("not configured"),
            "{e}"
        );
    }
}
