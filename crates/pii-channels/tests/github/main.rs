//! Spec 406: the github channel, offline (a routing fake transport, no network).
//!
//! The trust gate is the point of these tests: an issue reaches the queue only when the repository
//! owner (or a login in `PII_GITHUB_ALLOW_USERS`) applied the `ready` label, read from the issue's
//! own event log. Stranger labels, removed labels and stale owner labels are all dropped.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the spec names its CONTROL rows in this form.
    non_snake_case
)]

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_channels::Channel as _;
use pii_channels::github::{GithubChannel, GithubSettings};
use pii_channels::is_auth_rejected;
use pii_core::{Budget, CapSet, Ctx, HttpTransport};
use serde_json::{Value, json};

const REPO: &str = "pii-owner/pii";
const OWNER: &str = "pii-owner";
const STRANGER: &str = "mallory";
const TOKEN: &str = "ghp_SECRET-TOKEN-123";
/// 2026-11-10T11:00:00Z in epoch seconds: between the two `updated_at` values used below.
const CURSOR_1100Z: u64 = 1_794_308_400;

mod event_log;

type Handler = Box<dyn Fn(&HttpRequest) -> HttpResponse + Send + Sync>;

/// A fake transport that answers each request through a closure and records every request.
struct Fake {
    handler: Handler,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Fake {
    fn new(handler: impl Fn(&HttpRequest) -> HttpResponse + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            handler: Box::new(handler),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<HttpRequest> {
        self.seen.lock().clone()
    }
}

impl HttpTransport for Fake {
    fn request(&self, _ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
        self.seen.lock().push(req.clone());
        Ok((self.handler)(req))
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

/// The value of `key` in the URL's query string, if present.
fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

/// The issue number in `.../issues/N/events`.
fn event_number(url: &str) -> u64 {
    let path = url.split_once('?').map_or(url, |(p, _)| p);
    let tail = path.rsplit_once("/issues/").expect("issues path").1;
    tail.split('/').next().unwrap().parse().unwrap()
}

fn issue(n: u64, author: &str, labels: &[&str], updated_at: &str) -> Value {
    json!({
        "number": n,
        "title": format!("Issue {n} title"),
        "body": format!("Issue {n} body"),
        "user": { "login": author },
        "labels": labels.iter().map(|l| json!({ "name": l })).collect::<Vec<_>>(),
        "updated_at": updated_at,
        "state": "open",
    })
}

fn labeled(actor: &str, label: &str) -> Value {
    json!({ "event": "labeled", "label": { "name": label }, "actor": { "login": actor } })
}

fn unlabeled(actor: &str, label: &str) -> Value {
    json!({ "event": "unlabeled", "label": { "name": label }, "actor": { "login": actor } })
}

/// A fake GitHub: `pages` is the open-issue listing, one array per page (page 1 carries a `Link`
/// to the next page; past the last page the listing is empty). `events` maps an issue number to
/// its event log, oldest first. A comment POST is accepted.
fn github(pages: Vec<Vec<Value>>, events: Vec<(u64, Vec<Value>)>) -> Arc<Fake> {
    Fake::new(move |req| {
        if req.url.contains("/events") {
            let n = event_number(&req.url);
            let log = events
                .iter()
                .find(|(num, _)| *num == n)
                .map(|(_, log)| log.clone())
                .unwrap_or_default();
            return resp(200, &Value::Array(log).to_string());
        }
        if req.method == HttpMethod::Post {
            return resp(201, "{}");
        }
        let page: usize = query_param(&req.url, "page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(1);
        let body = pages
            .get(page - 1)
            .cloned()
            .map_or_else(|| json!([]), Value::Array)
            .to_string();
        let mut r = resp(200, &body);
        if page < pages.len() {
            r.headers.push((
                String::from("Link"),
                format!("<https://api.github.com/repos/{REPO}/issues?labels=ready&state=open&page={}>; rel=\"next\"", page + 1),
            ));
        }
        r
    })
}

fn settings() -> GithubSettings {
    GithubSettings {
        repo: REPO.to_owned(),
        base_url: None,
        token_env: None,
        token_file: None,
    }
}

/// An environment with the token set, plus `extra` overrides.
fn env_with(extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let mut vars: Vec<(String, String)> =
        vec![(String::from("PII_GITHUB_TOKEN"), TOKEN.to_owned())];
    vars.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    move |key: &str| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn channel(fake: &Arc<Fake>, extra: &[(&str, &str)]) -> GithubChannel {
    GithubChannel::build(
        "gh",
        Arc::clone(fake) as Arc<dyn HttpTransport>,
        &settings(),
        env_with(extra),
    )
    .unwrap()
}

#[test]
fn a_ready_issue_labelled_by_the_owner_is_returned() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(OWNER, "ready")])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].chat, "7");
    assert_eq!(got[0].user, "ann");
    assert_eq!(got[0].text, "Issue 7 title\n\nIssue 7 body");
    assert_eq!(got[0].channel, "gh");
}

#[test]
fn CONTROL_an_issue_labelled_by_a_stranger_is_dropped_and_counted() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(STRANGER, "ready")])],
    );
    let ch = channel(&fake, &[]);
    let got = ch.poll(&ctx(), None).unwrap();
    assert!(
        got.is_empty(),
        "a stranger's label must not reach the queue: {got:?}"
    );
    assert_eq!(ch.counters().unwrap().dropped, 1);
    let health = ch.health_issue().unwrap_or_default();
    assert!(
        !health.contains("Issue 7"),
        "issue text must not be surfaced: {health}"
    );
}

