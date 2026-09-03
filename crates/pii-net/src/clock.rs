//! Clock re-export (F1.3): `pii-net` consumes `pii-core`'s injectable clock;
//! the only `SystemTime::now` site stays in `pii_core::SystemClock`.

pub use pii_core::{Clock, FixedClock, SystemClock};
