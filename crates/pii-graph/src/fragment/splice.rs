//! The splice step: checked, loaded fragments inlined into the host graph (spec 212).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use pii_abi::{NodeId, PiiError, PortName};
use toml::Table;

use crate::spec::{Edge, EdgeKind, GraphSpec, NodeSpec};

use super::check::{Attach, Link, Loaded, Piece};
use super::check::{Fragment, check_fragment, sha256_hex};
use super::shared::{PRE_SURFACES, SURFACES};
use super::weave::Inlined;
use crate::node::NodeRegistry;
/// The run.start chain sits in front of the host's entry.
pub(super) fn splice_run_start(
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
pub(super) fn splice_node_surfaces(
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
pub(super) fn splice_run_end(
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
pub(super) fn parse_attaches(doc: &Table) -> Result<Vec<Attach>, PiiError> {
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
pub(super) fn targets(a: &Attach, host: &[NodeId]) -> Result<Vec<NodeId>, PiiError> {
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
pub(super) fn load_fragment_file(
    a: &Attach,
    dir: &Path,
    reg: &NodeRegistry,
) -> Result<Fragment, PiiError> {
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
pub(super) fn check_unique(spec: &GraphSpec) -> Result<(), PiiError> {
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
