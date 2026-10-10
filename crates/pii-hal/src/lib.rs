//! `pii-hal` (spec 121): the hardware HAL — GPIO, I2C and SPI behind one
//! contract.
//!
//! * [`HalBackend`] is what a driver implements; [`Hal`] is what callers hold.
//!   `Hal` is where the rules live: a pin or bus must be **claimed** before use
//!   ([`Hal::claim_pin`]), a claim is bounded by a [`HalScope`] (the pins and
//!   buses the operator declared, attenuation-only like `pii_core::PathScope`),
//!   a double claim is refused (not idempotent), and a refusal is a
//!   `PiiError::Denied` raised before the backend is touched.
//! * [`SimBackend`] is the simulated backend: in-memory pins, a scriptable
//!   input sequence, addressable fake I2C devices, an SPI loopback. It is the
//!   only backend this host can hold to account, and every test uses it.
//! * `GpiodBackend` (feature `hal-gpiod`) is the real GPIO backend over
//!   `/dev/gpiochipN`. It is compiled by the gate and **never run** here.
//! * The HAL is **off unless configured**: [`HalHub`] starts empty and
//!   [`HalHub::require`] refuses until a daemon fills it from `[hal]`.

#![forbid(unsafe_code)]
// reason: every lock here is held on purpose across the whole operation — the
// claim check and the claim (or the backend call) must be one atomic step, or
// two callers could both pass "not yet claimed".
#![allow(clippy::significant_drop_tightening)]

mod backend;
mod hal;
mod hub;
mod scope;
mod sim;
mod types;

#[cfg(feature = "hal-gpiod")]
mod gpiod;

pub use backend::HalBackend;
pub use hal::{Hal, HalCounts};
pub use hub::HalHub;
pub use scope::HalScope;
pub use sim::SimBackend;
pub use types::{Direction, PinLevel};

#[cfg(feature = "hal-gpiod")]
pub use gpiod::GpiodBackend;
