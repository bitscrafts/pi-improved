//! Config -> channel for the spec 112 kinds (`telegram`, `matrix`, `webhook`).
//! Credentials come from env or a 0600 file ([`pii_channels::secret`]), never
//! from a config value.

use std::net::SocketAddr;
use std::sync::Arc;

use pii_abi::{HttpRequest, HttpResponse, PiiError};
use pii_channels::webhook::{WebhookChannel, WebhookConfig};
use pii_channels::{Channel, matrix, secret, telegram};
use pii_core::HttpTransport;
use pii_net::listen::{HttpHandler, ListenSpec, Protocol};

use super::config::{ChannelSection, MqttSection};

/// Default webhook bind: loopback only.
const DEFAULT_BIND: &str = "127.0.0.1:8787";

fn token(c: &ChannelSection, default_env: &str) -> Result<Option<String>, PiiError> {
    secret::resolve(
        Some(c.token_env.as_deref().unwrap_or(default_env)),
        c.token_file.as_deref(),
    )
}

pub(super) fn telegram(
    c: &ChannelSection,
    transport: Arc<dyn HttpTransport>,
) -> Result<Arc<dyn Channel>, PiiError> {
    Ok(Arc::new(telegram::TelegramChannel::new(
        &c.name,
        transport,
        token(c, telegram::DEFAULT_TOKEN_ENV)?,
        c.base_url.as_deref(),
    )))
}

pub(super) fn matrix(
    c: &ChannelSection,
    transport: Arc<dyn HttpTransport>,
) -> Result<Arc<dyn Channel>, PiiError> {
    Ok(Arc::new(matrix::MatrixChannel::new(
        &c.name,
        transport,
        token(c, matrix::DEFAULT_TOKEN_ENV)?,
        c.base_url.as_deref(),
        c.user_id.as_deref(),
    )))
}

/// Serves the channel's request handler on the daemon's `NetService`.
struct Handler(Arc<WebhookChannel>);

impl HttpHandler for Handler {
    fn handle(&self, req: &HttpRequest) -> HttpResponse {
        self.0.handle_inbound(req)
    }
}

/// Builds the webhook channel and binds its listener. The listener lives as
/// long as the daemon; it appears in `sys.status` `listeners`.
pub(super) fn webhook(
    c: &ChannelSection,
    net: &Arc<pii_net::NetService>,
    transport: Arc<dyn HttpTransport>,
    log: &Arc<dyn pii_observe::Observer>,
) -> Result<Arc<dyn Channel>, PiiError> {
    let bind: SocketAddr = c
        .bind
        .as_deref()
        .unwrap_or(DEFAULT_BIND)
        .parse()
        .map_err(|e| PiiError::Invalid(format!("channel '{}': bad bind address: {e}", c.name)))?;
    let ch = Arc::new(WebhookChannel::new(
        &c.name,
        transport,
        WebhookConfig {
            out_url: c.url.clone(),
            secret: secret::resolve(c.secret_env.as_deref(), c.secret_file.as_deref())?,
            loopback: bind.ip().is_loopback(),
        },
        Arc::clone(log),
    ));
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::NET;
    let ctx = pii_core::Ctx::root(caps, pii_core::Budget::unlimited());
    net.listen(
        &ctx,
        ListenSpec::Tcp { addr: bind },
        Protocol::Http,
        Arc::new(Handler(Arc::clone(&ch))),
    )?;
    Ok(ch)
}

/// Builds and starts an MQTT connection (spec 117). Credentials come from env
/// or a 0600 file ([`secret::resolve`]); a refused config (plaintext
/// credentials to a remote broker, `mqtts://`) is an `Err` that names no secret.
pub(super) fn mqtt(
    m: &MqttSection,
    log: &Arc<dyn pii_observe::Observer>,
) -> Result<Arc<pii_channels::mqtt::MqttChannel>, PiiError> {
    let mut cfg = pii_channels::mqtt::MqttConfig::new(&m.name, &m.url);
    cfg.client_id.clone_from(&m.client_id);
    cfg.topics.clone_from(&m.topics);
    cfg.qos = m.qos;
    cfg.username = secret::resolve(m.username_env.as_deref(), m.username_file.as_deref())?;
    cfg.password = secret::resolve(m.password_env.as_deref(), m.password_file.as_deref())?;
    cfg.allow_plaintext_credentials = m.allow_plaintext_credentials;
    cfg.reconnect_min_ms = m.reconnect_min_ms;
    cfg.reconnect_max_ms = m.reconnect_max_ms;
    cfg.queue_cap = m.queue_cap;
    pii_channels::mqtt::MqttChannel::start(cfg, Arc::clone(log))
}
