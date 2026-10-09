//! `pii-tui`: a pure protocol client (F5.1–F5.4).
//!
//! Holds **no domain state** (F5.2): no scheduler, no graph, no store, no
//! model. `AppState` is populated only from protocol messages. `render` is
//! pure and testable against a `TestBackend`.

mod driver;
mod input;
pub mod render;
mod state;

pub use driver::run_tui;
pub use render::render;
pub use state::{AppState, ConnStatus, LogLine, TuiError};
// observability: none — pending the spec 100 adoption sweep (no seam yet)
