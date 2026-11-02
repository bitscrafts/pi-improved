//! `MqttChannel` (spec 117): a device-facing front door.
//!
//! MQTT 3.1.1 over **plain TCP only** — `rumqttc` is built without its TLS features so `rustls`
//! stays confined to `pii-net`; `mqtts://` is refused, and a password is never
//! sent in clear text to a non-loopback host without an explicit opt-in.
//!
//! **Inbound** — the client subscribes to the configured topic filters on
//! every (re)connect. A message waits in a bounded queue that
//! [`Channel::poll`] drains (chat = topic, text = payload); a full queue or a
//! non-UTF-8 payload is *dropped and counted* (`mqtt.message.dropped`).
//! **Outbound** — [`MqttChannel::publish`] with `QoS` and retain stated by the
//! caller; [`Channel::send`] is a deliberate no-op (replying onto the topic that
//! triggered a run would be a feedback loop).
//!
//! **Reconnect** — a failed or lost connection reports `Degraded` with the
//! reason, emits `mqtt.connection.lost`, and retries with exponential backoff
//! from `reconnect_min_ms` doubling to the `reconnect_max_ms` ceiling (default
//! 500 ms → 30 s). The driver thread never spins.
//!
//! The client dials its broker directly (not through `pii-net` grants): the
//! broker address is operator config, not something a graph can choose.

mod config;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_core::{Clock as _, Ctx};
use pii_observe::{Event, Level, Observer, names};
use rumqttc::{AsyncClient, ConnectionError, Event as Wire, MqttOptions, Packet, QoS};

pub use config::{
    DEFAULT_QUEUE_CAP, DEFAULT_RECONNECT_MAX_MS, DEFAULT_RECONNECT_MIN_MS, MAX_PACKET, MqttConfig,
    backoff_ms, is_loopback,
};

use crate::{Channel, ChannelCounters, Incoming, scrub};

/// Capacity of the client's outgoing request queue (publish/subscribe).
const REQUEST_CAP: usize = 64;
/// How often a sleeping driver notices it was asked to stop.
const STOP_SLICE: Duration = Duration::from_millis(25);

#[derive(Default)]
struct Link {
    connected: bool,
    last_error: Option<String>,
}

struct Shared {
    name: String,
    broker: String,
    observer: Arc<dyn Observer>,
    queue: Mutex<VecDeque<Incoming>>,
    queue_cap: usize,
    link: Mutex<Link>,
    next_cursor: AtomicU64,
    received: AtomicU64,
    dropped: AtomicU64,
    published: AtomicU64,
    attempts: AtomicU64,
    stop: AtomicBool,
}

impl Shared {
    fn event(&self, level: Level, kind: &'static str) -> Event {
        Event::new("mqtt", level, kind)
            .field("channel", self.name.as_str())
            .field("broker", self.broker.as_str())
    }

    fn drop_message(&self, topic: &str, reason: &'static str) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.observer.event(
            &self
                .event(Level::Warn, names::MQTT_MESSAGE_DROPPED)
                .field("topic", topic)
                .field("reason", reason),
        );
    }

    fn accept(&self, topic: &str, payload: &[u8]) {
        let Ok(text) = std::str::from_utf8(payload) else {
            return self.drop_message(topic, "payload is not valid UTF-8");
        };
        let mut q = self.queue.lock();
        if q.len() >= self.queue_cap {
            drop(q);
            return self.drop_message(topic, "inbound queue full");
        }
        q.push_back(Incoming {
            channel: self.name.clone(),
            chat: topic.to_owned(),
            user: String::from("mqtt"),
            text: text.to_owned(),
            cursor: self.next_cursor.fetch_add(1, Ordering::Relaxed),
        });
        drop(q);
        self.received.fetch_add(1, Ordering::Relaxed);
        self.observer.event(
            &self
                .event(Level::Info, names::MQTT_MESSAGE_RECEIVED)
                .field("topic", topic)
                .field("bytes", payload.len()),
        );
    }
}

/// The daemon's MQTT connections by `[[mqtt]]` name: filled by the daemon at
/// startup, read by the `mqtt` graph node (whose factory is built earlier).
#[derive(Default)]
pub struct MqttHub {
    map: Mutex<std::collections::BTreeMap<String, Arc<MqttChannel>>>,
}

impl MqttHub {
    /// Registers `ch` under its channel name.
    pub fn insert(&self, ch: Arc<MqttChannel>) {
        self.map.lock().insert(ch.name().to_owned(), ch);
    }

    /// The connection named `name`, if configured.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<MqttChannel>> {
        self.map.lock().get(name).cloned()
    }
}

/// One broker connection: a driver thread plus the shared queue.
pub struct MqttChannel {
    shared: Arc<Shared>,
    client: AsyncClient,
}

