//! `pii-observe`: the observability contract every crate can depend on.
//!
//! This crate defines the vocabulary — [`Event`], [`Measure`], [`Observer`],
//! [`Observable`], [`Health`] — that subsystems use to report what they are
//! doing and what they are, plus a [`names`] registry so call sites and tests
//! assert on symbols rather than string literals.
//!
//! It is the **dependency floor**: its only dependencies are `serde` and
//! `serde_json`, with no workspace crate and no async runtime, so any crate
//! (including `pii-proto`, `pii-tui`, `pii-web`) can depend on it without
//! creating an upward edge.
//!
//! Spec 103 adds correlation ([`RunId`], [`SessionId`]), a verbatim run
//! [`Outcome`], a [`RedactionPolicy`], and the std-only [`TrajectorySink`]
//! (file writing lives here, not in a new crate: it needs only `std::fs`, so
//! the dependency floor is unchanged).

#![forbid(unsafe_code)]

mod event;
mod filelog;
mod health;
mod measure;
pub mod names;
mod observer;
mod outcome;
mod redaction;
mod trajectory;

pub use event::{Event, Fields, Level, OwnedEvent, RunId, SessionId, SpanId};
pub use filelog::FileLog;
pub use health::{Health, State, SubsystemHealth};
pub use measure::Measure;
pub use observer::{
    NoopObserver, NullObserver, Observable, Observed, Observer, Recorder, StderrObserver,
};
pub use outcome::{Outcome, Verdict};
pub use redaction::{DropFields, NoRedaction, Redaction, RedactionPolicy};
pub use trajectory::{TrajectoryCaps, TrajectorySink};
