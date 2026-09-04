//! `DiscordChannel` (F6.3): plaintext HTTP against a Discord-API-compatible endpoint.
//!
//! TLS is cut to Phase 3b, so `PII_DISCORD_API_BASE` must point at plaintext
//! `http://` in this phase (the fake, or a future 3b `https://`).

use std::sync::Arc;

use pii_abi::{HttpMethod, HttpRequest, PiiError};
use pii_core::{Ctx, HttpTransport};
use serde::Deserialize;

use crate::{Channel, Incoming};

/// Default Discord API base (blocked on 3b's TLS; tests point this at a
/// loopback fake).
pub const DEFAULT_API_BASE: &str = "https://api.discord.com";

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
}

impl DiscordChannel {
    /// Builds a channel from the phase's env surface (N10):
    /// `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`, `PII_DISCORD_API_BASE`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when a required env var is missing.
    pub fn from_env(name: &str, transport: Arc<dyn HttpTransport>) -> Result<Self, PiiError> {
        let token = pii_core::env_var("PII_DISCORD_TOKEN")
            .ok_or_else(|| PiiError::Invalid(String::from("PII_DISCORD_TOKEN is not set")))?;
        let channel_id = pii_core::env_var("PII_DISCORD_CHANNEL_ID")
            .ok_or_else(|| PiiError::Invalid(String::from("PII_DISCORD_CHANNEL_ID is not set")))?;
        let base = pii_core::env_var("PII_DISCORD_API_BASE")
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            name: name.to_owned(),
            transport,
            base: base.trim_end_matches('/').to_owned(),
            token,
            channel_id,
            timeout_ms: 10_000,
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
            return Err(PiiError::Unavailable(format!(
                "discord: poll status {}",
                resp.status
            )));
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
        let body = serde_json::to_vec(&serde_json::json!({ "content": text }))
            .map_err(|e| PiiError::Internal(format!("discord: encode: {e}")))?;
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: format!("{}/channels/{to}/messages", self.base),
            headers: vec![
                self.auth_header(),
                (
                    String::from("Content-Type"),
                    String::from("application/json"),
                ),
            ],
            body,
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &req)?;
        if !resp.is_success() {
            return Err(PiiError::Unavailable(format!(
                "discord: send status {}",
                resp.status
            )));
        }
        Ok(())
    }
}
