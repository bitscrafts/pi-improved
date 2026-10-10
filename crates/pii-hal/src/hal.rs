//! [`Hal`]: the claim and scope rules in front of a [`HalBackend`], with the
//! observability seam. This is where a refusal is decided — before the backend
//! is touched.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_observe::{
    Event, Health, Level, Observable, Observed, Observer, State, SubsystemHealth, names,
};

use crate::backend::HalBackend;
use crate::scope::HalScope;
use crate::types::{Direction, PinLevel};

/// What an operation acts on.
#[derive(Debug, Clone, Copy)]
enum Target {
    Pin(u32),
    I2c(u8, u8),
    Spi(u8),
}

impl Target {
    const fn kind(self) -> &'static str {
        match self {
            Self::Pin(_) => "gpio",
            Self::I2c(..) => "i2c",
            Self::Spi(_) => "spi",
        }
    }

    fn describe(self) -> String {
        match self {
            Self::Pin(p) => format!("pin {p}"),
            Self::I2c(b, a) => format!("i2c {b}:{a:#04x}"),
            Self::Spi(b) => format!("spi bus {b}"),
        }
    }

    /// The target as event fields: `pin`, or `bus`+`addr`, or `bus`.
    fn tag(self, ev: Event) -> Event {
        let ev = ev.field("kind", self.kind());
        match self {
            Self::Pin(p) => ev.field("pin", u64::from(p)),
            Self::I2c(b, a) => ev.field("bus", u64::from(b)).field("addr", u64::from(a)),
            Self::Spi(b) => ev.field("bus", u64::from(b)),
        }
    }
}

/// Operation counts since the HAL was built (spec 121 R5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HalCounts {
    /// Claims that succeeded.
    pub claim: u64,
    /// Releases that succeeded.
    pub release: u64,
    /// Pin reads that succeeded.
    pub read: u64,
    /// Pin writes that succeeded.
    pub write: u64,
    /// I2C transactions and SPI transfers that succeeded.
    pub transaction: u64,
    /// Operations refused by the claim or scope rules.
    pub refusals: u64,
}

#[derive(Default)]
struct Inner {
    pins: BTreeMap<u32, Direction>,
    i2c: BTreeSet<(u8, u8)>,
    spi: BTreeSet<u8>,
    counts: HalCounts,
    last_error: Option<String>,
    /// True while the last failure was a backend fault (cleared by a success).
    degraded: bool,
}

