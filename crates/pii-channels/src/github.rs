//! `GithubChannel` (spec 406): a repository's issues that the owner has marked `ready`, over the
//! injected transport.
//!
//! Trust is the point of this adapter. An issue reaches the queue only when a login on the allow
//! list (the repository owner, or `PII_GITHUB_ALLOW_USERS`) applied the `ready` label, and that is
//! read from the issue's own event log, never from its author or its text. Text from a stranger is
//! data: this module hands it on and never executes it. The only write is a comment (`send`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::{Channel, ChannelCounters, Incoming, auth_rejected, scrub, secret};

/// Default REST base.
pub const DEFAULT_API_BASE: &str = "https://api.github.com";
/// Default env var holding the token.
pub const DEFAULT_TOKEN_ENV: &str = "PII_GITHUB_TOKEN";
/// Env var naming the logins whose `ready` label counts (replaces the owner, as Discord's list does).
const ALLOW_ENV: &str = "PII_GITHUB_ALLOW_USERS";
/// The label that marks an issue as approved for the queue.
const READY: &str = "ready";
/// Longest comment `send` posts, in bytes. Longer text is cut with [`CUT_MARK`].
const COMMENT_CAP: usize = 60_000;
const CUT_MARK: &str = "\n\n[cut: the comment was longer than 60000 bytes]";
/// Pages read before a listing is refused as runaway.
const MAX_PAGES: u32 = 50;

/// What `[[channel]]` names for a `github` channel (see `pii-server`'s config).
#[derive(Debug, Clone, Default)]
pub struct GithubSettings {
    /// `owner/name` of the repository.
    pub repo: String,
    /// REST base; `None` = [`DEFAULT_API_BASE`].
    pub base_url: Option<String>,
    /// Env var holding the token; `None` = [`DEFAULT_TOKEN_ENV`].
    pub token_env: Option<String>,
    /// A 0600 file holding the token, used when the env var is unset.
    pub token_file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Login {
    login: String,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    user: Option<Login>,
    #[serde(default)]
    labels: Vec<Named>,
    updated_at: String,
    /// Set for a pull request, which the issue listing also returns.
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(rename = "event")]
    action: String,
    label: Option<Named>,
    actor: Option<Login>,
    /// ISO 8601 UTC, as GitHub sends it; absent falls back to the issue's `updated_at`.
    created_at: Option<String>,
}

/// A GitHub channel for one repository.
pub struct GithubChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    base: String,
    repo: String,
    /// Never logged, never in an error (see [`scrub`]).
    token: Option<String>,
    /// Logins whose `ready` label counts (case-insensitive, as GitHub logins are).
    allow: Vec<String>,
    /// Set by a 401/403, cleared by the next success.
    rejected: Mutex<Option<String>>,
    received: AtomicU64,
    dropped: AtomicU64,
    published: AtomicU64,
}

impl GithubChannel {
    /// Builds the channel from its settings and an environment lookup. A missing token is allowed
    /// on purpose: the channel reports `Degraded` and every poll fails, instead of the daemon
    /// refusing to start (spec 112 R5).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when `repo` is not `owner/name`, or when `token_file` is loose or unreadable.
    pub fn build(
        name: &str,
        transport: Arc<dyn HttpTransport>,
        settings: &GithubSettings,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, PiiError> {
        let owner = match settings.repo.split_once('/') {
            Some((owner, repo)) if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') => {
                owner.to_owned()
            }
            _ => {
                return Err(PiiError::Invalid(format!(
                    "github '{name}': repo must be \"owner/name\""
                )));
            }
        };
        let token_env = settings.token_env.as_deref().unwrap_or(DEFAULT_TOKEN_ENV);
        let token = match env(token_env).filter(|v| !v.is_empty()) {
            Some(v) => Some(v),
            None => secret::resolve(None, settings.token_file.as_deref())?,
        };
        let allow = env(ALLOW_ENV).filter(|v| !v.trim().is_empty()).map_or_else(
            || vec![owner],
            |list| {
                list.split(',')
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            },
        );
        Ok(Self {
            name: name.to_owned(),
            transport,
            base: settings
                .base_url
                .as_deref()
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_owned(),
            repo: settings.repo.clone(),
            token,
            allow,
            rejected: Mutex::new(None),
            received: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            published: AtomicU64::new(0),
        })
    }

