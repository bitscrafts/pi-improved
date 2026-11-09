//! Spec 401: the `pii` Discord channel reads a forum's active threads and answers inside them,
//! offline (a scripted transport answers by method and path; no network).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_channels::Channel as _;
use pii_channels::discord::DiscordChannel;
use pii_core::{Budget, CapSet, Ctx, HttpTransport};
use serde_json::{Value, json};

const BASE: &str = "http://fake.test";
/// The configured channel: a forum (type 15) in most tests, a text channel (type 0) in one.
const CHANNEL: &str = "F1";
const GUILD: &str = "G1";
/// The guild owner: on the allow list by default (spec 401 R4).
const OWNER: &str = "7";
/// Our own bot (the token's user): its id, and the username a human may share with it.
const BOT: &str = "900";
const BOT_NAME: &str = "pii";

/// A Discord fake. It answers by method and path and records every request. A route it does not
/// know is a 404, so a request the channel should not make shows up as a failure.
struct Fake {
    /// The configured channel's type: 15 forum, 0 text.
    kind: u64,
    /// The active threads the guild listing returns (`GET /guilds/{g}/threads/active`).
    threads: Vec<Value>,
    /// `(channel id, message)` pairs; the fake returns them newest first, as Discord does.
    messages: Vec<(&'static str, Value)>,
    /// Status of the thread listing.
    threads_status: u16,
    /// Status of `GET /users/@me`.
    me_status: u16,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Fake {
    fn new(kind: u64) -> Self {
        Self {
            kind,
            threads: Vec::new(),
            messages: Vec::new(),
            threads_status: 200,
            me_status: 200,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn route(&self, req: &HttpRequest) -> (u16, String) {
        let (path, query) = req.url.split_once('?').unwrap_or((req.url.as_str(), ""));
        let path = path.strip_prefix(BASE).unwrap_or(path);
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (req.method == HttpMethod::Get, segs.as_slice()) {
            (true, ["guilds", "G1", "threads", "active"]) => {
                if self.threads_status != 200 {
                    let body = r#"{"message": "Missing Access", "code": 50001}"#;
                    return (self.threads_status, body.to_owned());
                }
                let listing = json!({ "threads": self.threads, "members": [], "has_more": false });
                (200, listing.to_string())
            }
            (true, ["users", "@me"]) => {
                if self.me_status != 200 {
                    return (self.me_status, String::from("{}"));
                }
                (
                    200,
                    json!({ "id": BOT, "username": BOT_NAME, "bot": true }).to_string(),
                )
            }
            (true, ["guilds", "G1"]) => {
                (200, json!({ "id": GUILD, "owner_id": OWNER }).to_string())
            }
            (true, ["channels", id]) => {
                // The configured channel has the configured type; every other id is a thread.
                let kind = if *id == CHANNEL { self.kind } else { 11 };
                (
                    200,
                    json!({ "id": id, "type": kind, "guild_id": GUILD }).to_string(),
                )
            }
            (true, ["channels", id, "messages"]) => {
                let after = after_of(query);
                let mut msgs: Vec<&Value> = self
                    .messages
                    .iter()
                    .filter(|(c, m)| c == id && after.is_none_or(|a| id_of(m) > a))
                    .map(|(_, m)| m)
                    .collect();
                msgs.sort_by_key(|m| std::cmp::Reverse(id_of(m)));
                msgs.truncate(20);
                (
                    200,
                    Value::Array(msgs.into_iter().cloned().collect()).to_string(),
                )
            }
            (false, ["channels", _, "messages"]) => (200, String::from("{}")),
            _ => (404, String::from("{}")),
        }
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().clone()
    }

    fn posts(&self) -> Vec<HttpRequest> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == HttpMethod::Post)
            .collect()
    }
}

impl HttpTransport for Fake {
    fn request(&self, _ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
        self.seen.lock().push(req.clone());
        let (status, body) = self.route(req);
        Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: body.into_bytes(),
        })
    }
}