impl MqttChannel {
    /// Validates `cfg` (see [`MqttConfig::validate`]) and starts the driver.
    /// The first connection attempt happens on the driver thread; an
    /// unreachable broker is `Degraded`, not an error here.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when the config is refused (bad url, `mqtts://`,
    /// credentials to a non-loopback plaintext broker without the opt-in) or
    /// the driver thread cannot start. Errors never contain a credential.
    pub fn start(cfg: MqttConfig, observer: Arc<dyn Observer>) -> Result<Arc<Self>, PiiError> {
        let (host, port) = cfg.validate()?;
        let broker = format!("{host}:{port}");
        let shared = Arc::new(Shared {
            name: cfg.name.clone(),
            broker,
            observer,
            queue: Mutex::new(VecDeque::new()),
            queue_cap: cfg.queue_cap,
            link: Mutex::new(Link::default()),
            next_cursor: AtomicU64::new(pii_core::SystemClock.now_ms()),
            received: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            published: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        if cfg.plaintext_credentials_opted_in() {
            shared.observer.event(
                &shared
                    .event(Level::Warn, names::MQTT_PLAINTEXT_CREDENTIALS)
                    .field("note", "credentials sent in clear text by explicit opt-in"),
            );
        }
        let mut opts = MqttOptions::new(client_id(&cfg), host, port);
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_clean_session(true);
        opts.set_max_packet_size(MAX_PACKET, MAX_PACKET);
        if cfg.username.is_some() || cfg.password.is_some() {
            opts.set_credentials(
                cfg.username.clone().unwrap_or_default(),
                cfg.password.clone().unwrap_or_default(),
            );
        }
        let (client, eventloop) = AsyncClient::new(opts, REQUEST_CAP);
        let driver = Driver {
            shared: Arc::clone(&shared),
            client: client.clone(),
            cfg,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PiiError::Invalid(format!("mqtt: runtime: {e}")))?;
        std::thread::Builder::new()
            .name(format!("mqtt-{}", shared.name))
            .spawn(move || rt.block_on(driver.run(eventloop)))
            .map_err(|e| PiiError::Invalid(format!("mqtt: driver thread: {e}")))?;
        Ok(Arc::new(Self { shared, client }))
    }

    /// Publishes `payload` to `topic`. `QoS` (0..=2) and `retain` are the
    /// caller's explicit choice; the message is counted and reported
    /// (`mqtt.published`) once the client accepts it for sending.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for `QoS` above 2; [`PiiError::Unavailable`] when
    /// the outgoing queue is full or the driver stopped (also counted as a
    /// drop).
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> Result<(), PiiError> {
        let q = qos_of(qos)?;
        if payload.len() > MAX_PACKET {
            self.shared
                .drop_message(topic, "payload exceeds the packet ceiling");
            return Err(PiiError::Invalid(format!(
                "mqtt publish: payload over {MAX_PACKET} bytes"
            )));
        }
        if let Err(e) = self.client.try_publish(topic, q, retain, payload.to_vec()) {
            self.shared
                .drop_message(topic, "outgoing queue full or driver stopped");
            return Err(PiiError::Unavailable(format!("mqtt publish: {e}")));
        }
        self.shared.published.fetch_add(1, Ordering::Relaxed);
        self.shared.observer.event(
            &self
                .shared
                .event(Level::Info, names::MQTT_PUBLISHED)
                .field("topic", topic)
                .field("qos", qos)
                .field("retain", retain)
                .field("bytes", payload.len()),
        );
        Ok(())
    }

    /// Failed or lost connections since start (one per `mqtt.connection.lost`).
    #[must_use]
    pub fn reconnect_attempts(&self) -> u64 {
        self.shared.attempts.load(Ordering::Relaxed)
    }
}

impl Drop for MqttChannel {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let _ = self.client.try_disconnect();
    }
}

impl Channel for MqttChannel {
    fn name(&self) -> &str {
        &self.shared.name
    }

    fn kind(&self) -> &'static str {
        "mqtt"
    }

    fn poll(&self, _ctx: &Ctx, _cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        Ok(self.shared.queue.lock().drain(..).collect())
    }

    /// Deliberately a no-op: see the module docs. Publish through the `mqtt`
    /// node (or [`MqttChannel::publish`]) with an explicit topic.
    fn send(&self, _ctx: &Ctx, _to: &str, _text: &str) -> Result<(), PiiError> {
        Ok(())
    }

    fn health_issue(&self) -> Option<String> {
        let link = self.shared.link.lock();
        if link.connected {
            None
        } else {
            link.last_error.clone()
        }
    }