    fn token(&self) -> Result<&str, PiiError> {
        self.token.as_deref().ok_or_else(|| {
            PiiError::Invalid(format!(
                "github '{}': no token (set token_env or token_file)",
                self.name
            ))
        })
    }

    fn request(&self, token: &str, method: HttpMethod, path: &str, body: Vec<u8>) -> HttpRequest {
        let mut headers = vec![
            (String::from("Authorization"), format!("Bearer {token}")),
            (
                String::from("Accept"),
                String::from("application/vnd.github+json"),
            ),
            (String::from("User-Agent"), String::from("pii")),
        ];
        if !body.is_empty() {
            headers.push((
                String::from("Content-Type"),
                String::from("application/json"),
            ));
        }
        HttpRequest {
            method,
            url: format!("{}/repos/{}/{path}", self.base, self.repo),
            headers,
            body,
            timeout_ms: 10_000,
        }
    }

    fn call(&self, ctx: &Ctx, req: &HttpRequest, token: &str) -> Result<HttpResponse, PiiError> {
        let resp = self
            .transport
            .request(ctx, req)
            .map_err(|e| redact(e, token))?;
        if rate_limited(&resp) {
            return Err(PiiError::Unavailable(format!(
                "github: rate limited (status {})",
                resp.status
            )));
        }
        if matches!(resp.status, 401 | 403) {
            *self.rejected.lock() =
                Some(format!("api rejected the token (status {})", resp.status));
            return Err(auth_rejected("github", resp.status));
        }
        if !resp.is_success() {
            return Err(PiiError::Unavailable(format!(
                "github: status {}",
                resp.status
            )));
        }
        *self.rejected.lock() = None;
        Ok(resp)
    }

    /// Every page of a GET listing (`path` carries a query string), following `Link: rel="next"`.
    fn paged<T: DeserializeOwned>(&self, ctx: &Ctx, path: &str) -> Result<Vec<T>, PiiError> {
        let token = self.token()?;
        let mut out = Vec::new();
        let mut page = 1;
        loop {
            let req = self.request(
                token,
                HttpMethod::Get,
                &format!("{path}&page={page}"),
                Vec::new(),
            );
            let resp = self.call(ctx, &req, token)?;
            let items: Vec<T> = serde_json::from_slice(&resp.body)
                .map_err(|e| PiiError::Unavailable(format!("github: bad listing body: {e}")))?;
            out.extend(items);
            let has_next = resp
                .header("link")
                .is_some_and(|l| l.split(',').any(|part| part.contains("rel=\"next\"")));
            if !has_next {
                return Ok(out);
            }
            page += 1;
            if page > MAX_PAGES {
                return Err(PiiError::Unavailable(String::from(
                    "github: too many pages",
                )));
            }
        }
    }

    /// The time (epoch seconds) of the latest `ready` event on issue `number` when it is a labelling
    /// by an allowed login; `None` otherwise. The event log decides; the issue's author and text do not.
    /// An event without `created_at` takes the issue's `updated` time.
    fn approval(&self, ctx: &Ctx, number: u64, updated: u64) -> Result<Option<u64>, PiiError> {
        let events: Vec<Event> =
            self.paged(ctx, &format!("issues/{number}/events?per_page=100"))?;
        let latest = events.iter().rev().find(|e| {
            e.label.as_ref().is_some_and(|l| l.name == READY)
                && matches!(e.action.as_str(), "labeled" | "unlabeled")
        });
        Ok(latest
            .filter(|e| {
                e.action == "labeled"
                    && e.actor.as_ref().is_some_and(|a| {
                        self.allow.iter().any(|x| x.eq_ignore_ascii_case(&a.login))
                    })
            })
            .map(|e| {
                e.created_at
                    .as_deref()
                    .and_then(epoch_secs)
                    .unwrap_or(updated)
            }))
    }
}

