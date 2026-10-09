//! `pii-worker`: supervision and hot reload (F3.1–F3.6). The worker wire
//! vocabulary lives here, decoupled from `pii-proto` (F3.1).

pub mod control;
pub mod frame;
pub mod spec;
pub mod supervisor;

pub use frame::{Hello, WORKER_PROTOCOL, WorkerErr, WorkerFrame};
pub use spec::{ReloadReport, WorkerInfo, WorkerSpec, WorkerState};
pub use supervisor::Supervisor;
pub use supervisor::WorkerEvent;
// observability: none — pending the spec 100 adoption sweep (no seam yet)
