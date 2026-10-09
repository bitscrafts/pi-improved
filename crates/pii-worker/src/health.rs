//! [`Health`] for a supervised worker: its state, pid, build tag and restarts.

use pii_observe::{Health, State, SubsystemHealth};

use crate::spec::WorkerState;
use crate::supervisor::Supervisor;

impl Health for Supervisor {
    fn health(&self) -> SubsystemHealth {
        let info = self.info();
        let mut h = SubsystemHealth::ok(format!("worker.{}", info.name));
        h.state = match info.state {
            WorkerState::Ready | WorkerState::Starting => State::Ok,
            WorkerState::Draining | WorkerState::Restarting => State::Degraded,
            WorkerState::Failed => State::Down,
        };
        h.identity
            .insert("state".into(), format!("{:?}", info.state));
        h.identity.insert("pid".into(), info.pid.to_string());
        h.identity.insert("tag".into(), info.tag);
        h.identity
            .insert("restarts".into(), info.restarts.to_string());
        h.identity
            .insert("in_flight".into(), info.in_flight.to_string());
        h
    }
}
