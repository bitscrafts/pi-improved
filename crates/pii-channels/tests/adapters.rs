//! Spec 112: the telegram, matrix and webhook adapters, offline (mock
//! transport, no network).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_channels::matrix::MatrixChannel;
use pii_channels::observe::ObservedChannel;
use pii_channels::telegram::TelegramChannel;
use pii_channels::webhook::{WebhookChannel, WebhookConfig};
use pii_channels::{Channel, is_auth_rejected};
use pii_core::{Budget, CapSet, Ctx, HttpTransport};
use pii_observe::{Recorder, names};

/// Scripted transport: pops canned responses, records every request.
struct Mock {
    replies: Mutex<Vec<Result<HttpResponse, PiiError>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Mock {
    fn new(replies: Vec<Result<HttpResponse, PiiError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl Mock {
    fn seen(&self) -> Vec<HttpRequest> {
        self.seen.lock().clone()
    }
}

impl HttpTransport for Mock {
    fn request(&self, _ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
        self.seen.lock().push(req.clone());
        let mut r = self.replies.lock();
        if r.is_empty() {
            return Ok(resp(200, "{}"));
        }
        r.remove(0)
    }
}

fn resp(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

const TOKEN: &str = "123:SECRET-TOKEN";

fn telegram(mock: &Arc<Mock>) -> TelegramChannel {
    TelegramChannel::new(
        "tg",
        Arc::clone(mock) as Arc<dyn HttpTransport>,
        Some(TOKEN.to_owned()),
        Some("http://tg.test"),
    )
}

#[test]
fn telegram_poll_maps_updates_and_advances_with_offset() {
    let mock = Mock::new(vec![
        Ok(resp(
            200,
            r#"{"ok":true,"result":[
              {"update_id":10,"message":{"chat":{"id":77},"from":{"username":"ann"},"text":"hi"}},
              {"update_id":11,"message":{"chat":{"id":77},"from":{"username":"ann"},"text":"again"}}]}"#,
        )),
        Ok(resp(200, r#"{"ok":true,"result":[]}"#)),
    ]);
    let ch = telegram(&mock);
    let got = ch.poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].cursor, 10);
    assert_eq!(got[1].cursor, 11);
    assert_eq!(got[0].chat, "77");
    assert_eq!(got[0].user, "ann");
    assert_eq!(got[0].text, "hi");
    assert_eq!(got[0].channel, "tg");
    assert!(ch.poll(&ctx(), Some(11)).unwrap().is_empty());
    let seen = mock.seen();
    assert!(
        !seen[0].url.contains("offset="),
        "first poll: from the beginning"
    );
    assert!(
        seen[1].url.contains("offset=12"),
        "offset = cursor + 1: {}",
        seen[1].url
    );
    assert!(
        seen[1]
            .url
            .contains("/botTOKEN".replace("TOKEN", TOKEN).as_str())
    );
}

#[test]
fn telegram_send_posts_chat_and_text() {
    let mock = Mock::new(vec![Ok(resp(200, r#"{"ok":true}"#))]);
    telegram(&mock).send(&ctx(), "77", "hello").unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].method, HttpMethod::Post);
    assert!(seen[0].url.ends_with("/sendMessage"));
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(body["chat_id"], "77");
    assert_eq!(body["text"], "hello");
}

#[test]
fn telegram_401_is_auth_rejected_and_degraded_and_never_leaks_the_token() {
    let mock = Mock::new(vec![Ok(resp(401, r#"{"ok":false}"#))]);
    let rec = Arc::new(Recorder::new());
    let ch = Arc::new(telegram(&mock));
    assert!(ch.health_issue().is_none(), "healthy before any failure");
    let observed = ObservedChannel::new(ch, rec.clone());
    let err = observed.poll(&ctx(), None).unwrap_err();
    assert!(is_auth_rejected(&err), "{err:?}");
    assert!(!err.to_string().contains("SECRET-TOKEN"));
    let issue = observed.health_issue().expect("Degraded after 401");
    assert!(issue.contains("401"), "{issue}");
    let events = rec.events();
    assert!(
        events
            .iter()
            .any(|e| e.kind == names::CHANNEL_AUTH_REJECTED)
    );
    assert!(events.iter().any(|e| e.kind == names::CHANNEL_POLL_FAILED));
    let dump = format!("{events:?}");
    assert!(!dump.contains("SECRET-TOKEN"), "token leaked into an event");
}

#[test]
fn telegram_transport_error_text_is_scrubbed_of_the_token() {
    let mock = Mock::new(vec![Err(PiiError::Unavailable(format!(
        "dial http://tg.test/bot{TOKEN}/getUpdates refused"
    )))]);
    let err = telegram(&mock).poll(&ctx(), None).unwrap_err();
    assert!(!err.to_string().contains("SECRET-TOKEN"), "{err}");
}

#[test]
fn telegram_missing_token_fails_loudly_and_is_degraded() {
    let mock = Mock::new(vec![]);
    let ch = TelegramChannel::new("tg", mock.clone() as Arc<dyn HttpTransport>, None, None);
    assert!(ch.health_issue().unwrap().contains("token"));
    assert!(matches!(ch.poll(&ctx(), None), Err(PiiError::Invalid(_))));
    assert!(mock.seen().is_empty(), "no request without a token");
}

#[test]
fn matrix_sync_filters_by_cursor_skips_self_and_keeps_since() {
    let sync1 = r#"{"next_batch":"s1","rooms":{"join":{"!r:hs":{"timeline":{"events":[
      {"type":"m.room.message","sender":"@ann:hs","origin_server_ts":1000,"content":{"msgtype":"m.text","body":"hi"}},
      {"type":"m.room.message","sender":"@bot:hs","origin_server_ts":1001,"content":{"msgtype":"m.text","body":"my own reply"}},
      {"type":"m.room.encrypted","sender":"@ann:hs","origin_server_ts":1002,"content":{}}]}}}}}"#;
    let mock = Mock::new(vec![
        Ok(resp(200, sync1)),
        Ok(resp(200, r#"{"next_batch":"s2","rooms":{}}"#)),
    ]);
    let ch = MatrixChannel::new(
        "mx",
        mock.clone() as Arc<dyn HttpTransport>,
        Some("mxtoken".to_owned()),
        Some("http://hs.test"),
        Some("@bot:hs"),
    );
    let got = ch.poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "own + encrypted events are dropped: {got:?}");
    assert_eq!(got[0].chat, "!r:hs");
    assert_eq!(got[0].user, "@ann:hs");
    assert_eq!(got[0].cursor, 1000);
    assert!(ch.poll(&ctx(), Some(1000)).unwrap().is_empty());
    let seen = mock.seen();
    assert!(!seen[0].url.contains("since="));
    assert!(seen[1].url.contains("since=s1"), "{}", seen[1].url);
    assert!(
        seen[0]
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer mxtoken")
    );
}

#[test]
fn matrix_send_puts_an_encoded_room_path() {
    let mock = Mock::new(vec![Ok(resp(200, r#"{"event_id":"$e"}"#))]);
    let ch = MatrixChannel::new(
        "mx",
        mock.clone() as Arc<dyn HttpTransport>,
        Some("t".to_owned()),
        Some("http://hs.test"),
        Some("@bot:hs"),
    );
    ch.send(&ctx(), "!r:hs", "yo").unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].method, HttpMethod::Put);
    assert!(
        seen[0]
            .url
            .contains("/rooms/%21r%3Ahs/send/m.room.message/"),
        "{}",
        seen[0].url
    );
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(body["body"], "yo");
}

#[test]
fn matrix_401_and_missing_config_are_degraded() {
    let mock = Mock::new(vec![Ok(resp(401, "{}"))]);
    let ch = MatrixChannel::new(
        "mx",
        mock.clone() as Arc<dyn HttpTransport>,
        Some("t".to_owned()),
        Some("http://hs.test"),
        Some("@bot:hs"),
    );
    assert!(is_auth_rejected(&ch.poll(&ctx(), None).unwrap_err()));
    assert!(ch.health_issue().is_some());
    let bare = MatrixChannel::new("mx", mock as Arc<dyn HttpTransport>, None, None, None);
    assert!(bare.health_issue().is_some());
    assert!(bare.poll(&ctx(), None).is_err());
}

fn webhook(
    mock: &Arc<Mock>,
    secret: Option<&str>,
    loopback: bool,
    rec: &Arc<Recorder>,
) -> WebhookChannel {
    WebhookChannel::new(
        "hook",
        mock.clone() as Arc<dyn HttpTransport>,
        WebhookConfig {
            out_url: Some("http://sink.test/reply".to_owned()),
            secret: secret.map(str::to_owned),
            loopback,
        },
        rec.clone(),
    )
}

fn post(path: &str, secret: Option<&str>, body: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: path.to_owned(),
        headers: secret
            .map(|s| vec![("X-Pii-Secret".to_owned(), s.to_owned())])
            .unwrap_or_default(),
        body: body.as_bytes().to_vec(),
        timeout_ms: 0,
    }
}

#[test]
fn webhook_inbound_becomes_incoming_and_reply_is_posted() {
    let mock = Mock::new(vec![]);
    let rec = Arc::new(Recorder::new());
    let ch = webhook(&mock, None, true, &rec);
    let r = ch.handle_inbound(&post(
        "/channel/hook",
        None,
        r#"{"text":"ping","chat":"c1","user":"u1"}"#,
    ));
    assert_eq!(r.status, 202);
    let got = ch.poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].text.as_str(), got[0].chat.as_str()), ("ping", "c1"));
    // at-least-once: not acknowledged yet -> delivered again; acked -> gone
    assert_eq!(ch.poll(&ctx(), None).unwrap().len(), 1);
    assert!(ch.poll(&ctx(), Some(got[0].cursor)).unwrap().is_empty());
    ch.send(&ctx(), "c1", "pong").unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].url, "http://sink.test/reply");
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(body["to"], "c1");
    assert_eq!(body["text"], "pong");
}