/// The `after` query value of a messages URL.
fn after_of(query: &str) -> Option<u64> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("after=")?.parse().ok())
}

fn id_of(m: &Value) -> u64 {
    m["id"].as_str().unwrap().parse().unwrap()
}

/// A message as the live API shapes it: ids as strings, the author with its id and username.
fn message(id: u64, text: &str, author: &str, name: &str, bot: bool) -> Value {
    json!({
        "id": id.to_string(),
        "content": text,
        "author": { "id": author, "username": name, "bot": bot },
    })
}

/// An active thread under the forum, as the guild listing shapes it.
fn thread(id: &str, last_message_id: u64) -> Value {
    json!({
        "id": id,
        "parent_id": CHANNEL,
        "last_message_id": last_message_id.to_string(),
        "type": 11,
    })
}

/// The channel as the daemon builds it. The env is a closure, so no process-global env is set.
fn channel(fake: &Arc<Fake>, allow: Option<&str>) -> DiscordChannel {
    let allow = allow.map(str::to_owned);
    DiscordChannel::from_lookup(
        "discord1",
        Arc::clone(fake) as Arc<dyn HttpTransport>,
        move |k| match k {
            "PII_DISCORD_TOKEN" => Some(String::from("tok")),
            "PII_DISCORD_CHANNEL_ID" => Some(String::from(CHANNEL)),
            "PII_DISCORD_API_BASE" => Some(String::from(BASE)),
            "PII_DISCORD_ALLOW_USERS" => allow.clone(),
            _ => None,
        },
    )
    .unwrap()
}

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

/// The texts of the returned messages, in order.
fn texts(got: &[pii_channels::Incoming]) -> Vec<&str> {
    got.iter().map(|m| m.text.as_str()).collect()
}

#[test]
fn a_forum_poll_reads_the_active_threads_oldest_first() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 30), thread("T2", 20)];
    fake.messages = vec![
        ("T1", message(30, "newest", "7", "owner", false)),
        ("T2", message(20, "middle", "7", "owner", false)),
        ("T1", message(10, "oldest", "7", "owner", false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    let pairs: Vec<(u64, &str)> = got.iter().map(|m| (m.cursor, m.chat.as_str())).collect();
    assert_eq!(pairs, vec![(10, "T1"), (20, "T2"), (30, "T1")]);
}

#[test]
fn an_idle_forum_poll_makes_one_request_after_the_probe() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 30), thread("T2", 20)];
    fake.messages = vec![("T1", message(30, "seen", "7", "owner", false))];
    let fake = Arc::new(fake);
    let ch = channel(&fake, None);
    // The first poll learns the shape, the bot's identity and the owner; all are cached after it.
    ch.poll(&ctx(), None).unwrap();
    fake.seen.lock().clear();
    let got = ch.poll(&ctx(), Some(30)).unwrap();
    assert!(got.is_empty(), "nothing newer than the cursor");
    let urls: Vec<String> = fake.requests().into_iter().map(|r| r.url).collect();
    assert!(
        urls.iter().all(|u| !u.contains("/messages")),
        "no message request: {urls:?}"
    );
    assert_eq!(urls.len(), 1, "one request: the thread listing: {urls:?}");
}

