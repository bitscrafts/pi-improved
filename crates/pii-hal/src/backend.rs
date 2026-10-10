//! The backend trait: what a driver implements. Policy (claims, scope) is NOT
//! here — it lives in [`crate::Hal`], so no backend can bypass it.

use pii_abi::PiiError;

use crate::types::{Direction, PinLevel};

/// A hardware driver. Implementations may assume [`crate::Hal`] has already
/// checked scope and claims; they report device facts and perform the I/O.
pub trait HalBackend: Send + Sync {
    /// Short backend name (`sim`, `gpiod`), reported in health.
    fn name(&self) -> &'static str;
    /// True when the device has this GPIO line.
    fn has_pin(&self, pin: u32) -> bool;
    /// True when the device has an I2C peripheral at `bus`/`addr`.
    fn has_i2c(&self, bus: u8, addr: u8) -> bool;
    /// True when the device has this SPI bus.
    fn has_spi(&self, bus: u8) -> bool;
    /// Requests a line from the device.
    ///
    /// # Errors
    /// A backend fault (the line is busy, the chip vanished).
    fn claim_pin(&self, pin: u32, dir: Direction) -> Result<(), PiiError>;
    /// Gives a line back. Infallible: a release must always be able to proceed.
    fn release_pin(&self, pin: u32);
    /// Reads a line.
    ///
    /// # Errors
    /// A backend fault.
    fn read_pin(&self, pin: u32) -> Result<PinLevel, PiiError>;
    /// Drives a line.
    ///
    /// # Errors
    /// A backend fault.
    fn write_pin(&self, pin: u32, level: PinLevel) -> Result<(), PiiError>;
    /// One I2C transaction: write `write` (may be empty), then read `read_len` bytes.
    ///
    /// # Errors
    /// A backend fault, or the backend does not implement I2C.
    fn i2c_transaction(
        &self,
        bus: u8,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> Result<Vec<u8>, PiiError>;
    /// One full-duplex SPI transfer: the bytes clocked in while `tx` is clocked out.
    ///
    /// # Errors
    /// A backend fault, or the backend does not implement SPI.
    fn spi_transfer(&self, bus: u8, tx: &[u8]) -> Result<Vec<u8>, PiiError>;
}
