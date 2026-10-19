//! Spec 212: graph fragments — a skill or hook graph written once as a file
//! under `graphs/fragments/`, attached by name to a host graph, and woven in
//! at load time.
//!
//! A host declares `[[attach]]` tables. Each names a fragment file, the
//! surface it attaches to, and the sha256 `pin` the fragment must still match.
//! [`weave`] inlines the fragment's nodes as `<attach>.<node>`, stamps each
//! with `woven-from: <fragment>#<node>` and `hook: <attach>`, and splices them
//! into the host at the named surface. Inlining happens once, at load time: a
//! fragment is never generated or resolved during a run.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use pii_abi::{NodeId, PiiError, PortName};
use sha2::{Digest, Sha256};
use toml::Table;

use crate::node::NodeRegistry;
use crate::spec::{Edge, EdgeKind, GraphSpec, NodeSpec};
use crate::toml_load::load_toml_with_registry;

/// Surfaces an attach may name.
const SURFACES: &[&str] = &["run.start", "run.end", "node.before", "node.after"];
/// Surfaces that run before the work they guard: only a blocking fragment may attach here.
const PRE_SURFACES: &[&str] = &["run.start", "node.before"];
/// Top-level fragment keys that would give it a timer, a store or a budget of its own.
const FORBIDDEN_KEYS: &[&str] = &["timer", "store", "budget", "limits", "schedule", "cron"];
/// Node config keys that would give one fragment node a timer, a store or a budget.
const FORBIDDEN_NODE_KEYS: &[&str] = &["budget", "store", "limits", "cron", "timer_ms", "in_ms"];

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
}

/// A fragment file that passed its contract checks.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The fragment's own nodes and edges, with local ids.
    pub spec: GraphSpec,
    /// Whether the fragment may attach at a pre surface.
    pub may_block: bool,
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
    Ok(Woven { spec, sequence })
}

/// The run.start chain sits in front of the host's entry.
fn splice_run_start(
    spec: &mut GraphSpec,
    loaded: &[Loaded],
    host: &[NodeId],
    reg: &NodeRegistry,
    sequence: &mut Vec<Inlined>,
) -> Result<(), PiiError> {
    let starts = on_surface(loaded, "run.start", None, host)?;
    let pieces = starts
        .iter()
        .map(|l| piece(l, &l.attach.id, "run.start", reg))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(head) = pieces.first().map(|p| p.entry.clone()) else {
        return Ok(());
    };
    let tail = [Link {
        to: spec.entry.clone(),
        kind: EdgeKind::Data,
    }];
    spec.entry = head;
    splice(spec, pieces, &tail, sequence);
    Ok(())
}

/// node.before and node.after, per host node in declaration order.
fn splice_node_surfaces(
    spec: &mut GraphSpec,
    loaded: &[Loaded],
    host: &[NodeId],
    reg: &NodeRegistry,
    sequence: &mut Vec<Inlined>,
) -> Result<(), PiiError> {
    for target in host {
        let befores = on_surface(loaded, "node.before", Some(target), host)?;
        splice_before(spec, target, &befores, host, reg, sequence)?;
        let afters = on_surface(loaded, "node.after", Some(target), host)?;
        splice_after(spec, target, &afters, host, reg, sequence)?;
    }
    Ok(())
}

/// Inbound edges to `target` reach the chain first; the chain's exits lead to the node.
fn splice_before(
    spec: &mut GraphSpec,
    target: &NodeId,
    attaches: &[&Loaded],
    host: &[NodeId],
    reg: &NodeRegistry,
    sequence: &mut Vec<Inlined>,
) -> Result<(), PiiError> {
    if attaches.is_empty() {
        return Ok(());
    }
    let pieces = node_pieces(attaches, "node.before", target, host, reg)?;
    let Some(head) = pieces.first().map(|p| p.entry.clone()) else {
        return Ok(());
    };
    for e in &mut spec.edges {
        if e.to == *target {
            e.to = head.clone();
        }
    }
    if spec.entry == *target {
        spec.entry = head;
    }
    let tail = [Link {
        to: target.clone(),
        kind: EdgeKind::Data,
    }];
    splice(spec, pieces, &tail, sequence);
    Ok(())
}

