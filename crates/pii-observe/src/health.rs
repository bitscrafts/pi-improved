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

/// Remembers each subsystem's last observed [`State`].
///
/// A change is reported once, as [`crate::names::HEALTH_STATE_CHANGED`]
/// (spec 109 R2). Health is pulled (`sys.status`), so a change is seen at the first poll after it.
#[derive(Debug, Default)]
// reason: pii-observe is the dependency floor (serde only); it cannot take
// parking_lot, and a poisoned lock here just skips one report.
#[allow(clippy::disallowed_types)]
pub struct HealthWatch {
    last: std::sync::Mutex<BTreeMap<String, State>>,
}

impl HealthWatch {
    /// A watch that has seen nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `h`; when the subsystem was seen before in another state,
    /// emits a `health.state.changed` event (`subsystem`, `from`, `to`,
    /// `detail`) to `observer`. The first sighting only records.
    pub fn note(&self, h: &SubsystemHealth, observer: &dyn crate::Observer) {
        let previous = self
            .last
            .lock()
            .ok()
            .and_then(|mut g| g.insert(h.name.clone(), h.state));
        let Some(from) = previous.filter(|p| *p != h.state) else {
            return;
        };
        let level = if h.state == State::Ok {
            crate::Level::Info
        } else {
            crate::Level::Warn
        };
        observer.event(
            &crate::Event::new("health", level, crate::names::HEALTH_STATE_CHANGED)
                .field("subsystem", h.name.as_str())
                .field("from", format!("{from:?}").to_lowercase())
                .field("to", format!("{:?}", h.state).to_lowercase())
                .field("detail", h.detail.clone().unwrap_or_default()),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

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

    #[test]
    fn a_state_change_is_reported_once_and_the_first_sighting_is_silent() {
        let rec = crate::Recorder::new();
        let w = HealthWatch::new();
        let mut h = SubsystemHealth::ok("store");
        w.note(&h, &rec);
        w.note(&h, &rec);
        assert!(
            rec.events().is_empty(),
            "first sighting and no change: silent"
        );
        h.state = State::Degraded;
        h.detail = Some("slow".into());
        w.note(&h, &rec);
        w.note(&h, &rec);
        let ev = rec.events_named(crate::names::HEALTH_STATE_CHANGED);
        assert_eq!(ev.len(), 1, "reported once, not per poll");
        assert_eq!(ev[0].fields.get("from"), Some(&serde_json::json!("ok")));
        assert_eq!(ev[0].fields.get("to"), Some(&serde_json::json!("degraded")));
        assert_eq!(
            ev[0].fields.get("subsystem"),
            Some(&serde_json::json!("store"))
        );
    }
}
