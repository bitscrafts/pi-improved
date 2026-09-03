//! Graph specification model (pass-1 §3.6, spec F4.1).

use pii_abi::{NodeId, PiiError, PortName};
use serde::{Deserialize, Serialize};

/// How an edge participates in scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// A value-carrying dependency (readiness + payload).
    Data,
    /// An ordering-only dependency (readiness only; no payload).
    Order,
}

/// How a node's inbound data edges gate its readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinPolicy {
    /// Every inbound edge must fire (serial join). Default.
    All,
    /// Any one inbound edge fires.
    Any {
        /// Cancel sibling branches once one wins.
        cancel_losers: bool,
    },
    /// At least `k` of the inbound edges fire.
    Quorum {
        /// Required number of firing edges.
        k: u16,
        /// Cancel sibling branches once the quorum is met.
        cancel_losers: bool,
    },
    /// Wait for every inbound edge to settle (fired or dead).
    AllSettled,
}

impl JoinPolicy {
    /// Parses a `JoinPolicy` from the compact string grammar.
    ///
    /// Valid: `"all"`, `"any"`, `"any:cancel"`, `"quorum:K"`, `"quorum:K:cancel"`,
    /// `"allsettled"`.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] for anything not in the grammar.
    pub fn parse(s: &str) -> Result<Self, PiiError> {
        match s {
            "all" => Ok(Self::All),
            "any" => Ok(Self::Any {
                cancel_losers: false,
            }),
            "any:cancel" => Ok(Self::Any {
                cancel_losers: true,
            }),
            "allsettled" => Ok(Self::AllSettled),
            other => {
                let parts: Vec<&str> = other.split(':').collect();
                let head = parts.first().copied().unwrap_or("");
                let cancel = parts.contains(&"cancel");
                if head == "quorum" {
                    let k_raw = parts.get(1).copied().unwrap_or("");
                    let k: u16 = k_raw.parse().map_err(|_| {
                        PiiError::Invalid(format!("join policy '{other}' has bad quorum k"))
                    })?;
                    Ok(Self::Quorum {
                        k,
                        cancel_losers: cancel,
                    })
                } else {
                    Err(PiiError::Invalid(format!("unknown join policy '{other}'")))
                }
            }
        }
    }
}

/// A directed edge between two nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// Source node.
    pub from: NodeId,
    /// The output port the edge leaves.
    pub port: PortName,
    /// Destination node.
    pub to: NodeId,
    /// Edge kind.
    pub kind: EdgeKind,
}

/// Retry policy for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RetryPolicy {
    /// Maximum retries before giving up (default 0).
    pub max: u32,
}

/// Execution limits for a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphLimits {
    /// Maximum scheduler steps before a run is failed (default 1000).
    pub max_steps: u32,
    /// Maximum wall-clock budget in ms (default `600_000`).
    pub max_wall_ms: u64,
    /// Maximum token budget (default `u64::MAX` = unbounded).
    pub max_tokens: u64,
    /// Maximum micro-dollar budget (default `u64::MAX` = unbounded).
    pub max_micro_usd: u64,
}

impl Default for GraphLimits {
    fn default() -> Self {
        Self {
            max_steps: 1000,
            max_wall_ms: 600_000,
            max_tokens: u64::MAX,
            max_micro_usd: u64::MAX,
        }
    }
}

/// The static description of one node in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Unique node id within the graph.
    pub id: NodeId,
    /// The node implementation kind (e.g. `transform`, `route`).
    pub kind: String,
    /// How inbound data edges gate readiness.
    pub join: JoinPolicy,
    /// Optional loop budget that permits a bounded cycle.
    pub loop_budget: Option<u32>,
    /// Retry policy.
    pub retry: RetryPolicy,
    /// Capability bits this node may use.
    pub caps: u32,
    /// Free-form implementation configuration.
    pub cfg: serde_json::Value,
}