/// The node's ok edges are lifted out and re-hung after the chain.
fn splice_after(
    spec: &mut GraphSpec,
    target: &NodeId,
    attaches: &[&Loaded],
    host: &[NodeId],
    reg: &NodeRegistry,
    sequence: &mut Vec<Inlined>,
) -> Result<(), PiiError> {
    if attaches.is_empty() {
        return Ok(());
    }
    let pieces = node_pieces(attaches, "node.after", target, host, reg)?;
    let Some(head) = pieces.first().map(|p| p.entry.clone()) else {
        return Ok(());
    };
    let mut tail = Vec::new();
    spec.edges.retain(|e| {
        let leaving = e.from == *target && e.port.as_str() == "ok";
        if leaving {
            tail.push(Link {
                to: e.to.clone(),
                kind: e.kind,
            });
        }
        !leaving
    });
    spec.edges.push(Edge {
        from: target.clone(),
        port: PortName::from("ok"),
        to: head,
        kind: EdgeKind::Data,
    });
    splice(spec, pieces, &tail, sequence);
    Ok(())
}

/// The run.end chain follows every host node that has no ok successor.
fn splice_run_end(
    spec: &mut GraphSpec,
    loaded: &[Loaded],
    host: &[NodeId],
    reg: &NodeRegistry,
    sequence: &mut Vec<Inlined>,
) -> Result<(), PiiError> {
    let ends = on_surface(loaded, "run.end", None, host)?;
    if ends.is_empty() {
        return Ok(());
    }
    let sinks: Vec<NodeId> = host
        .iter()
        .filter(|id| {
            !spec
                .edges
                .iter()
                .any(|e| e.from == **id && e.port.as_str() == "ok")
        })
        .cloned()
        .collect();
    if sinks.is_empty() {
        return Err(PiiError::Invalid(
            "fragment.surface.unreachable: run.end names no host node to follow".into(),
        ));
    }
    let pieces = ends
        .iter()
        .map(|l| piece(l, &l.attach.id, "run.end", reg))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(head) = pieces.first().map(|p| p.entry.clone()) else {
        return Ok(());
    };
    for sink in sinks {
        spec.edges.push(Edge {
            from: sink,
            port: PortName::from("ok"),
            to: head.clone(),
            kind: EdgeKind::Order,
        });
    }
    splice(spec, pieces, &[], sequence);
    Ok(())
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
struct Attach {
    id: String,
    fragment: String,
    at: String,
    on: Vec<String>,
    order: i64,
    pin: String,
}

/// An attach whose fragment has been read, pin-checked and contract-checked.
struct Loaded {
    attach: Attach,
    frag: Fragment,
}

/// One inlined copy of a fragment, ready to splice.
struct Piece {
    attach: String,
    surface: String,
    order: i64,
    nodes: Vec<NodeSpec>,
    edges: Vec<Edge>,
    entry: NodeId,
    /// Nodes whose `ok` port is not wired inside the fragment: they feed what follows.
    exits: Vec<NodeId>,
}

/// Where a chain's last exits lead: always through the `ok` port.
#[derive(Clone)]
struct Link {
    to: NodeId,
    kind: EdgeKind,
}

fn parse_attaches(doc: &Table) -> Result<Vec<Attach>, PiiError> {
    let items = match doc.get("attach") {
        None => return Ok(Vec::new()),
        Some(toml::Value::Array(items)) => items,
        Some(_) => {
            return Err(PiiError::Invalid(
                "'attach' must be an array of tables".into(),
            ));
        }
    };
    let mut out: Vec<Attach> = Vec::new();
    for item in items {
        let t = item
            .as_table()
            .ok_or_else(|| PiiError::Invalid("each [[attach]] must be a table".into()))?;
        let id = text(t, "id")?;
        if out.iter().any(|a| a.id == id) {
            return Err(PiiError::Invalid(format!("duplicate attach id '{id}'")));
        }
        let at = text(t, "at")?;
        if !SURFACES.contains(&at.as_str()) {
            return Err(PiiError::Invalid(format!(
                "fragment.surface.unknown: attach '{id}' names surface '{at}'"
            )));
        }
        let on = names(t, &id)?;
        if at.starts_with("node.") && on.is_empty() {
            return Err(PiiError::Invalid(format!(
                "attach '{id}' at {at} needs on = [node ids] or [\"*\"]"
            )));
        }
        if !at.starts_with("node.") && !on.is_empty() {
            return Err(PiiError::Invalid(format!(
                "attach '{id}' at {at} takes no on list"
            )));
        }
        let pin = text(t, "pin").map_err(|_| {
            PiiError::Invalid(format!(
                "fragment.pin.missing: attach '{id}' carries no pin; a fragment is inlined only by its sha256"
            ))
        })?;
        let order = match t.get("order") {
            None => 0,
            Some(v) => v.as_integer().ok_or_else(|| {
                PiiError::Invalid(format!("attach '{id}' order must be an integer"))
            })?,
        };
        out.push(Attach {
            fragment: text(t, "fragment")?,
            id,
            at,
            on,
            order,
            pin: pin.to_ascii_lowercase(),
        });
    }
    Ok(out)
}

fn text(t: &Table, key: &str) -> Result<String, PiiError> {
    t.get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PiiError::Invalid(format!("attach missing string '{key}'")))
}