#[test]
fn an_issue_whose_ready_label_was_removed_is_not_returned() {
    let fake = github(
        vec![vec![issue(7, "ann", &[], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(OWNER, "ready"), unlabeled(OWNER, "ready")])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_stale_owner_label_does_not_cover_a_later_stranger_label() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(
            7,
            vec![
                labeled(OWNER, "ready"),
                unlabeled(OWNER, "ready"),
                labeled(STRANGER, "ready"),
            ],
        )],
    );
    let ch = channel(&fake, &[]);
    let got = ch.poll(&ctx(), None).unwrap();
    assert!(got.is_empty(), "{got:?}");
    assert_eq!(ch.counters().unwrap().dropped, 1);
}

// The labels say `ready` while the event log says otherwise. Only the event-log check can tell these
// apart from a ready issue, because the current-labels check sees `ready` in both.
#[test]
fn CONTROL_an_issue_written_by_the_owner_without_the_label_is_not_returned() {
    let fake = github(
        vec![vec![issue(7, OWNER, &[], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(OWNER, "ready")])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    assert!(got.is_empty(), "authorship is not approval: {got:?}");
    let events_asked = fake.seen().iter().any(|r| r.url.contains("/events"));
    assert!(
        !events_asked,
        "no events request for an issue without the label"
    );
}

#[test]
fn the_allow_list_env_replaces_the_default() {
    let fake = github(
        vec![vec![
            issue(1, "ann", &["ready"], "2026-11-10T12:00:00Z"),
            issue(2, "ann", &["ready"], "2026-11-10T12:00:00Z"),
        ]],
        vec![
            (1, vec![labeled("triager", "ready")]),
            (2, vec![labeled(OWNER, "ready")]),
        ],
    );
    let ch = channel(&fake, &[("PII_GITHUB_ALLOW_USERS", "triager")]);
    let got = ch.poll(&ctx(), None).unwrap();
    let chats: Vec<&str> = got.iter().map(|i| i.chat.as_str()).collect();
    assert_eq!(chats, ["1"], "only the allow-listed actor passes");
    assert_eq!(
        ch.counters().unwrap().dropped,
        1,
        "the owner is no longer allowed"
    );
}

#[test]
fn poll_reads_every_page() {
    let fake = github(
        vec![
            vec![issue(1, "ann", &["ready"], "2026-11-10T12:00:00Z")],
            vec![issue(2, "bob", &["ready"], "2026-11-10T12:00:00Z")],
        ],
        vec![
            (1, vec![labeled(OWNER, "ready")]),
            (2, vec![labeled(OWNER, "ready")]),
        ],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    let chats: Vec<&str> = got.iter().map(|i| i.chat.as_str()).collect();
    assert_eq!(chats, ["1", "2"], "issues from both pages");
}

#[test]
fn poll_returns_only_issues_newer_than_the_cursor() {
    let fake = github(
        vec![vec![
            issue(1, "ann", &["ready"], "2026-11-10T10:00:00Z"),
            issue(2, "ann", &["ready"], "2026-11-10T12:00:00Z"),
        ]],
        vec![
            (1, vec![labeled(OWNER, "ready")]),
            (2, vec![labeled(OWNER, "ready")]),
        ],
    );
    let got = channel(&fake, &[])
        .poll(&ctx(), Some(CURSOR_1100Z))
        .unwrap();
    let chats: Vec<&str> = got.iter().map(|i| i.chat.as_str()).collect();
    assert_eq!(chats, ["2"], "only the issue updated after the cursor");
}

#[test]
fn a_bad_page_is_unavailable_not_fatal() {
    let fake = Fake::new(|req| {
        if req.url.contains("/events") {
            return resp(200, "[]");
        }
        if query_param(&req.url, "page").as_deref() == Some("2") {
            return resp(500, "{}");
        }
        let mut r = resp(
            200,
            &json!([issue(1, "ann", &["ready"], "2026-11-10T12:00:00Z")]).to_string(),
        );
        r.headers.push((
            String::from("Link"),
            format!("<https://api.github.com/repos/{REPO}/issues?page=2>; rel=\"next\""),
        ));
        r
    });
    let err = channel(&fake, &[]).poll(&ctx(), None).unwrap_err();
    assert!(matches!(err, PiiError::Unavailable(_)), "{err:?}");
}

#[test]
fn send_posts_a_comment_on_that_issue() {
    let fake = github(vec![], vec![]);
    channel(&fake, &[]).send(&ctx(), "7", "done").unwrap();
    let seen = fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].method, HttpMethod::Post);
    assert!(
        seen[0]
            .url
            .ends_with(&format!("/repos/{REPO}/issues/7/comments")),
        "{}",
        seen[0].url
    );
    let body: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(body["body"], "done");
}

#[test]
fn CONTROL_send_to_a_non_number_is_refused_and_sends_nothing() {
    let fake = github(vec![], vec![]);
    let ch = channel(&fake, &[]);
    for to in ["7/../8", ""] {
        let err = ch.send(&ctx(), to, "x").unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "{to:?}: {err:?}");
    }
    assert!(
        fake.seen().is_empty(),
        "nothing may be sent: {:?}",
        fake.seen()
    );
}