/// A complete, validated-ready graph specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSpec {
    /// Human-readable graph name.
    pub name: String,
    /// Capability bits granted to the whole graph (scopes are empty; the CLI
    /// applies allow-scopes at run time).
    pub caps: u32,
    /// All nodes in declaration order.
    pub nodes: Vec<NodeSpec>,
    /// All edges.
    pub edges: Vec<Edge>,
    /// The entry node id (`entry`).
    pub entry: NodeId,
    /// Execution limits.
    pub limits: GraphLimits,
}

impl GraphSpec {
    /// Looks up a node by id.
    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&NodeSpec> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    /// Edge indices feeding `id` of a given kind, in declaration order.
    #[must_use]
    pub fn inbound(&self, id: &NodeId, kind: EdgeKind) -> Vec<usize> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| &e.to == id && e.kind == kind)
            .map(|(i, _)| i)
            .collect()
    }

    /// Edge indices leaving `id`, in declaration order.
    #[must_use]
    pub fn outbound(&self, id: &NodeId) -> Vec<usize> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| &e.from == id)
            .map(|(i, _)| i)
            .collect()
    }

    /// Serialises the spec to JSON bytes.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a serialisation failure.
    pub fn to_json(&self) -> Result<Vec<u8>, PiiError> {
        serde_json::to_vec(self).map_err(|e| PiiError::Internal(format!("spec to_json: {e}")))
    }

    /// Deserialises a spec from JSON bytes.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] on a deserialisation failure.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PiiError> {
        serde_json::from_slice(bytes).map_err(|e| PiiError::Invalid(format!("spec from_json: {e}")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
    use pii_abi::NodeId;

    #[test]
    fn join_policy_parse_grammar() {
        use JoinPolicy::*;
        assert_eq!(JoinPolicy::parse("all").unwrap(), All);
        assert_eq!(
            JoinPolicy::parse("any").unwrap(),
            Any {
                cancel_losers: false
            }
        );
        assert_eq!(
            JoinPolicy::parse("any:cancel").unwrap(),
            Any {
                cancel_losers: true
            }
        );
        assert_eq!(
            JoinPolicy::parse("quorum:2").unwrap(),
            Quorum {
                k: 2,
                cancel_losers: false
            }
        );
        assert_eq!(
            JoinPolicy::parse("quorum:2:cancel").unwrap(),
            Quorum {
                k: 2,
                cancel_losers: true
            }
        );
        assert_eq!(JoinPolicy::parse("allsettled").unwrap(), AllSettled);
        assert!(JoinPolicy::parse("bogus").is_err());
        assert!(JoinPolicy::parse("quorum:x").is_err());
    }

    #[test]
    fn spec_inbound_outbound_indexes() {
        let spec = GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes: vec![node("a"), node("b"), node("c")],
            edges: vec![
                Edge {
                    from: NodeId::from("a"),
                    port: "ok".into(),
                    to: NodeId::from("b"),
                    kind: EdgeKind::Data,
                },
                Edge {
                    from: NodeId::from("b"),
                    port: "ok".into(),
                    to: NodeId::from("c"),
                    kind: EdgeKind::Data,
                },
                Edge {
                    from: NodeId::from("a"),
                    port: "ok".into(),
                    to: NodeId::from("c"),
                    kind: EdgeKind::Data,
                },
            ],
            entry: NodeId::from("a"),
            limits: GraphLimits::default(),
        };
        let inbound_c = spec.inbound(&NodeId::from("c"), EdgeKind::Data);
        assert_eq!(inbound_c, vec![1, 2]);
        let outbound_a = spec.outbound(&NodeId::from("a"));
        assert_eq!(outbound_a, vec![0, 2]);
    }

    fn node(id: &str) -> NodeSpec {
        NodeSpec {
            id: NodeId::from(id),
            kind: "transform".into(),
            join: JoinPolicy::All,
            loop_budget: None,
            retry: RetryPolicy::default(),
            caps: 0,
            cfg: serde_json::json!({}),
        }
    }
}