#[test]
fn webhook_wide_bind_requires_the_secret_with_401_and_an_event() {
    let mock = Mock::new(vec![]);
    let rec = Arc::new(Recorder::new());
    let ch = webhook(&mock, Some("s3cret"), false, &rec);
    let body = r#"{"text":"x"}"#;
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", None, body)).status,
        401
    );
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", Some("wrong"), body))
            .status,
        401
    );
    let denied = rec
        .events()
        .iter()
        .filter(|e| e.kind == names::CHANNEL_AUTH_REJECTED)
        .count();
    assert_eq!(denied, 2);
    assert!(
        ch.poll(&ctx(), None).unwrap().is_empty(),
        "nothing was accepted"
    );
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", Some("s3cret"), body))
            .status,
        202
    );
    assert_eq!(ch.poll(&ctx(), None).unwrap().len(), 1);
    assert!(!format!("{:?}", rec.events()).contains("s3cret"));
}

#[test]
fn webhook_wide_bind_without_a_configured_secret_refuses_everything() {
    let mock = Mock::new(vec![]);
    let rec = Arc::new(Recorder::new());
    let ch = webhook(&mock, None, false, &rec);
    assert!(ch.health_issue().unwrap().contains("secret"));
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", None, r#"{"text":"x"}"#))
            .status,
        401
    );
}