    fn counters(&self) -> Option<ChannelCounters> {
        Some(ChannelCounters {
            received: self.shared.received.load(Ordering::Relaxed),
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            published: self.shared.published.load(Ordering::Relaxed),
        })
    }
}

fn qos_of(qos: u8) -> Result<QoS, PiiError> {
    match qos {
        0 => Ok(QoS::AtMostOnce),
        1 => Ok(QoS::AtLeastOnce),
        2 => Ok(QoS::ExactlyOnce),
        _ => Err(PiiError::Invalid(format!(
            "mqtt: qos {qos} is not 0, 1 or 2"
        ))),
    }
}

/// `cfg.client_id`, else an id unique per process and per instance.
fn client_id(cfg: &MqttConfig) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    cfg.client_id.clone().unwrap_or_else(|| {
        format!(
            "pii-{:x}-{:x}-{:x}",
            std::process::id(),
            pii_core::SystemClock.now_ms() & 0xff_ffff,
            SEQ.fetch_add(1, Ordering::Relaxed)
        )
    })
}

struct Driver {
    shared: Arc<Shared>,
    client: AsyncClient,
    cfg: MqttConfig,
}

impl Driver {
    async fn run(self, mut eventloop: rumqttc::EventLoop) {
        let mut failures: u32 = 0;
        // Subscribing from INSIDE the poll handler cannot work: `try_subscribe` pushes into the request channel that
        // this very loop drains, and rumqttc answers every failed push with the same string. Measured 2026-11-02
        // against the real Zigbee broker: two topics, a 64-slot channel, and both were dropped as "queue full".
        // The flag is set in the handler and acted on OUTSIDE it, with the awaited API, which cannot re-enter.
        let mut subscribe_now = false;
        while !self.shared.stop.load(Ordering::SeqCst) {
            match eventloop.poll().await {
                Ok(Wire::Incoming(Packet::ConnAck(_))) => {
                    failures = 0;
                    self.connected();
                    subscribe_now = true;
                }
                Ok(Wire::Incoming(Packet::Publish(p))) => self.shared.accept(&p.topic, &p.payload),
                Ok(_) => {}
                Err(e) => {
                    if self.shared.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let wait = backoff_ms(
                        failures,
                        self.cfg.reconnect_min_ms,
                        self.cfg.reconnect_max_ms,
                    );
                    failures = failures.saturating_add(1);
                    self.lost(&e, wait);
                    self.sleep(wait).await;
                }
            }
            if subscribe_now {
                subscribe_now = false;
                self.subscribe_all().await;
            }
        }
    }

    fn connected(&self) {
        let sh = &self.shared;
        {
            let mut link = sh.link.lock();
            link.connected = true;
            link.last_error = None;
        }
        sh.observer
            .event(&sh.event(Level::Info, names::MQTT_CONNECTED));
    }

    /// Subscribes to every configured filter, from OUTSIDE the event-loop handler. The awaited API waits for
    /// capacity instead of guessing that there is none, and the reason reported on failure is the error itself.
    async fn subscribe_all(&self) {
        let sh = &self.shared;
        let qos = qos_of(self.cfg.qos).unwrap_or(QoS::AtMostOnce);
        for filter in &self.cfg.topics {
            if let Err(e) = self.client.subscribe(filter, qos).await {
                sh.observer.event(
                    &sh.event(Level::Warn, names::MQTT_MESSAGE_DROPPED)
                        .field("topic", filter.as_str())
                        .field("reason", format!("subscribe failed: {e}")),
                );
            }
        }
    }

    fn lost(&self, err: &ConnectionError, retry_in_ms: u64) {
        let sh = &self.shared;
        let attempt = sh.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        let mut reason = err.to_string();
        for secret in [&self.cfg.password, &self.cfg.username]
            .into_iter()
            .flatten()
        {
            reason = scrub(&reason, secret);
        }
        let was_connected = {
            let mut link = sh.link.lock();
            let was = link.connected;
            link.connected = false;
            link.last_error = Some(format!(
                "broker {} unreachable: {reason} (attempt {attempt}, retry in {retry_in_ms} ms)",
                sh.broker
            ));
            was
        };
        sh.observer.event(
            &sh.event(Level::Warn, names::MQTT_CONNECTION_LOST)
                .field("error", reason)
                .field("attempt", attempt)
                .field("retry_in_ms", retry_in_ms)
                .field("was_connected", was_connected),
        );
    }

    /// Sleeps `ms` in slices so a dropped channel stops the thread promptly.
    async fn sleep(&self, ms: u64) {
        let mut left = Duration::from_millis(ms);
        while !left.is_zero() && !self.shared.stop.load(Ordering::SeqCst) {
            let step = left.min(STOP_SLICE);
            tokio::time::sleep(step).await;
            left -= step;
        }
    }
}