fn names(t: &Table, id: &str) -> Result<Vec<String>, PiiError> {
    match t.get("on") {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    PiiError::Invalid(format!("attach '{id}': 'on' entries must be strings"))
                })
            })
            .collect(),
        Some(_) => Err(PiiError::Invalid(format!(
            "attach '{id}': 'on' must be an array"
        ))),
    }
}

/// The host nodes an attach applies to: `["*"]` is every node, else the named ones.
fn targets(a: &Attach, host: &[NodeId]) -> Result<Vec<NodeId>, PiiError> {
    if a.on.len() == 1 && a.on.first().map(String::as_str) == Some("*") {
        return Ok(host.to_vec());
    }
    a.on.iter()
        .map(|name| {
            let id = NodeId::from(name.as_str());
            if host.contains(&id) {
                Ok(id)
            } else {
                Err(PiiError::Invalid(format!(
                    "fragment.target.unknown: attach '{}' names node '{name}', which the host does not declare",
                    a.id
                )))
            }
        })
        .collect()
}

/// The attaches on `surface` (and, for node surfaces, on `target`), in run order: `order`, then id.
fn on_surface<'a>(
    loaded: &'a [Loaded],
    surface: &str,
    target: Option<&NodeId>,
    host: &[NodeId],
) -> Result<Vec<&'a Loaded>, PiiError> {
    let mut out = Vec::new();
    for l in loaded {
        if l.attach.at != surface {
            continue;
        }
        match target {
            None => out.push(l),
            Some(t) => {
                if targets(&l.attach, host)?.contains(t) {
                    out.push(l);
                }
            }
        }
    }
    out.sort_by(|a, b| (a.attach.order, &a.attach.id).cmp(&(b.attach.order, &b.attach.id)));
    Ok(out)
}

/// The pieces for one node surface target, namespaced by target when an attach spans several.
fn node_pieces(
    attaches: &[&Loaded],
    surface: &str,
    target: &NodeId,
    host: &[NodeId],
    reg: &NodeRegistry,
) -> Result<Vec<Piece>, PiiError> {
    let mut pieces = Vec::with_capacity(attaches.len());
    for l in attaches {
        let count = targets(&l.attach, host)?.len();
        let prefix = if count == 1 {
            l.attach.id.clone()
        } else {
            format!("{}@{target}", l.attach.id)
        };
        pieces.push(piece(l, &prefix, surface, reg)?);
    }
    Ok(pieces)
}

fn fragment_path(a: &Attach, dir: &Path) -> Result<PathBuf, PiiError> {
    let plain = Path::new(a.fragment.as_str())
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if a.fragment.is_empty() || !plain {
        return Err(PiiError::Invalid(format!(
            "fragment.path.invalid: attach '{}' names '{}'",
            a.id, a.fragment
        )));
    }
    Ok(dir.join(format!("{}.toml", a.fragment)))
}

/// Reads one fragment, refuses it on a pin drift before anything else, then
/// runs its contract checks and the block rule for the attach's surface.
fn load_fragment_file(a: &Attach, dir: &Path, reg: &NodeRegistry) -> Result<Fragment, PiiError> {
    let path = fragment_path(a, dir)?;
    let bytes = std::fs::read(&path)
        .map_err(|e| PiiError::Invalid(format!("fragment {}: read: {e}", path.display())))?;
    let actual = sha256_hex(&bytes);
    if actual != a.pin {
        return Err(PiiError::Invalid(format!(
            "fragment.pin.drift: {}: expected {}, actual {actual}",
            path.display(),
            a.pin
        )));
    }
    let src = std::str::from_utf8(&bytes)
        .map_err(|_| PiiError::Invalid(format!("fragment {}: not utf-8", path.display())))?;
    let frag = check_fragment(src, reg)?;
    if PRE_SURFACES.contains(&a.at.as_str()) && !frag.may_block {
        return Err(PiiError::Invalid(format!(
            "fragment.block.invalid: {} is may_block = false and cannot attach at {}",
            a.fragment, a.at
        )));
    }
    Ok(frag)
}

