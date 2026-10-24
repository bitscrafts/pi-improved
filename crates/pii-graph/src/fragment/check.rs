//! Fragment checks: what a fragment file may be (spec 212).

use pii_abi::{NodeId, PiiError};
use sha2::{Digest, Sha256};
use toml::Table;

use crate::node::NodeRegistry;
use crate::spec::{Edge, EdgeKind, GraphSpec, NodeSpec};
use crate::toml_load::load_toml_with_registry;

use super::shared::{FORBIDDEN_KEYS, FORBIDDEN_NODE_KEYS};

/// A fragment file that passed its contract checks.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The fragment's own nodes and edges, with local ids.
    pub spec: GraphSpec,
    /// Whether the fragment may attach at a pre surface.
    pub may_block: bool,
}

/// Whether `src` is a fragment file (it declares a `[fragment]` table) rather than a host graph.
#[must_use]
pub fn declares_fragment(src: &str) -> bool {
    toml::from_str::<Table>(src).is_ok_and(|t| t.contains_key("fragment"))
}

/// Checks a fragment file on its own: its header, its contract, and that it
/// loads as a graph. Used by `pii graph check` on a file under `fragments/`.
///
/// # Errors
/// Returns [`PiiError::Invalid`] naming the `fragment.*` rule that failed.
pub fn check_fragment(src: &str, reg: &NodeRegistry) -> Result<Fragment, PiiError> {
    let doc: Table =
        toml::from_str(src).map_err(|e| PiiError::Invalid(format!("fragment toml parse: {e}")))?;
    if doc.contains_key("attach") {
        return Err(PiiError::Invalid(
            "fragment.nesting.forbidden: a fragment may not attach a fragment (depth 1)".into(),
        ));
    }
    if super::call::has_call(&doc) {
        return Err(PiiError::Invalid(
            "fragment.nesting.forbidden: a fragment may not call a graph (spec 255)".into(),
        ));
    }
    let header = doc
        .get("fragment")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            PiiError::Invalid(
                "fragment.header.missing: a fragment declares a [fragment] table".into(),
            )
        })?;
    for key in ["name", "inputs", "outputs"] {
        if !header.contains_key(key) {
            return Err(PiiError::Invalid(format!(
                "fragment.header.missing: [fragment] lacks '{key}'"
            )));
        }
    }
    let may_block = header
        .get("may_block")
        .and_then(toml::Value::as_bool)
        .ok_or_else(|| {
            PiiError::Invalid(
                "fragment.header.missing: [fragment] lacks a boolean 'may_block'".into(),
            )
        })?;
    if let Some(key) = FORBIDDEN_KEYS
        .iter()
        .copied()
        .find(|k| doc.contains_key(*k))
    {
        return Err(autonomy(key));
    }
    let spec = load_toml_with_registry(src, reg)?;
    for n in &spec.nodes {
        let own_budget = FORBIDDEN_NODE_KEYS.iter().any(|k| n.cfg.get(*k).is_some());
        if n.kind == "timer" || own_budget {
            return Err(autonomy(&format!("node '{}'", n.id)));
        }
    }
    Ok(Fragment { spec, may_block })
}

/// The lowercase hex sha256 of `bytes`: the value a `pin` must carry.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// An attach as written in the host.
#[derive(Debug, Clone)]
pub(super) struct Attach {
    pub(super) id: String,
    pub(super) fragment: String,
    pub(super) at: String,
    pub(super) on: Vec<String>,
    pub(super) order: i64,
    pub(super) pin: String,
}

/// An attach whose fragment has been read, pin-checked and contract-checked.
pub(super) struct Loaded {
    pub(super) attach: Attach,
    pub(super) frag: Fragment,
}

/// One inlined copy of a fragment, ready to splice.
pub(super) struct Piece {
    pub(super) attach: String,
    pub(super) surface: String,
    pub(super) order: i64,
    pub(super) nodes: Vec<NodeSpec>,
    pub(super) edges: Vec<Edge>,
    pub(super) entry: NodeId,
    /// Nodes whose `ok` port is not wired inside the fragment: they feed what follows.
    pub(super) exits: Vec<NodeId>,
}

/// Where a chain's last exits lead: always through the `ok` port.
#[derive(Clone)]
pub(super) struct Link {
    pub(super) to: NodeId,
    pub(super) kind: EdgeKind,
}

fn autonomy(what: &str) -> PiiError {
    PiiError::Invalid(format!(
        "fragment.autonomy.forbidden: {what} gives a fragment its own timer, store or budget; a fragment is work, never a scheduler"
    ))
}
