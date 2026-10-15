//! Spec 157: the Discord channel's credentials are a state to report, and the daemon's first post
//! names the revision it runs.

use pii_abi::PiiError;

use super::{Daemon, config, identity};
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
