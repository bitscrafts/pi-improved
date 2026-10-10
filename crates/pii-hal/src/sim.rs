//! The SIMULATED backend: in-memory pins, a scriptable input sequence, fake
//! addressable I2C devices and an SPI loopback. Deterministic and offline.
//!
//! Nothing here touches hardware; every result it produces is a simulation
//! result and must be labelled so wherever it is reported.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use parking_lot::Mutex;
use pii_abi::PiiError;

use crate::backend::HalBackend;
use crate::types::{Direction, PinLevel};

struct SimPin {
    claimed: Option<Direction>,
    level: PinLevel,
    script: VecDeque<PinLevel>,
    last_scripted: Option<PinLevel>,
}

impl SimPin {
    fn new() -> Self {
        Self {
            claimed: None,
            level: PinLevel::Low,
            script: VecDeque::new(),
            last_scripted: None,
        }
    }
}

#[derive(Default)]
struct SimState {
    pins: BTreeMap<u32, SimPin>,
    /// `(bus, addr)` -> sparse register file (an unset register reads 0).
    i2c: BTreeMap<(u8, u8), BTreeMap<u8, u8>>,
    /// Register pointer per device, advanced by every byte moved.
    i2c_ptr: BTreeMap<(u8, u8), u8>,
    spi: BTreeSet<u8>,
}

/// An in-memory device. See the module docs.
pub struct SimBackend {
    state: Mutex<SimState>,
}

impl SimBackend {
    /// A device with nothing on it.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            state: Mutex::new(SimState::default()),
        }
    }

    /// The standard simulated board: GPIO lines `0..32`; I2C bus 1 with a
    /// device at `0x48` (register 0 = `0x5A`) and one at `0x49` (register 0 =
    /// `0x5B`); SPI buses 0 and 1, both loopback.
    #[must_use]
    pub fn standard() -> Self {
        Self::empty()
            .with_pins(32)
            .with_i2c_device(1, 0x48, &[(0x00, 0x5A)])
            .with_i2c_device(1, 0x49, &[(0x00, 0x5B)])
            .with_spi_bus(0)
            .with_spi_bus(1)
    }

    /// Builder: GPIO lines `0..count`, all low and unclaimed.
    #[must_use]
    pub fn with_pins(self, count: u32) -> Self {
        {
            let mut s = self.state.lock();
            for p in 0..count {
                s.pins.insert(p, SimPin::new());
            }
        }
        self
    }

    /// Builder: a fake I2C device with the given `(register, value)` preload.
    #[must_use]
    pub fn with_i2c_device(self, bus: u8, addr: u8, regs: &[(u8, u8)]) -> Self {
        {
            let mut s = self.state.lock();
            s.i2c.insert((bus, addr), regs.iter().copied().collect());
        }
        self
    }

    /// Builder: an SPI bus that loops every transmitted byte back.
    #[must_use]
    pub fn with_spi_bus(self, bus: u8) -> Self {
        self.state.lock().spi.insert(bus);
        self
    }

    /// Scripts the levels a claimed *input* `pin` reads: each read pops the
    /// next one, then the last value holds.
    pub fn script_input(&self, pin: u32, levels: &[PinLevel]) {
        if let Some(p) = self.state.lock().pins.get_mut(&pin) {
            p.script = levels.iter().copied().collect();
        }
    }

    /// The simulated line level, or `None` when the device has no such pin.
    #[must_use]
    pub fn level(&self, pin: u32) -> Option<PinLevel> {
        self.state.lock().pins.get(&pin).map(|p| p.level)
    }

    /// True when the backend itself holds `pin` as claimed (i.e. the HAL let a
    /// claim through to it).
    #[must_use]
    pub fn is_claimed(&self, pin: u32) -> bool {
        self.state
            .lock()
            .pins
            .get(&pin)
            .is_some_and(|p| p.claimed.is_some())
    }
}

fn no_pin(pin: u32) -> PiiError {
    PiiError::NotFound(format!("sim: no pin {pin}"))
}

impl HalBackend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn has_pin(&self, pin: u32) -> bool {
        self.state.lock().pins.contains_key(&pin)
    }

    fn has_i2c(&self, bus: u8, addr: u8) -> bool {
        self.state.lock().i2c.contains_key(&(bus, addr))
    }

    fn has_spi(&self, bus: u8) -> bool {
        self.state.lock().spi.contains(&bus)
    }

    fn claim_pin(&self, pin: u32, dir: Direction) -> Result<(), PiiError> {
        let mut s = self.state.lock();
        let p = s.pins.get_mut(&pin).ok_or_else(|| no_pin(pin))?;
        p.claimed = Some(dir);
        Ok(())
    }

    fn release_pin(&self, pin: u32) {
        if let Some(p) = self.state.lock().pins.get_mut(&pin) {
            p.claimed = None;
        }
    }

    fn read_pin(&self, pin: u32) -> Result<PinLevel, PiiError> {
        let mut s = self.state.lock();
        let p = s.pins.get_mut(&pin).ok_or_else(|| no_pin(pin))?;
        if p.claimed == Some(Direction::In) {
            if let Some(next) = p.script.pop_front() {
                p.last_scripted = Some(next);
                p.level = next;
            } else if let Some(last) = p.last_scripted {
                p.level = last;
            }
        }
        Ok(p.level)
    }

    fn write_pin(&self, pin: u32, level: PinLevel) -> Result<(), PiiError> {
        let mut s = self.state.lock();
        let p = s.pins.get_mut(&pin).ok_or_else(|| no_pin(pin))?;
        p.level = level;
        Ok(())
    }

    fn i2c_transaction(
        &self,
        bus: u8,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> Result<Vec<u8>, PiiError> {
        let mut s = self.state.lock();
        let key = (bus, addr);
        let mut ptr = s.i2c_ptr.get(&key).copied().unwrap_or(0);
        let regs = s
            .i2c
            .get_mut(&key)
            .ok_or_else(|| PiiError::NotFound(format!("sim: no i2c device {bus}:{addr:#04x}")))?;
        if let Some((first, data)) = write.split_first() {
            ptr = *first;
            for b in data {
                regs.insert(ptr, *b);
                ptr = ptr.wrapping_add(1);
            }
        }
        let mut out = Vec::with_capacity(read_len);
        for _ in 0..read_len {
            out.push(regs.get(&ptr).copied().unwrap_or(0));
            ptr = ptr.wrapping_add(1);
        }
        s.i2c_ptr.insert(key, ptr);
        Ok(out)
    }

    fn spi_transfer(&self, bus: u8, tx: &[u8]) -> Result<Vec<u8>, PiiError> {
        if self.state.lock().spi.contains(&bus) {
            Ok(tx.to_vec())
        } else {
            Err(PiiError::NotFound(format!("sim: no spi bus {bus}")))
        }
    }
}