/// Keeps the error variant, drops the token.
fn redact(e: PiiError, token: &str) -> PiiError {
    match e {
        PiiError::Denied(m) => PiiError::Denied(scrub(&m, token)),
        PiiError::Invalid(m) => PiiError::Invalid(scrub(&m, token)),
        other => PiiError::Unavailable(scrub(&other.to_string(), token)),
    }
}

/// A 429, or a 403 that GitHub sends for an exhausted quota: a pause, not a token rejection.
fn rate_limited(resp: &HttpResponse) -> bool {
    resp.status == 429
        || (resp.status == 403
            && (resp.header("retry-after").is_some()
                || resp.header("x-ratelimit-reset").is_some()
                || resp.header("x-ratelimit-remaining") == Some("0")))
}

/// `2026-11-10T12:00:00Z` as epoch seconds; `None` for any other shape.
fn epoch_secs(ts: &str) -> Option<u64> {
    let body = ts.strip_suffix('Z')?;
    let parts = body
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let [y, mo, d, h, mi, s] = parts.as_slice() else {
        return None;
    };
    let y = if *mo <= 2 { y - 1 } else { *y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + s).ok()
}

/// `text` cut to [`COMMENT_CAP`] bytes (on a character boundary) with a marker, when longer.
fn cap_comment(text: &str) -> String {
    if text.len() <= COMMENT_CAP {
        return text.to_owned();
    }
    let mut end = COMMENT_CAP - CUT_MARK.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT_MARK}", text.get(..end).unwrap_or_default())
}

impl Channel for GithubChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "github"
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

    fn counters(&self) -> Option<ChannelCounters> {
        Some(ChannelCounters {
            received: self.received.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            published: self.published.load(Ordering::Relaxed),
        })
    }

    /// `cursor` is the time an owner last applied `ready`, in epoch seconds. An issue updated at or
    /// before it is skipped without an events call; otherwise its label time decides. Each `Incoming`
    /// carries that label time as its cursor; the result is oldest label first.
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let issues: Vec<Issue> = self.paged(ctx, "issues?labels=ready&state=open&per_page=100")?;
        let mut out = Vec::new();
        for issue in issues {
            if issue.pull_request.is_some() {
                continue;
            }
            let updated = epoch_secs(&issue.updated_at).ok_or_else(|| {
                PiiError::Unavailable(String::from("github: bad updated_at in listing"))
            })?;
            if cursor.is_some_and(|c| updated <= c) {
                continue;
            }
            if !issue.labels.iter().any(|l| l.name == READY) {
                continue;
            }
            let Some(labelled) = self.approval(ctx, issue.number, updated)? else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if cursor.is_some_and(|c| labelled <= c) {
                continue;
            }
            self.received.fetch_add(1, Ordering::Relaxed);
            out.push(Incoming {
                channel: self.name.clone(),
                chat: issue.number.to_string(),
                user: issue.user.map(|u| u.login).unwrap_or_default(),
                text: format!("{}\n\n{}", issue.title, issue.body.unwrap_or_default()),
                cursor: labelled,
            });
        }
        out.sort_by_key(|i| i.cursor);
        Ok(out)
    }

    /// Comments on issue `to`, which must be a decimal issue number.
    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        if to.is_empty() || !to.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PiiError::Invalid(format!(
                "github '{}': recipient must be an issue number",
                self.name
            )));
        }
        let token = self.token()?;
        let body = serde_json::to_vec(&serde_json::json!({ "body": cap_comment(text) }))
            .map_err(|e| PiiError::Internal(format!("github: encode: {e}")))?;
        let req = self.request(
            token,
            HttpMethod::Post,
            &format!("issues/{to}/comments"),
            body,
        );
        self.call(ctx, &req, token)?;
        self.published.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