#[test]
fn a_401_degrades_health_and_hides_the_token() {
    let fake = Fake::new(|_| resp(401, r#"{"message":"Bad credentials"}"#));
    let ch = channel(&fake, &[]);
    assert!(ch.health_issue().is_none(), "healthy before any failure");
    let err = ch.poll(&ctx(), None).unwrap_err();
    assert!(is_auth_rejected(&err), "{err:?}");
    assert!(!err.to_string().contains(TOKEN));
    let issue = ch.health_issue().expect("degraded after a 401");
    assert!(
        issue.contains("api rejected the token (status 401)"),
        "{issue}"
    );
    assert!(!issue.contains(TOKEN));
}

#[test]
fn a_rate_limit_is_not_a_token_rejection() {
    let fake = Fake::new(|_| {
        let mut r = resp(403, r#"{"message":"API rate limit exceeded"}"#);
        r.headers
            .push((String::from("x-ratelimit-remaining"), String::from("0")));
        r
    });
    let ch = channel(&fake, &[]);
    let err = ch.poll(&ctx(), None).unwrap_err();
    assert!(matches!(err, PiiError::Unavailable(_)), "{err:?}");
    assert!(
        ch.health_issue().is_none(),
        "a rate limit is not a credential fault"
    );
}

#[test]
fn egress_is_the_api_host_only() {
    let fake = github(vec![], vec![]);
    assert_eq!(channel(&fake, &[]).egress_hosts(), ["api.github.com:443"]);
}

#[test]
fn a_missing_token_is_degraded_not_a_refusal_to_start() {
    let fake = github(vec![], vec![]);
    let ch = GithubChannel::build(
        "gh",
        Arc::clone(&fake) as Arc<dyn HttpTransport>,
        &settings(),
        |_: &str| None,
    )
    .expect("the channel starts without a token");
    assert_eq!(ch.health_issue().as_deref(), Some("token missing"));
}

#[test]
fn the_channel_never_writes_a_label() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(OWNER, "ready")])],
    );
    let ch = channel(&fake, &[]);
    ch.poll(&ctx(), None).unwrap();
    ch.send(&ctx(), "7", "done").unwrap();
    for req in fake.seen() {
        assert!(!req.url.contains("/labels"), "label write: {}", req.url);
        match req.method {
            HttpMethod::Get => {}
            HttpMethod::Post => assert!(
                req.url.ends_with("/comments"),
                "only the comment POST may write: {}",
                req.url
            ),
            other => panic!("write method {other:?} to {}", req.url),
        }
    }
}
