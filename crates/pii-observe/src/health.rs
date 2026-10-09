//! Health: how a subsystem reports its state and what it actually is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Coarse subsystem state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Working normally.
    Ok,
    /// Working with reduced function.
    Degraded,
    /// Not working.
    Down,
}

/// A subsystem's health report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemHealth {
    /// Subsystem name.
    pub name: String,
    /// Current state.
    pub state: State,
    /// Human-readable detail, if any.
    pub detail: Option<String>,
    /// What the subsystem actually is: paths, versions, counts.
    pub identity: BTreeMap<String, String>,
}

impl SubsystemHealth {
    /// A healthy report with no detail or identity.
    #[must_use]
    pub fn ok(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: State::Ok,
            detail: None,
            identity: BTreeMap::new(),
        }
    }
}

/// Implemented by anything that can report its health.
pub trait Health {
    /// The current health report.
    fn health(&self) -> SubsystemHealth;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn subsystem_health_serde_round_trip() {
        let mut h = SubsystemHealth::ok("store");
        h.state = State::Degraded;
        h.detail = Some("slow".into());
        h.identity.insert("path".into(), "/tmp/x.redb".into());
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains(r#""state":"degraded""#));
        assert_eq!(serde_json::from_str::<SubsystemHealth>(&json).unwrap(), h);
    }

    #[test]
    fn state_orders_by_severity() {
        assert!(State::Ok < State::Degraded && State::Degraded < State::Down);
    }
}