/// Namespaces one fragment for one attach and works out its exits.
fn piece(l: &Loaded, prefix: &str, surface: &str, reg: &NodeRegistry) -> Result<Piece, PiiError> {
    let local = &l.frag.spec;
    let ns = |id: &NodeId| NodeId::from(format!("{prefix}.{id}").as_str());

    let mut nodes = Vec::with_capacity(local.nodes.len());
    for n in &local.nodes {
        let mut cfg = n.cfg.clone();
        if let Some(obj) = cfg.as_object_mut() {
            obj.insert(
                "woven-from".to_owned(),
                serde_json::json!(format!("{}#{}", l.attach.fragment, n.id)),
            );
            obj.insert("hook".to_owned(), serde_json::json!(l.attach.id));
        }
        nodes.push(NodeSpec {
            id: ns(&n.id),
            kind: n.kind.clone(),
            join: n.join.clone(),
            loop_budget: n.loop_budget,
            retry: n.retry,
            caps: n.caps,
            cfg,
        });
    }
    let edges: Vec<Edge> = local
        .edges
        .iter()
        .map(|e| Edge {
            from: ns(&e.from),
            port: e.port.clone(),
            to: ns(&e.to),
            kind: e.kind,
        })
        .collect();

    let mut exits = Vec::new();
    for n in &nodes {
        let has_ok = reg
            .factory(&n.kind)?
            .ports(&n.cfg)?
            .iter()
            .any(|p| p.as_str() == "ok");
        let wired = edges
            .iter()
            .any(|e| e.from == n.id && e.port.as_str() == "ok");
        if has_ok && !wired {
            exits.push(n.id.clone());
        }
    }

    Ok(Piece {
        attach: l.attach.id.clone(),
        surface: surface.to_owned(),
        order: l.attach.order,
        nodes,
        edges,
        entry: ns(&local.entry),
        exits,
    })
}

/// Adds the pieces to the host in order: each piece's exits feed the next
/// piece's entry, and the last piece's exits feed `tail`.
fn splice(spec: &mut GraphSpec, pieces: Vec<Piece>, tail: &[Link], sequence: &mut Vec<Inlined>) {
    let entries: Vec<NodeId> = pieces.iter().map(|p| p.entry.clone()).collect();
    for (i, piece) in pieces.into_iter().enumerate() {
        let next: Vec<Link> = entries.get(i + 1).map_or_else(
            || tail.to_vec(),
            |entry| {
                vec![Link {
                    to: entry.clone(),
                    kind: EdgeKind::Data,
                }]
            },
        );
        for exit in &piece.exits {
            for link in &next {
                spec.edges.push(Edge {
                    from: exit.clone(),
                    port: PortName::from("ok"),
                    to: link.to.clone(),
                    kind: link.kind,
                });
            }
        }
        for n in &piece.nodes {
            sequence.push(Inlined {
                surface: piece.surface.clone(),
                attach: piece.attach.clone(),
                order: piece.order,
                node: n.id.clone(),
                origin: origin_of(n),
            });
        }
        spec.nodes.extend(piece.nodes);
        spec.edges.extend(piece.edges);
    }
}

fn origin_of(n: &NodeSpec) -> String {
    n.cfg
        .get("woven-from")
        .and_then(serde_json::Value::as_str)
        .map_or_else(String::new, str::to_owned)
}

/// Refuses a woven graph whose namespaced ids collide with each other or with the host.
fn check_unique(spec: &GraphSpec) -> Result<(), PiiError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for n in &spec.nodes {
        if !seen.insert(n.id.as_str()) {
            return Err(PiiError::Invalid(format!(
                "duplicate node id '{}' after weaving",
                n.id
            )));
        }
    }
    Ok(())
}

fn autonomy(what: &str) -> PiiError {
    PiiError::Invalid(format!(
        "fragment.autonomy.forbidden: {what} gives a fragment its own timer, store or budget; a fragment is work, never a scheduler"
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

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
