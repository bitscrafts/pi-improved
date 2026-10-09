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
//! This crate is the contract only. Adopting it across crates, the
//! enforcement lint, and the health payload are specs 98, 99 and 100.

#![forbid(unsafe_code)]

mod event;
mod health;
mod measure;
pub mod names;
mod observer;

pub use event::{Event, Fields, Level, OwnedEvent, SpanId};
pub use health::{Health, State, SubsystemHealth};
pub use measure::Measure;
pub use observer::{
    NoopObserver, NullObserver, Observable, Observed, Observer, Recorder, StderrObserver,
};
