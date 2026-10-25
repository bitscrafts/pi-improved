//! The weave: fragment nodes spliced into a host graph at its surfaces (spec 212).

use std::path::{Path, PathBuf};

use pii_abi::{NodeId, PiiError};
use toml::Table;

use super::call_record::CallRecord;
use crate::node::NodeRegistry;
use crate::spec::GraphSpec;
use crate::toml_load::load_toml_with_registry;

use super::call::expand_host;
use super::check::Loaded;
use super::splice::{
    check_unique, load_fragment_file, parse_attaches, splice_node_surfaces, splice_run_end,
    splice_run_start, targets,
};

/// One inlined node, with the surface and attach it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inlined {
    /// The surface the attach named: `run.start`, `run.end`, `node.before`, `node.after`.
    pub surface: String,
    /// The attach id the node came from.
    pub attach: String,
    /// The attach's declared `order`.
    pub order: i64,
    /// The namespaced node id, `<attach>.<node>`.
    pub node: NodeId,
    /// The provenance stamp, `<fragment>#<node>`.
    pub origin: String,
}

/// A host graph with every attach woven in.
#[derive(Debug, Clone)]
pub struct Woven {
    /// The woven graph, ready to validate and run.
    pub spec: GraphSpec,
    /// Every inlined node, in the order the weave applied them.
    pub sequence: Vec<Inlined>,
    /// The records of every call the load expanded (spec 269), in expansion order.
    pub calls: Vec<CallRecord>,
}

/// Weaves the `[[attach]]` tables of `host_src` into the host graph.
///
/// Fragment files are read from `fragments_dir`. A node surface with several
/// targets gives each target its own copy, namespaced `<attach>@<node>.<node>`;
/// a single target keeps the plain `<attach>.<node>` form.
///
/// # Errors
/// Returns [`PiiError::Invalid`] naming the rule code (`fragment.*`) on a pin
/// drift, a missing pin, a nesting, autonomy, or block violation, an unknown
/// surface or target, or any structural error in the host or a fragment.
pub fn weave(host_src: &str, fragments_dir: &Path, reg: &NodeRegistry) -> Result<Woven, PiiError> {
    let mut doc: Table = toml::from_str(host_src)
        .map_err(|e| PiiError::Invalid(format!("graph toml parse: {e}")))?;
    let attaches = parse_attaches(&doc)?;
    doc.remove("attach");
    let base = toml::to_string(&doc).map_err(|e| PiiError::Invalid(format!("graph toml: {e}")))?;
    let mut spec = load_toml_with_registry(&base, reg)?;
    let host_ids: Vec<NodeId> = spec.nodes.iter().map(|n| n.id.clone()).collect();

    let mut loaded = Vec::with_capacity(attaches.len());
    for attach in attaches {
        let frag = load_fragment_file(&attach, fragments_dir, reg)?;
        loaded.push(Loaded { attach, frag });
    }
    for l in &loaded {
        if l.attach.at.starts_with("node.") {
            targets(&l.attach, &host_ids)?;
        }
    }

    let mut sequence = Vec::new();
    splice_run_start(&mut spec, &loaded, &host_ids, reg, &mut sequence)?;
    splice_node_surfaces(&mut spec, &loaded, &host_ids, reg, &mut sequence)?;
    splice_run_end(&mut spec, &loaded, &host_ids, reg, &mut sequence)?;
    check_unique(&spec)?;
    Ok(Woven {
        spec,
        sequence,
        calls: Vec::new(),
    })
}

/// Loads a host graph for running or checking (spec 219).
///
/// A graph that attaches fragments is woven, so its pins are verified and its fragments inlined. Any other graph
/// loads exactly as [`load_toml_with_registry`] loads it. Every caller that runs or checks a graph goes through
/// here, so "check passed" and "run started" cannot disagree. A graph with a `call` node has no graph directory
/// here, so it is refused; use [`load_host_with_graphs`].
///
/// # Errors
/// The errors of [`load_host_with_graphs`] with no graph directory.
pub fn load_host(
    host_src: &str,
    fragments_dir: &Path,
    reg: &NodeRegistry,
) -> Result<Woven, PiiError> {
    load_host_with_graphs(host_src, fragments_dir, &[], reg)
}

/// Loads a host graph, inlining its `call` nodes from `graph_dirs` first (spec 255).
///
/// The calls are expanded on the host table before anything else, so `pii graph check`, a run start and a resume
/// all load the same expanded graph. A graph with no call is loaded from its own text, as [`load_host`] did.
///
/// # Errors
/// The errors of [`expand_host`] for a bad call, and those of [`load_host`]'s other paths otherwise.
pub fn load_host_with_graphs(
    host_src: &str,
    fragments_dir: &Path,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
) -> Result<Woven, PiiError> {
    let expanded = expand_host(host_src, graph_dirs, reg)?;
    let (expanded, calls) = expanded;
    let host_src = expanded.as_deref().unwrap_or(host_src);
    let attaches = toml::from_str::<Table>(host_src).is_ok_and(|t| t.contains_key("attach"));
    if attaches {
        return weave(host_src, fragments_dir, reg).map(|w| Woven { calls, ..w });
    }
    Ok(Woven {
        spec: load_toml_with_registry(host_src, reg)?,
        sequence: Vec::new(),
        calls,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::fragment::check::Attach;
    use crate::fragment::sha256_hex;

    #[test]
    fn sha256_matches_the_known_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn targets_reject_an_undeclared_node() {
        let a = Attach {
            id: "g".into(),
            fragment: "f".into(),
            at: "node.before".into(),
            on: vec!["nope".into()],
            order: 0,
            pin: String::new(),
        };
        let err = targets(&a, &[NodeId::from("build")]).unwrap_err();
        assert!(err.to_string().contains("fragment.target.unknown"));
    }
}