#[test]
fn webhook_rejects_bad_routes_methods_and_bodies() {
    let mock = Mock::new(vec![]);
    let rec = Arc::new(Recorder::new());
    let ch = webhook(&mock, None, true, &rec);
    assert_eq!(
        ch.handle_inbound(&post("/channel/other", None, r#"{"text":"x"}"#))
            .status,
        404
    );
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", None, "not json"))
            .status,
        400
    );
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", None, r#"{"nope":1}"#))
            .status,
        400
    );
    let mut get = post("/channel/hook", None, "");
    get.method = HttpMethod::Get;
    assert_eq!(ch.handle_inbound(&get).status, 405);
    let huge = format!(r#"{{"text":"{}"}}"#, "a".repeat(70_000));
    assert_eq!(
        ch.handle_inbound(&post("/channel/hook", None, &huge))
            .status,
        413
    );
}

#[test]
fn webhook_outbound_outside_the_allowlist_is_denied_and_named() {
    let mock = Mock::new(vec![Err(PiiError::Denied(
        "net: host sink.test not granted".into(),
    ))]);
    let rec = Arc::new(Recorder::new());
    let inner = Arc::new(webhook(&mock, None, true, &rec));
    let ch = ObservedChannel::new(inner, rec.clone());
    assert!(matches!(
        ch.send(&ctx(), "c", "t"),
        Err(PiiError::Denied(_))
    ));
    let events = rec.events();
    assert!(
        events
            .iter()
            .any(|e| e.kind == names::CHANNEL_EGRESS_DENIED)
    );
    assert!(events.iter().any(|e| e.kind == names::CHANNEL_SEND_FAILED));
}
