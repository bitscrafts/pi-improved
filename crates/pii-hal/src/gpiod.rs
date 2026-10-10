//! The REAL GPIO backend over the Linux GPIO character device
//! (`/dev/gpiochipN`), via the `gpiocdev` crate (0.8, optional, feature
//! `hal-gpiod`).
//!
//! **Never run on this host.** The container has no `/dev/gpiochip*`; this
//! module is compiled and linted by the gate so it cannot rot, and nothing has
//! exercised it against a line. I2C and SPI are not implemented here: they
//! return [`PiiError::Unavailable`] rather than pretending.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gpiocdev::Request;
use gpiocdev::line::Value;
use parking_lot::Mutex;
use pii_abi::PiiError;

use crate::backend::HalBackend;
use crate::types::{Direction, PinLevel};

/// GPIO lines of one chip, requested lazily at claim time.
pub struct GpiodBackend {
    chip: PathBuf,
    requests: Mutex<BTreeMap<u32, Request>>,
}

impl GpiodBackend {
    /// A backend for the chip at `chip` (for example `/dev/gpiochip0`).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] when the chip cannot be opened.
    pub fn open(chip: impl Into<PathBuf>) -> Result<Self, PiiError> {
        let chip = chip.into();
        gpiocdev::chip::Chip::from_path(&chip)
            .map_err(|e| PiiError::Unavailable(format!("gpiod: open {}: {e}", chip.display())))?;
        Ok(Self {
            chip,
            requests: Mutex::new(BTreeMap::new()),
        })
    }

    fn fault(what: &str, e: &gpiocdev::Error) -> PiiError {
        PiiError::Unavailable(format!("gpiod: {what}: {e}"))
    }
}

impl HalBackend for GpiodBackend {
    fn name(&self) -> &'static str {
        "gpiod"
    }

    fn has_pin(&self, pin: u32) -> bool {
        gpiocdev::chip::Chip::from_path(&self.chip)
            .and_then(|c| c.info())
            .is_ok_and(|i| pin < i.num_lines)
    }

    fn has_i2c(&self, _bus: u8, _addr: u8) -> bool {
        false
    }

    fn has_spi(&self, _bus: u8) -> bool {
        false
    }

    fn claim_pin(&self, pin: u32, dir: Direction) -> Result<(), PiiError> {
        let mut b = Request::builder();
        b.on_chip(&self.chip)
            .with_line(pin)
            .with_consumer("pii-hal");
        match dir {
            Direction::In => b.as_input(),
            Direction::Out => b.as_output(Value::Inactive),
        };
        let req = b.request().map_err(|e| Self::fault("request line", &e))?;
        self.requests.lock().insert(pin, req);
        Ok(())
    }

    fn release_pin(&self, pin: u32) {
        self.requests.lock().remove(&pin);
    }

    fn read_pin(&self, pin: u32) -> Result<PinLevel, PiiError> {
        let g = self.requests.lock();
        let req = g
            .get(&pin)
            .ok_or_else(|| PiiError::Internal(format!("gpiod: pin {pin} has no request")))?;
        let v = req.value(pin).map_err(|e| Self::fault("read", &e))?;
        Ok(PinLevel::from(v == Value::Active))
    }

    fn write_pin(&self, pin: u32, level: PinLevel) -> Result<(), PiiError> {
        let g = self.requests.lock();
        let req = g
            .get(&pin)
            .ok_or_else(|| PiiError::Internal(format!("gpiod: pin {pin} has no request")))?;
        let v = if level.is_high() {
            Value::Active
        } else {
            Value::Inactive
        };
        req.set_value(pin, v).map_err(|e| Self::fault("write", &e))
    }

    fn i2c_transaction(&self, _: u8, _: u8, _: &[u8], _: usize) -> Result<Vec<u8>, PiiError> {
        Err(PiiError::Unavailable(String::from(
            "gpiod: I2C is not implemented by the GPIO backend",
        )))
    }

    fn spi_transfer(&self, _: u8, _: &[u8]) -> Result<Vec<u8>, PiiError> {
        Err(PiiError::Unavailable(String::from(
            "gpiod: SPI is not implemented by the GPIO backend",
        )))
    }
}
