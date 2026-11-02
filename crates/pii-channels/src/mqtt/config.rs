//! MQTT settings, URL parsing and the two safety rules that live before any
//! socket opens: no TLS in this build, and no password in clear text to a
//! host that is not this machine.

use pii_abi::PiiError;

/// Ceiling on one inbound/outbound packet. A larger packet makes the client
/// close the connection (reported as `mqtt.connection.lost`).
pub const MAX_PACKET: usize = 256 * 1024;
/// Default inbound queue bound (messages waiting for the router).
pub const DEFAULT_QUEUE_CAP: usize = 1024;
/// Default first reconnect delay.
pub const DEFAULT_RECONNECT_MIN_MS: u64 = 500;
/// Default reconnect ceiling: the longest the channel ever waits.
pub const DEFAULT_RECONNECT_MAX_MS: u64 = 30_000;

/// Static settings of one broker connection.
#[derive(Clone)]
pub struct MqttConfig {
    /// Channel name (`[[mqtt]] name`).
    pub name: String,
    /// `mqtt://host[:port]` (`mqtts://` is refused: this build has no TLS).
    pub url: String,
    /// MQTT client id; `None` = unique per instance (`pii-<pid>-<time>-<seq>`).
    pub client_id: Option<String>,
    /// Topic filters subscribed on every (re)connect.
    pub topics: Vec<String>,
    /// Devices the model may command, by base (`tasmota_6DE85C`) or name; empty = none writable (spec 336).
    pub allow_devices: Vec<String>,
    /// `QoS` (0..=2) used for the subscriptions.
    pub qos: u8,
    /// Broker user name (resolved from env/file by the caller, never config).
    pub username: Option<String>,
    /// Broker password (resolved from env/file by the caller, never config).
    pub password: Option<String>,
    /// Opt in to sending credentials in clear text to a non-loopback broker.
    pub allow_plaintext_credentials: bool,
    /// First reconnect delay; doubles per consecutive failure.
    pub reconnect_min_ms: u64,
    /// Reconnect delay ceiling.
    pub reconnect_max_ms: u64,
    /// Bound on the inbound queue; beyond it messages are dropped and counted.
    pub queue_cap: usize,
}

impl MqttConfig {
    /// Settings for `url` with every optional key at its default.
    #[must_use]
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_owned(),
            url: url.to_owned(),
            client_id: None,
            topics: Vec::new(),
            allow_devices: Vec::new(),
            qos: 0,
            username: None,
            password: None,
            allow_plaintext_credentials: false,
            reconnect_min_ms: DEFAULT_RECONNECT_MIN_MS,
            reconnect_max_ms: DEFAULT_RECONNECT_MAX_MS,
            queue_cap: DEFAULT_QUEUE_CAP,
        }
    }

    /// Splits `url` into `(host, port)`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for `mqtts://` (no TLS here), another scheme, or
    /// an unusable host/port.
    pub fn endpoint(&self) -> Result<(String, u16), PiiError> {
        let Some(rest) = self.url.strip_prefix("mqtt://") else {
            return Err(PiiError::Invalid(if self.url.starts_with("mqtts://") {
                format!(
                    "mqtt '{}': mqtts:// needs TLS, which this build does not have; use mqtt:// to a loopback broker",
                    self.name
                )
            } else {
                format!("mqtt '{}': url must start with mqtt://", self.name)
            }));
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !p.contains(']') => (
                h,
                p.parse::<u16>().map_err(|_| {
                    PiiError::Invalid(format!("mqtt '{}': bad port in url", self.name))
                })?,
            ),
            _ => (authority, 1883),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err(PiiError::Invalid(format!(
                "mqtt '{}': url has no host",
                self.name
            )));
        }
        Ok((host.to_owned(), port))
    }

    /// Checks everything that must hold before connecting; returns the
    /// endpoint. Never includes a credential in an error.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a bad url/qos/bounds, or on credentials bound
    /// for a non-loopback plaintext broker without the opt-in.
    pub fn validate(&self) -> Result<(String, u16), PiiError> {
        let (host, port) = self.endpoint()?;
        if self.qos > 2 {
            return Err(PiiError::Invalid(format!(
                "mqtt '{}': qos must be 0, 1 or 2",
                self.name
            )));
        }
        if self.queue_cap == 0 || self.reconnect_min_ms == 0 || self.reconnect_max_ms == 0 {
            return Err(PiiError::Invalid(format!(
                "mqtt '{}': queue_cap and the reconnect bounds must be non-zero",
                self.name
            )));
        }
        let has_creds = self.username.is_some() || self.password.is_some();
        if has_creds && !is_loopback(&host) && !self.allow_plaintext_credentials {
            return Err(PiiError::Invalid(format!(
                "mqtt '{}': refusing to send credentials in plaintext to non-loopback broker '{host}'; \
                 TLS is unavailable in this build, set allow_plaintext_credentials = true to accept the risk",
                self.name
            )));
        }
        Ok((host, port))
    }

    /// True when this config sends credentials to a non-loopback host.
    #[must_use]
    pub fn plaintext_credentials_opted_in(&self) -> bool {
        let creds = self.username.is_some() || self.password.is_some();
        creds
            && self.allow_plaintext_credentials
            && self.endpoint().is_ok_and(|(h, _)| !is_loopback(&h))
    }
}

impl std::fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |o: &Option<String>| if o.is_some() { "<redacted>" } else { "<none>" };
        f.debug_struct("MqttConfig")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("client_id", &self.client_id)
            .field("topics", &self.topics)
            .field("allow_devices", &self.allow_devices)
            .field("qos", &self.qos)
            .field("username", &set(&self.username))
            .field("password", &set(&self.password))
            .field(
                "allow_plaintext_credentials",
                &self.allow_plaintext_credentials,
            )
            .finish_non_exhaustive()
    }
}

/// True for `localhost` and loopback IPs. A name is never resolved here: an
/// unknown name counts as non-loopback, which is the safe answer.
#[must_use]
pub fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Delay before reconnect attempt number `failures` (0 = first retry):
/// `min * 2^failures`, capped at `max`. Never overflows; `max` wins over `min`.
#[must_use]
pub fn backoff_ms(failures: u32, min: u64, max: u64) -> u64 {
    let doubled = 1u64
        .checked_shl(failures)
        .and_then(|m| min.checked_mul(m))
        .unwrap_or(u64::MAX);
    doubled.min(max)
}
