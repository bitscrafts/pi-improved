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

use super::config::{ChannelSection, HalSection, MqttSection, WasmSection};

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

/// Fills the HAL, plugin-host and entity-store slots from `[hal]`, `[wasm]` and `[memory_graph]`.
pub(super) fn install_devices(
    engine: &crate::Engine,
    cfg: &super::config::Config,
    runtime_dir: &std::path::Path,
) -> Result<(), PiiError> {
    install_hal(engine, cfg)?;
    install_wasm(engine, cfg)?;
    install_entities(engine, cfg, runtime_dir)
}

/// Installs the entity store (spec 126) under the daemon's runtime dir, reporting to the
/// engine's log. Always installed: `[memory_graph]` only tunes its bounds.
pub(super) fn install_entities(
    engine: &crate::Engine,
    cfg: &super::config::Config,
    runtime_dir: &std::path::Path,
) -> Result<(), PiiError> {
    let store = pii_memory::entity::EntityStore::open(runtime_dir, cfg.memory_graph.graph_config())
        .map_err(|e| PiiError::Internal(e.to_string()))?
        .with_observer(engine.log());
    engine.entities.set(Arc::new(store));
    Ok(())
}

/// Builds the `[hal]` backend, when configured, into the engine's HAL slot.
/// No `[hal]` section leaves the slot empty: the HAL stays off.
pub(super) fn install_hal(
    engine: &crate::Engine,
    cfg: &super::config::Config,
) -> Result<(), PiiError> {
    if let Some(h) = &cfg.hal {
        engine.hal.set(Arc::new(hal(h, &engine.log())?));
    }
    Ok(())
}

/// Config -> [`pii_hal::Hal`] (spec 121). `sim` is the in-memory board;
/// `gpiod` needs the `hal-gpiod` build feature and a real chip.
pub(super) fn hal(
    h: &HalSection,
    log: &Arc<dyn pii_observe::Observer>,
) -> Result<pii_hal::Hal, PiiError> {
    let scope = pii_hal::HalScope::from_config(&h.pins, &h.i2c, &h.spi)?;
    let backend: Arc<dyn pii_hal::HalBackend> = match h.backend.as_str() {
        "sim" => Arc::new(pii_hal::SimBackend::standard()),
        #[cfg(feature = "hal-gpiod")]
        "gpiod" => Arc::new(pii_hal::GpiodBackend::open(
            h.chip.as_deref().unwrap_or("/dev/gpiochip0"),
        )?),
        #[cfg(not(feature = "hal-gpiod"))]
        "gpiod" => {
            return Err(PiiError::Invalid(String::from(
                "hal: backend 'gpiod' needs a build with the hal-gpiod feature",
            )));
        }
        other => {
            return Err(PiiError::Invalid(format!(
                "hal: unknown backend '{other}' (known: sim, gpiod)"
            )));
        }
    };
    Ok(pii_hal::Hal::new(backend, scope, Some(Arc::clone(log))))
}

/// Builds the `[wasm]` host, when configured, into the engine's plugin slot.
/// No `[wasm]` section leaves the slot empty: the plugin host stays off.
pub(super) fn install_wasm(
    engine: &crate::Engine,
    cfg: &super::config::Config,
) -> Result<(), PiiError> {
    if let Some(w) = &cfg.wasm {
        engine.wasm.set(Arc::new(wasm(w, &engine.log())?));
    }
    Ok(())
}

/// Config -> [`pii_wasm::WasmHost`] (spec 122).
pub(super) fn wasm(
    w: &WasmSection,
    log: &Arc<dyn pii_observe::Observer>,
) -> Result<pii_wasm::WasmHost, PiiError> {
    let mut cfg = pii_wasm::WasmConfig::new(&w.dir);
    cfg.timeout = std::time::Duration::from_millis(w.timeout_ms);
    cfg.max_memory = w.max_memory_mib.saturating_mul(1024 * 1024);
    cfg.fuel = w.fuel;
    pii_wasm::WasmHost::new(cfg, Some(Arc::clone(log)))
}
