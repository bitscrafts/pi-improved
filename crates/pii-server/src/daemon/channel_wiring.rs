//! Spec 157: the Discord channel's credentials are a state to report, and the daemon's first post
//! names the revision it runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::PiiError;

use super::{Daemon, adapters, config, identity};
use crate::Engine;

/// How long the startup verification post may take before it is reported as timed out.
const VERIFICATION_POST_TIMEOUT_MS: u64 = 15_000;

/// The credential variables a `discord` channel lacks, by name (never by value); `None` when it
/// has both or is not a `discord` channel. A fresh host has neither, and that is a state to
/// report, not a reason to refuse to start (spec 157).
fn missing_discord_credentials(
    c: &config::ChannelSection,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Vec<&'static str>> {
    if c.kind != "discord" {
        return None;
    }
    let missing: Vec<&'static str> = ["PII_DISCORD_TOKEN", "PII_DISCORD_CHANNEL_ID"]
        .into_iter()
        .filter(|k| env(k).is_none_or(|v| v.is_empty()))
        .collect();
    (!missing.is_empty()).then_some(missing)
}

fn report_channel_absent(engine: &Engine, c: &config::ChannelSection, missing: &[&str]) {
    engine.log().event(
        &pii_observe::Event::new(
            "channel",
            pii_observe::Level::Warn,
            pii_observe::names::CHANNEL_MISCONFIGURED,
        )
        .field("channel", c.name.as_str())
        .field("kind", c.kind.as_str())
        .field(
            "issue",
            format!(
                "not started: {} not set in the environment",
                missing.join(", ")
            ),
        ),
    );
}

/// True (after reporting it by name) when `c` is a `discord` channel without its credentials:
/// the daemon runs without it instead of refusing to start.
pub(super) fn skip_unconfigured(
    engine: &Engine,
    c: &config::ChannelSection,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    missing_discord_credentials(c, env).is_some_and(|missing| {
        report_channel_absent(engine, c, &missing);
        true
    })
}

impl Daemon {
    /// Spec 157: posts one message naming the deployed revision to the `discord` channel, at the
    /// destination `PII_DISCORD_CHANNEL_ID`. `None` when no such channel is running (the
    /// credentials were absent): nothing is attempted. A refused or failed post is the `Err`,
    /// counted by the hub and logged; it never takes the daemon down. Blocks up to the post
    /// timeout, so a caller on the daemon's async runtime runs it on a blocking thread.
    pub fn post_verification(
        &self,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Result<pii_channels::post::PostOutcome, PiiError>> {
        let name = {
            let channels = self.ctx.channels.lock();
            channels
                .iter()
                .find(|(_, rt)| rt.channel.kind() == "discord")
                .map(|(n, _)| n.clone())?
        };
        let to = env("PII_DISCORD_CHANNEL_ID").filter(|t| !t.is_empty())?;
        // A FORUM turns every message into a thread (measured 2026-11-02: the destination is the `pii` forum,
        // type 15), so a startup notice is not a line in a channel — it is a new thread. Restarting a few times
        // would bury the digests under "pii is up" posts. Silent by default; `PII_VERIFY_POST=1` posts it.
        if !env("PII_VERIFY_POST").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")) {
            let ready = pii_observe::Event::new(
                "channel",
                pii_observe::Level::Info,
                pii_observe::names::CHANNEL_READY,
            )
            .field("revision", identity::REVISION)
            .field("note", "startup post disabled; PII_VERIFY_POST=1 posts it")
            .field("channel", name);
            self.engine.log().event(&ready);
            return None;
        }
        let text = format!(
            "pii {} is up (revision {})",
            env!("CARGO_PKG_VERSION"),
            identity::REVISION
        );
        let key = format!("verify-{}", identity::REVISION);
        let result = self
            .engine
            .channel_hub
            .post(&pii_channels::post::PostRequest {
                channel: &name,
                to: &to,
                text: &text,
                key: Some(&key),
                tag: None,
                timeout_ms: VERIFICATION_POST_TIMEOUT_MS,
            });
        let event = match &result {
            Ok(_) => pii_observe::Event::new(
                "channel",
                pii_observe::Level::Info,
                pii_observe::names::CHANNEL_VERIFICATION_POSTED,
            )
            .field("text", text),
            Err(e) => pii_observe::Event::new(
                "channel",
                pii_observe::Level::Warn,
                pii_observe::names::CHANNEL_POST_REFUSED,
            )
            .field("reason", e.to_string()),
        };
        self.engine.log().event(&event.field("channel", name));
        Some(result)
    }
}

/// Spec 157: the first post names the deployed revision. Off the startup path, so an unreachable
/// channel delays nothing; the daemon counts and logs its outcome.
impl Daemon {
    /// Runs [`Self::post_verification`] on its own thread.
    pub fn spawn_verification_post(self: &Arc<Self>) {
        let daemon = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name(String::from("verification-post"))
            .spawn(move || {
                let _ = daemon.post_verification(&pii_core::env_var);
            });
    }
}

pub(super) fn build_raw_channel(
    c: &config::ChannelSection,
    net: &Arc<pii_net::NetService>,
    runtime_dir: &Path,
    log: &Arc<dyn pii_observe::Observer>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Arc<dyn pii_channels::Channel>, PiiError> {
    let transport = Arc::clone(net) as Arc<dyn pii_core::HttpTransport>;
    match c.kind.as_str() {
        "telegram" => adapters::telegram(c, transport),
        "matrix" => adapters::matrix(c, transport),
        "webhook" => adapters::webhook(c, net, transport, log),
        "github" => adapters::github(c, transport, env),
        "discord" => Ok(Arc::new(
            pii_channels::discord::DiscordChannel::from_lookup(&c.name, transport, env)?,
        )),
        "cli" => {
            let inbox = c.inbox.clone().map_or_else(
                || runtime_dir.join(format!("{}.in.jsonl", c.name)),
                PathBuf::from,
            );
            let outbox = c.outbox.clone().map_or_else(
                || runtime_dir.join(format!("{}.out.jsonl", c.name)),
                PathBuf::from,
            );
            Ok(Arc::new(pii_channels::cli::CliChannel::new(
                c.name.clone(),
                inbox,
                outbox,
            )))
        }
        other => Err(PiiError::Invalid(format!(
            "channel '{}': unknown kind '{other}'",
            c.name
        ))),
    }
}
