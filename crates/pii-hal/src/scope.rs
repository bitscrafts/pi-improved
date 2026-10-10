//! The capability scope of the HAL: which pins and buses an operator declared.
//!
//! Same discipline as `pii_core::PathScope`: empty permits nothing, and the
//! only combining operation is [`HalScope::intersect`], which never widens.

use std::collections::BTreeSet;

use pii_abi::PiiError;

/// The pins, I2C addresses and SPI buses a HAL may touch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HalScope {
    pins: BTreeSet<u32>,
    i2c: BTreeSet<(u8, u8)>,
    spi: BTreeSet<u8>,
}

impl HalScope {
    /// A scope permitting nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builder: also permit GPIO line `pin`.
    #[must_use]
    pub fn with_pin(mut self, pin: u32) -> Self {
        self.pins.insert(pin);
        self
    }

    /// Builder: also permit I2C address `addr` on `bus`.
    #[must_use]
    pub fn with_i2c(mut self, bus: u8, addr: u8) -> Self {
        self.i2c.insert((bus, addr));
        self
    }

    /// Builder: also permit SPI bus `bus`.
    #[must_use]
    pub fn with_spi(mut self, bus: u8) -> Self {
        self.spi.insert(bus);
        self
    }

    /// Builds a scope from the `[hal]` config values.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when an I2C entry is not `bus:addr`.
    pub fn from_config(pins: &[u32], i2c: &[String], spi: &[u8]) -> Result<Self, PiiError> {
        let mut s = Self::empty();
        for p in pins {
            s = s.with_pin(*p);
        }
        for entry in i2c {
            let (bus, addr) = Self::parse_i2c(entry)?;
            s = s.with_i2c(bus, addr);
        }
        for b in spi {
            s = s.with_spi(*b);
        }
        Ok(s)
    }

    /// Parses `bus:addr` (`1:0x48` or `1:72`); the address is 7-bit.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a malformed entry or an address above `0x7f`.
    pub fn parse_i2c(entry: &str) -> Result<(u8, u8), PiiError> {
        let bad = || {
            PiiError::Invalid(format!(
                "hal: i2c scope '{entry}' is not 'bus:addr' with a 7-bit address"
            ))
        };
        let (bus, addr) = entry.split_once(':').ok_or_else(bad)?;
        let bus: u8 = bus.trim().parse().map_err(|_| bad())?;
        let addr = addr.trim();
        let addr = addr
            .strip_prefix("0x")
            .map_or_else(|| addr.parse(), |hex| u8::from_str_radix(hex, 16))
            .map_err(|_| bad())?;
        if addr > 0x7f {
            return Err(bad());
        }
        Ok((bus, addr))
    }

    /// True when `pin` is declared.
    #[must_use]
    pub fn permits_pin(&self, pin: u32) -> bool {
        self.pins.contains(&pin)
    }

    /// True when `addr` on `bus` is declared.
    #[must_use]
    pub fn permits_i2c(&self, bus: u8, addr: u8) -> bool {
        self.i2c.contains(&(bus, addr))
    }

    /// True when SPI `bus` is declared.
    #[must_use]
    pub fn permits_spi(&self, bus: u8) -> bool {
        self.spi.contains(&bus)
    }

    /// Both scopes must permit: the result never widens either side.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            pins: self.pins.intersection(&other.pins).copied().collect(),
            i2c: self.i2c.intersection(&other.i2c).copied().collect(),
            spi: self.spi.intersection(&other.spi).copied().collect(),
        }
    }

    /// The declared pins, comma-separated, for health.
    #[must_use]
    pub fn pins_csv(&self) -> String {
        let v: Vec<String> = self.pins.iter().map(u32::to_string).collect();
        v.join(",")
    }
}
