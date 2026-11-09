//! Reading a forum's active threads (spec 401).
//!
//! A forum holds no messages of its own, so its replies are read from each active thread under it.
//! Three rules decide what comes back. Our own bot never does: a reply read back would be answered
//! again without end. Another bot never does: bots are context, never requests. A human does only
//! when the allow list names them (`PII_DISCORD_ALLOW_USERS`, else the guild owner), because anyone
//! in the guild can post in a thread.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pii_abi::{HttpMethod, HttpRequest, PiiError};
use pii_core::Ctx;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{DiscordChannel, DiscordMessage};
use crate::{ChannelCounters, Incoming};

/// The guild's active threads (`GET /guilds/{g}/threads/active`).
#[derive(Deserialize)]
struct ThreadListing {
    threads: Vec<ActiveThread>,
}

/// One active thread, as the listing shapes it (ids as strings).
#[derive(Deserialize)]
struct ActiveThread {
    id: String,
    parent_id: Option<String>,
    last_message_id: Option<String>,
}

/// Our own user (`GET /users/@me`).
#[derive(Deserialize)]
struct Me {
    id: String,
}

/// The guild (`GET /guilds/{g}`), for its owner.
#[derive(Deserialize)]
struct Guild {
    owner_id: String,
}

/// Who may talk to pii, and what the forum read has learned (spec 401 R3, R4).
#[derive(Default)]
pub(super) struct Inbound {
    /// The `PII_DISCORD_ALLOW_USERS` ids, when set. Unset, the list is the guild owner.
    allow: Option<Vec<String>>,
    /// Learned once and kept: our own bot id, and each guild's owner id.
    learned: Mutex<Learned>,
    received: AtomicU64,
    dropped: AtomicU64,
}

#[derive(Default)]
struct Learned {
    bot: Option<String>,
    owners: BTreeMap<String, String>,
}

impl Inbound {
    /// `allow` is the raw env value: comma-separated ids. Blank entries are ignored, and a list
    /// with none left counts as unset.
    pub(super) fn new(allow: Option<String>) -> Self {
        let ids: Vec<String> = allow
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            allow: (!ids.is_empty()).then_some(ids),
            ..Self::default()
        }
    }

    /// Received and dropped counts of the forum read. `published` stays 0: sends are not counted.
    pub(super) fn counters(&self) -> ChannelCounters {
        ChannelCounters {
            received: self.received.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            published: 0,
        }
    }
}

impl DiscordChannel {
    /// The forum's poll: each active thread under `forum` that holds a message past `cursor`, read
    /// and filtered (spec 401 R1 to R4). The result is oldest first.
    pub(super) fn poll_forum(
        &self,
        ctx: &Ctx,
        cursor: Option<u64>,
        guild: &str,
    ) -> Result<Vec<Incoming>, PiiError> {
        if guild.is_empty() {
            return Err(PiiError::Unavailable(format!(
                "discord: forum {} names no guild",
                self.channel_id
            )));
        }
        let me = self.bot_id(ctx)?;
        let allowed = self.allowed(ctx, guild)?;
        let listing: ThreadListing =
            self.get_json(ctx, &format!("guilds/{guild}/threads/active"))?;
        let mut out = Vec::new();
        for t in listing.threads {
            if t.parent_id.as_deref() != Some(self.channel_id.as_str()) {
                continue;
            }
            let Some(last) = t.last_message_id else {
                continue;
            };
            let last: u64 = last.parse().map_err(|e| {
                PiiError::Unavailable(format!("discord: non-numeric thread id '{last}': {e}"))
            })?;
            if cursor.is_some_and(|c| last <= c) {
                continue;
            }
            self.read_thread(ctx, &t.id, cursor, &me, &allowed, &mut out)?;
        }
        out.sort_by_key(|m| m.cursor);
        self.inbound
            .received
            .fetch_add(out.len() as u64, Ordering::Relaxed);
        Ok(out)
    }

    /// The messages of one thread past `cursor`, filtered by the loop guard and the allow list.
    /// A dropped message is counted and its text is never logged.
    fn read_thread(
        &self,
        ctx: &Ctx,
        thread: &str,
        cursor: Option<u64>,
        me: &str,
        allowed: &[String],
        out: &mut Vec<Incoming>,
    ) -> Result<(), PiiError> {
        let path = cursor.map_or_else(
            || format!("channels/{thread}/messages?limit=20"),
            |c| format!("channels/{thread}/messages?after={c}&limit=20"),
        );
        let msgs: Vec<DiscordMessage> = self.get_json(ctx, &path)?;
        for m in msgs {
            // The loop guard compares ids: a username can be shared with the bot (spec 401 R3).
            if m.author.id == me || m.author.bot {
                continue;
            }
            if !allowed.contains(&m.author.id) {
                self.inbound.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let cursor = m.id.parse::<u64>().map_err(|e| {
                PiiError::Unavailable(format!("discord: non-numeric message id '{}': {e}", m.id))
            })?;
            out.push(Incoming {
                channel: self.name.clone(),
                chat: thread.to_owned(),
                user: m.author.username,
                text: m.content,
                cursor,
            });
        }
        Ok(())
    }

    /// Our own bot id, learned once. Unknown, the poll is refused: the loop guard cannot run without it.
    fn bot_id(&self, ctx: &Ctx) -> Result<String, PiiError> {
        let known = self.inbound.learned.lock().bot.clone();
        if let Some(id) = known {
            return Ok(id);
        }
        let me: Me = self.get_json(ctx, "users/@me")?;
        self.inbound.learned.lock().bot = Some(me.id.clone());
        Ok(me.id)
    }

    /// The ids whose messages come back: the env list when set, else the guild owner (spec 401 R4).
    fn allowed(&self, ctx: &Ctx, guild: &str) -> Result<Vec<String>, PiiError> {
        if let Some(ids) = &self.inbound.allow {
            return Ok(ids.clone());
        }
        let known = self.inbound.learned.lock().owners.get(guild).cloned();
        if let Some(owner) = known {
            return Ok(vec![owner]);
        }
        let g: Guild = self.get_json(ctx, &format!("guilds/{guild}"))?;
        self.inbound
            .learned
            .lock()
            .owners
            .insert(guild.to_owned(), g.owner_id.clone());
        Ok(vec![g.owner_id])
    }

    /// A GET under the API base, answered as JSON. A non-2xx answer is `Unavailable` naming the
    /// endpoint, never an empty result (spec 401 R6).
    fn get_json<T: DeserializeOwned>(&self, ctx: &Ctx, path: &str) -> Result<T, PiiError> {
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: format!("{}/{path}", self.base),
            headers: vec![self.auth_header()],
            body: Vec::new(),
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(self.refusal(path, &resp));
        }
        serde_json::from_slice(&resp.body)
            .map_err(|e| PiiError::Unavailable(format!("discord: bad {path} body: {e}")))
    }
}

impl DiscordChannel {
    /// A text channel's poll: its messages, newest first from Discord, oldest first out (spec 180).
    pub(super) fn poll_messages(
        &self,
        ctx: &Ctx,
        cursor: Option<u64>,
    ) -> Result<Vec<Incoming>, PiiError> {
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
}
