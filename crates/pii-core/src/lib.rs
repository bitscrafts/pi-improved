//! `pii-core`: the enforcing capability context used by the whole engine.
//!
//! * **Caps** — [`CapSet`] with monotonic attenuation via intersection.
//! * **Budget** — carveable, never-duplicated accounting.
//! * **Cancel** — an ancestor-propagating [`CancelToken`].
//! * **Ctx** — the enforcing context that combines all four.
//!
//! This crate must never reach `redb`, `petgraph`, or `toml`.

#![forbid(unsafe_code)]

mod budget;
mod cancel;
mod cap;
mod childproc;
mod clock;
mod config;
mod ctx;
mod fscap;
mod net;
mod observe;
mod principal;

pub use budget::{Budget, BudgetKind, BudgetShare};
pub use cancel::CancelToken;
pub use cap::{ArgvScope, CapSet, HostScope, PROTECTED_RELATIVE_PATHS, PathScope, SecretScope};
pub use childproc::{ChildProc, ProcOutput, set_test_kill_delay_ms};
pub use clock::{Clock, FixedClock, SystemClock};
pub use config::env_var;
pub use ctx::Ctx;
pub use fscap::FsCap;
pub use net::HttpTransport;
pub use observe::{Metric, MultiObserver, NullObserver, ObsEvent, Observer, RecordingObserver};
pub use pii_abi::{CapKind, PiiError};
/// The dependency-free observability contract (spec 97); the legacy
/// `Observer`/`ObsEvent`/`Metric` types above stay as the compatibility surface.
pub use pii_observe as contract;
pub use principal::{Principal, TraceCtx};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
}