#[test]
fn a_message_from_our_own_bot_is_never_returned() {
    // The bot and the human have DIFFERENT ids and the SAME username (spec 401 R3).
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 20)];
    fake.messages = vec![
        ("T1", message(20, "our reply", BOT, BOT_NAME, true)),
        ("T1", message(10, "a human", "7", BOT_NAME, false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    assert_eq!(texts(&got), vec!["a human"]);
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_a_listed_human_in_the_same_thread_is_returned() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 30)];
    fake.messages = vec![
        ("T1", message(20, "our reply", BOT, BOT_NAME, true)),
        ("T1", message(30, "a human, newer", "7", BOT_NAME, false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].text, "a human, newer");
    assert_eq!(got[0].chat, "T1", "chat is the thread id");
}

#[test]
fn a_message_from_another_bot_is_not_returned() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 20)];
    fake.messages = vec![
        ("T1", message(20, "another bot", "901", "helper", true)),
        ("T1", message(10, "a human", "7", "owner", false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    assert_eq!(texts(&got), vec!["a human"]);
}

#[test]
fn an_author_not_on_the_allow_list_is_dropped_and_counted() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 10)];
    fake.messages = vec![("T1", message(10, "stranger text", "8", "stranger", false))];
    let fake = Arc::new(fake);
    let ch = channel(&fake, None);
    let got = ch.poll(&ctx(), None).unwrap();
    assert!(got.is_empty(), "{got:?}");
    assert_eq!(ch.counters().map(|c| c.dropped), Some(1));
}

#[test]
fn the_default_allow_list_is_the_guild_owner() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 20)];
    fake.messages = vec![
        ("T1", message(20, "owner text", "7", "owner", false)),
        ("T1", message(10, "not the owner", "8", "other", false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    assert_eq!(texts(&got), vec!["owner text"]);
}

#[test]
fn the_env_allow_list_replaces_the_default() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 20)];
    fake.messages = vec![
        ("T1", message(20, "owner text", "7", "owner", false)),
        ("T1", message(10, "listed text", "8", "other", false)),
    ];
    let fake = Arc::new(fake);
    let got = channel(&fake, Some("8")).poll(&ctx(), None).unwrap();
    assert_eq!(texts(&got), vec!["listed text"]);
}

#[test]
fn an_unresolvable_identity_refuses_the_poll() {
    let mut fake = Fake::new(15);
    fake.threads = vec![thread("T1", 10)];
    fake.messages = vec![("T1", message(10, "text", "7", "owner", false))];
    fake.me_status = 500;
    let fake = Arc::new(fake);
    let ch = channel(&fake, None);
    assert!(ch.poll(&ctx(), None).is_err(), "refused, never unfiltered");
    let urls: Vec<String> = fake.requests().into_iter().map(|r| r.url).collect();
    assert!(
        urls.iter().all(|u| !u.contains("/messages")),
        "no message read: {urls:?}"
    );
}

#[test]
fn a_reply_to_a_thread_id_posts_a_message_in_that_thread() {
    let fake = Arc::new(Fake::new(15));
    channel(&fake, None)
        .send(&ctx(), "T1", "hello thread")
        .unwrap();
    let posts = fake.posts();
    assert_eq!(posts.len(), 1, "one POST: {posts:?}");
    assert_eq!(posts[0].url, "http://fake.test/channels/T1/messages");
    assert!(
        posts.iter().all(|r| !r.url.ends_with("/threads")),
        "no thread created"
    );
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(body["content"], "hello thread");
}

#[test]
fn a_failed_thread_listing_is_an_error_not_an_empty_poll() {
    let mut fake = Fake::new(15);
    fake.threads_status = 403;
    let fake = Arc::new(fake);
    let err = channel(&fake, None).poll(&ctx(), None).unwrap_err();
    assert!(
        err.to_string().contains("threads/active"),
        "names the endpoint: {err}"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_a_text_channel_polls_as_before() {
    let mut fake = Fake::new(0);
    fake.messages = vec![("F1", message(10, "hello", "8", "anyone", false))];
    let fake = Arc::new(fake);
    let got = channel(&fake, None).poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    let urls: Vec<String> = fake.requests().into_iter().map(|r| r.url).collect();
    let messages: Vec<&String> = urls.iter().filter(|u| u.contains("/messages")).collect();
    assert_eq!(
        messages,
        vec!["http://fake.test/channels/F1/messages?limit=20"],
        "{urls:?}"
    );
    assert!(urls.iter().all(|u| !u.contains("/threads")), "{urls:?}");
}
