//! Spec 137: a channel's env-derived egress host needs the configuration's sanction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pii_channels::discord::DiscordChannel;

use super::check_sanctioned;
use super::config::{Config, ConfigGrants};

fn net() -> Arc<pii_net::NetService> {
    Arc::new(pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(pii_net::grant::StaticGrants::default()),
        clock: Arc::new(pii_core::SystemClock),
    }))
}

fn discord(base: Option<&str>) -> DiscordChannel {
    DiscordChannel::from_lookup("d", net(), |k| match k {
        "PII_DISCORD_TOKEN" => Some(String::from("t")),
        "PII_DISCORD_CHANNEL_ID" => Some(String::from("1")),
        "PII_DISCORD_API_BASE" => base.map(str::to_owned),
        _ => None,
    })
    .unwrap()
}

fn check(cfg_text: &str, ch: &DiscordChannel) -> Result<(), pii_abi::PiiError> {
    let cfg = Config::parse(cfg_text).unwrap();
    let grants = ConfigGrants::from_section(&cfg.net).unwrap();
    check_sanctioned(cfg.channel.first().unwrap(), ch, &grants)
}

const BASE: &str = "[[channel]]\nname = \"d\"\nkind = \"discord\"\ngraph = \"g\"\n";

#[test]
fn an_env_override_outside_the_config_is_refused() {
    let err = check(BASE, &discord(Some("https://evil.example")))
        .unwrap_err()
        .to_string();
    assert!(err.contains("'d'"), "{err}");
    assert!(err.contains("evil.example:443"), "{err}");
    assert!(err.contains("hosts") && err.contains("[net]"), "{err}");
}

#[test]
fn the_shipped_default_is_granted() {
    check(BASE, &discord(None)).unwrap();
    assert_eq!(
        pii_channels::Channel::egress_hosts(&discord(None)),
        vec![String::from("discord.com:443")]
    );
}

#[test]
fn a_config_named_host_is_granted() {
    let ch = discord(Some("https://evil.example"));
    check(&format!("{BASE}hosts = [\"evil.example:443\"]\n"), &ch).unwrap();
    check(
        &format!("[net]\ngrants = [\"evil.example:443\"]\n\n{BASE}"),
        &ch,
    )
    .unwrap();
}