/// The HAL a caller holds: claim, then use, inside a declared [`HalScope`].
pub struct Hal {
    backend: Arc<dyn HalBackend>,
    scope: HalScope,
    observer: Option<Arc<dyn Observer>>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Hal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hal")
            .field("backend", &self.backend.name())
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl Observable for Hal {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

impl Hal {
    /// A HAL over `backend`, limited to `scope`, reporting to `observer`.
    #[must_use]
    pub fn new(
        backend: Arc<dyn HalBackend>,
        scope: HalScope,
        observer: Option<Arc<dyn Observer>>,
    ) -> Self {
        Self {
            backend,
            scope,
            observer,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Operation counts so far.
    #[must_use]
    pub fn counts(&self) -> HalCounts {
        self.inner.lock().counts
    }

    fn refuse(
        &self,
        g: &mut Inner,
        op: &'static str,
        t: Target,
        reason: &str,
        msg: String,
    ) -> PiiError {
        g.counts.refusals += 1;
        self.note_error(g, op, t, reason, &msg, false);
        PiiError::Denied(msg)
    }

    fn fault(&self, g: &mut Inner, op: &'static str, t: Target, e: PiiError) -> PiiError {
        self.note_error(g, op, t, "backend", &e.to_string(), true);
        e
    }

    fn note_error(
        &self,
        g: &mut Inner,
        op: &'static str,
        t: Target,
        reason: &str,
        msg: &str,
        degraded: bool,
    ) {
        g.last_error = Some(msg.to_owned());
        g.degraded = degraded;
        self.emit(
            &t.tag(Event::new("hal", Level::Warn, names::HAL_ERROR))
                .field("op", op)
                .field("reason", reason)
                .field("error", msg),
        );
    }

    fn ok(g: &mut Inner) {
        g.degraded = false;
    }

    fn in_scope(&self, g: &mut Inner, op: &'static str, t: Target) -> Result<(), PiiError> {
        let allowed = match t {
            Target::Pin(p) => self.scope.permits_pin(p),
            Target::I2c(b, a) => self.scope.permits_i2c(b, a),
            Target::Spi(b) => self.scope.permits_spi(b),
        };
        if allowed {
            return Ok(());
        }
        let msg = format!("hal: {} is outside the declared scope", t.describe());
        Err(self.refuse(g, op, t, "out_of_scope", msg))
    }

    fn exists(&self, g: &mut Inner, op: &'static str, t: Target) -> Result<(), PiiError> {
        let present = match t {
            Target::Pin(p) => self.backend.has_pin(p),
            Target::I2c(b, a) => self.backend.has_i2c(b, a),
            Target::Spi(b) => self.backend.has_spi(b),
        };
        if present {
            return Ok(());
        }
        let e = PiiError::NotFound(format!("hal: the device has no {}", t.describe()));
        Err(self.fault(g, op, t, e))
    }

    fn claimed(g: &Inner, t: Target) -> bool {
        match t {
            Target::Pin(p) => g.pins.contains_key(&p),
            Target::I2c(b, a) => g.i2c.contains(&(b, a)),
            Target::Spi(b) => g.spi.contains(&b),
        }
    }

    /// Scope, then claim state: the check every use of a claimed target shares.
    fn use_check(&self, g: &mut Inner, op: &'static str, t: Target) -> Result<(), PiiError> {
        self.in_scope(g, op, t)?;
        if Self::claimed(g, t) {
            return Ok(());
        }
        let msg = format!("hal: {} is not claimed", t.describe());
        Err(self.refuse(g, op, t, "unclaimed", msg))
    }

    fn claim(&self, t: Target, dir: Option<Direction>) -> Result<(), PiiError> {
        let mut g = self.inner.lock();
        self.in_scope(&mut g, "claim", t)?;
        if Self::claimed(&g, t) {
            let msg = format!("hal: {} is already claimed", t.describe());
            return Err(self.refuse(&mut g, "claim", t, "already_claimed", msg));
        }
        self.exists(&mut g, "claim", t)?;
        if let (Target::Pin(p), Some(d)) = (t, dir) {
            self.backend
                .claim_pin(p, d)
                .map_err(|e| self.fault(&mut g, "claim", t, e))?;
            g.pins.insert(p, d);
        }
        match t {
            Target::I2c(b, a) => {
                g.i2c.insert((b, a));
            }
            Target::Spi(b) => {
                g.spi.insert(b);
            }
            Target::Pin(_) => {}
        }
        g.counts.claim += 1;
        Self::ok(&mut g);
        let mut ev = t.tag(Event::new("hal", Level::Info, names::HAL_CLAIM));
        if let Some(d) = dir {
            ev = ev.field("dir", d.as_str());
        }
        self.emit(&ev);
        Ok(())
    }

    fn release(&self, t: Target) -> Result<(), PiiError> {
        let mut g = self.inner.lock();
        self.use_check(&mut g, "release", t)?;
        match t {
            Target::Pin(p) => {
                g.pins.remove(&p);
                self.backend.release_pin(p);
            }
            Target::I2c(b, a) => {
                g.i2c.remove(&(b, a));
            }
            Target::Spi(b) => {
                g.spi.remove(&b);
            }
        }
        g.counts.release += 1;
        Self::ok(&mut g);
        self.emit(&t.tag(Event::new("hal", Level::Info, names::HAL_RELEASE)));
        Ok(())
    }

    /// Claims GPIO line `pin` as `dir`.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when `pin` is outside the scope or already claimed;
    /// [`PiiError::NotFound`] when the device has no such line; a backend fault.
    pub fn claim_pin(&self, pin: u32, dir: Direction) -> Result<(), PiiError> {
        self.claim(Target::Pin(pin), Some(dir))
    }

    /// Releases a claimed pin.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the pin is outside the scope or not claimed.
    pub fn release_pin(&self, pin: u32) -> Result<(), PiiError> {
        self.release(Target::Pin(pin))
    }

    /// Claims I2C address `addr` on `bus`.
    ///
    /// # Errors
    /// As [`Hal::claim_pin`].
    pub fn claim_i2c(&self, bus: u8, addr: u8) -> Result<(), PiiError> {
        self.claim(Target::I2c(bus, addr), None)
    }

    /// Releases a claimed I2C address.
    ///
    /// # Errors
    /// As [`Hal::release_pin`].
    pub fn release_i2c(&self, bus: u8, addr: u8) -> Result<(), PiiError> {
        self.release(Target::I2c(bus, addr))
    }

    /// Claims SPI `bus`.
    ///
    /// # Errors
    /// As [`Hal::claim_pin`].
    pub fn claim_spi(&self, bus: u8) -> Result<(), PiiError> {
        self.claim(Target::Spi(bus), None)
    }

    /// Releases a claimed SPI bus.
    ///
    /// # Errors
    /// As [`Hal::release_pin`].
    pub fn release_spi(&self, bus: u8) -> Result<(), PiiError> {
        self.release(Target::Spi(bus))
    }

    /// Reads a claimed pin (an output pin reads back what was driven).
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the pin is outside the scope or not claimed; a backend fault.
    pub fn read_pin(&self, pin: u32) -> Result<PinLevel, PiiError> {
        let t = Target::Pin(pin);
        let mut g = self.inner.lock();
        self.use_check(&mut g, "read", t)?;
        let level = self
            .backend
            .read_pin(pin)
            .map_err(|e| self.fault(&mut g, "read", t, e))?;
        g.counts.read += 1;
        Self::ok(&mut g);
        self.emit(
            &t.tag(Event::new("hal", Level::Info, names::HAL_READ))
                .field("level", level.as_str()),
        );
        Ok(level)
    }

    /// Drives a pin claimed as an output.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the pin is outside the scope, not claimed, or
    /// claimed as an input; a backend fault.
    pub fn write_pin(&self, pin: u32, level: PinLevel) -> Result<(), PiiError> {
        let t = Target::Pin(pin);
        let mut g = self.inner.lock();
        self.use_check(&mut g, "write", t)?;
        if g.pins.get(&pin) == Some(&Direction::In) {
            let msg = format!("hal: pin {pin} is claimed as input and cannot be written");
            return Err(self.refuse(&mut g, "write", t, "wrong_direction", msg));
        }
        self.backend
            .write_pin(pin, level)
            .map_err(|e| self.fault(&mut g, "write", t, e))?;
        g.counts.write += 1;
        Self::ok(&mut g);
        self.emit(
            &t.tag(Event::new("hal", Level::Info, names::HAL_WRITE))
                .field("level", level.as_str()),
        );
        Ok(())
    }

    /// One I2C transaction on a claimed address: write `write`, then read `read_len` bytes.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the address is outside the scope or not claimed; a backend fault.
    pub fn i2c_transaction(
        &self,
        bus: u8,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> Result<Vec<u8>, PiiError> {
        let t = Target::I2c(bus, addr);
        let mut g = self.inner.lock();
        self.use_check(&mut g, "transaction", t)?;
        let out = self
            .backend
            .i2c_transaction(bus, addr, write, read_len)
            .map_err(|e| self.fault(&mut g, "transaction", t, e))?;
        self.transacted(&mut g, t, write.len(), out.len());
        Ok(out)
    }

    /// One SPI transfer on a claimed bus; returns the bytes clocked in.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the bus is outside the scope or not claimed; a backend fault.
    pub fn spi_transfer(&self, bus: u8, tx: &[u8]) -> Result<Vec<u8>, PiiError> {
        let t = Target::Spi(bus);
        let mut g = self.inner.lock();
        self.use_check(&mut g, "transaction", t)?;
        let out = self
            .backend
            .spi_transfer(bus, tx)
            .map_err(|e| self.fault(&mut g, "transaction", t, e))?;
        self.transacted(&mut g, t, tx.len(), out.len());
        Ok(out)
    }

    fn transacted(&self, g: &mut Inner, t: Target, write_len: usize, read_len: usize) {
        g.counts.transaction += 1;
        Self::ok(g);
        self.emit(
            &t.tag(Event::new("hal", Level::Info, names::HAL_TRANSACTION))
                .field("write_len", write_len as u64)
                .field("read_len", read_len as u64),
        );
    }
}

impl Health for Hal {
    fn health(&self) -> SubsystemHealth {
        let g = self.inner.lock();
        let csv = |v: Vec<String>| v.join(",");
        let mut h = SubsystemHealth::ok("hal");
        h.state = if g.degraded {
            State::Degraded
        } else {
            State::Ok
        };
        h.detail = g.degraded.then(|| g.last_error.clone()).flatten();
        h.identity
            .insert("backend".into(), self.backend.name().into());
        h.identity.insert(
            "claimed_pins".into(),
            csv(g.pins.keys().map(u32::to_string).collect()),
        );
        h.identity.insert(
            "claimed_i2c".into(),
            csv(g.i2c.iter().map(|(b, a)| format!("{b}:{a:#04x}")).collect()),
        );
        h.identity.insert(
            "claimed_spi".into(),
            csv(g.spi.iter().map(u8::to_string).collect()),
        );
        h.identity
            .insert("scope_pins".into(), self.scope.pins_csv());
        h.identity.insert(
            "last_error".into(),
            g.last_error.clone().unwrap_or_else(|| "none".into()),
        );
        h
    }
}
