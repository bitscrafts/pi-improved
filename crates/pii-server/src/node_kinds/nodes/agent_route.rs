//! Reading what an agent says about leaving the route it was asked to use.
//!
//! pi can swap models mid-turn when `retry.fallback_chains` is configured, and the swap spends a different provider's
//! quota. Nothing in the coder asked for that, so it must show in the journal as drift, not as a pass.

use super::agent_worker::reported_routes;

/// One model swap pi reported with a `failover_start` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failover {
    /// `provider/model` the turn was on.
    pub from: String,
    /// `provider/model` the turn moved to.
    pub to: String,
    /// The failure class that triggered it: `quota`, `overload` or `transient`.
    pub class: String,
}

/// The failovers an agent's JSON event stream reports, in order.
///
/// The event shape is taken from pi v0.4.0 (`AgentEvent::FailoverStart`: tag `type`, camelCase fields). The installed
/// pi is 0.2.0, which cannot emit it, so this has not yet been checked against a real stream.
#[must_use]
pub fn failovers(output: &str) -> Vec<Failover> {
    let text = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("type").and_then(serde_json::Value::as_str) == Some("failover_start"))
        .map(|v| Failover {
            from: format!("{}/{}", text(&v, "fromProvider"), text(&v, "fromModel")),
            to: format!("{}/{}", text(&v, "toProvider"), text(&v, "toModel")),
            class: text(&v, "class"),
        })
        .collect()
}

/// Every route in `reported` that is not the `requested` one, as `provider/model`.
///
/// An empty `reported` is unknown, not drift. A requested model that is only an alias counts as the same route when
/// the reported id contains it (`haiku` in `claude-haiku-5-5`), because claude reports the full id.
#[must_use]
pub fn route_drift(requested: &(String, String), reported: &[(String, String)]) -> Vec<String> {
    reported
        .iter()
        .filter(|(p, m)| !(p == &requested.0 && (m == &requested.1 || m.contains(&requested.1))))
        .map(|(p, m)| format!("{p}/{m}"))
        .collect()
}

/// The routes an output reported that differ from the requested one.
#[must_use]
pub fn drift_of(requested: &(String, String), output: &str) -> Vec<String> {
    route_drift(requested, &reported_routes(output))
}
