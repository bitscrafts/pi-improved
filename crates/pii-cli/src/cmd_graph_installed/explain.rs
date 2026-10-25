//! `pii graph explain <name>`: rendered from the installed file's own fields, never paraphrased.

use std::collections::BTreeMap;
use std::process::ExitCode;

use pii_abi::PortName;
use pii_core::CapKind;
use pii_graph::fragment::{CallRecord, Inlined, sha256_hex};
use pii_graph::{GraphSpec, NodeRegistry, NodeSpec};
use serde_json::Value;

use super::edit::drift_verdict;
use super::{
    Opts, check, effect_of, error, fragments_dir, installed_dirs, one_name, registry, repo_dir,
    trigger,
};

/// The node ids where two specs disagree: changed, only in the installed copy, only in the repo's.
pub(super) fn node_diff<'a>(
    installed: &'a GraphSpec,
    repo: &'a GraphSpec,
) -> (Vec<&'a str>, Vec<&'a str>, Vec<&'a str>) {
    let i: BTreeMap<&str, &NodeSpec> = installed.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let r: BTreeMap<&str, &NodeSpec> = repo.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let changed = i
        .iter()
        .filter(|(id, n)| r.get(*id).is_some_and(|rn| rn != *n))
        .map(|(id, _)| *id)
        .collect();
    let only_i = i
        .keys()
        .filter(|id| !r.contains_key(*id))
        .copied()
        .collect();
    let only_r = r
        .keys()
        .filter(|id| !i.contains_key(*id))
        .copied()
        .collect();
    (changed, only_i, only_r)
}

/// `pii graph explain <name>`: rendered from the installed file's own fields, never paraphrased.
pub(super) fn run(o: &Opts) -> ExitCode {
    let name = match one_name(o) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let installed = match installed_dirs(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let Some(path) = pii_server::find_graph(&installed, &name) else {
        return error(pii_server::graph_not_found(&name, &installed));
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return error(format!("{name} is not installed: {e}")),
    };
    let dir = path.parent().unwrap_or(repo.as_path());
    let reg = registry();
    let sha = sha256_hex(src.as_bytes());
    let checked = match check(&src, &fragments_dir(dir, &repo), &[dir.to_path_buf()], &reg) {
        Ok(c) => c,
        Err(e) => return error(format!("{name} does not load: {e}")),
    };
    let spec = &checked.spec;

    println!("graph {name}");
    println!("file {}", path.display());
    println!("sha256 {sha}");
    println!("TRIGGER {}", trigger(spec));
    print_nodes(spec, &reg);
    print_effects(spec);
    print_capabilities(spec);
    print_woven(&checked.woven);
    println!("DRIFT {}", drift_verdict(&name, &repo, &sha, spec, &reg));
    print_calls(&checked.calls);
    ExitCode::SUCCESS
}

/// One `call` line per call the load expanded (spec 269): the graph, its verified digest's first 12 hex, the caps,
/// the sub-graph's `needs` (`none` when it declares none), its exit, and the call that holds it when nested.
fn print_calls(calls: &[CallRecord]) {
    for c in calls {
        let needs = c
            .needs
            .as_ref()
            .map_or_else(|| String::from("none"), |keys| keys.join(","));
        let parent = c
            .parent
            .as_ref()
            .map_or_else(String::new, |p| format!(" (in {p})"));
        let short = c.sha256.get(..12).unwrap_or(&c.sha256);
        println!(
            "call {} -> graph {} sha256 {short} caps [{}] needs [{needs}] exit {}{parent}",
            c.call,
            c.graph,
            c.caps.join(","),
            c.exit
        );
    }
}

/// The nodes in dependency order, each with its kind, its declared ports and its outgoing edges.
fn print_nodes(spec: &GraphSpec, reg: &NodeRegistry) {
    println!("NODES in dependency order");
    for id in pii_graph::render::topo_order(spec) {
        let Some(node) = spec.node(&id) else { continue };
        let ports = reg
            .factory(&node.kind)
            .ok()
            .and_then(|f| f.ports(&node.cfg).ok())
            .map_or_else(
                || String::from("?"),
                |ps| {
                    ps.iter()
                        .map(PortName::as_str)
                        .collect::<Vec<&str>>()
                        .join(",")
                },
            );
        let edges: Vec<String> = spec
            .outbound(&id)
            .into_iter()
            .filter_map(|i| spec.edges.get(i))
            .map(|e| format!("{}->{}", e.port.as_str(), e.to.as_str()))
            .collect();
        println!(
            "  {id}  kind {}  ports {ports}  {}",
            node.kind,
            edges.join(" ")
        );
    }
}

/// What the graph writes or touches, grouped by effect class, with the node ids that do it.
fn print_effects(spec: &GraphSpec) {
    println!("EFFECTS");
    let mut effects: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &spec.nodes {
        if let Some(label) = effect_of(&node.kind) {
            effects.entry(label).or_default().push(node.id.as_str());
        }
    }
    if effects.is_empty() {
        println!("  none");
    }
    for (label, ids) in &effects {
        println!("  {label}  {}", ids.join(", "));
    }
}

/// The graph's capability grant, then the channel, agent and model each node names.
fn print_capabilities(spec: &GraphSpec) {
    println!("CAPABILITIES");
    let graph_caps: Vec<&str> = CapKind::from_bits_truncate(spec.caps)
        .iter_names()
        .map(|(n, _)| n)
        .collect();
    if graph_caps.is_empty() {
        println!("  graph none");
    } else {
        println!("  graph {}", graph_caps.join(" | "));
    }
    for node in &spec.nodes {
        for key in ["channel", "agent", "model"] {
            if let Some(v) = node.cfg.get(key).and_then(Value::as_str) {
                println!("  {key} {v} (node {})", node.id);
            }
        }
    }
}

/// The fragment nodes woven into the graph, with the fragment and node each came from.
fn print_woven(woven: &[Inlined]) {
    println!("WOVEN");
    if woven.is_empty() {
        println!("  none");
    }
    for w in woven {
        println!(
            "  {} {} origin {} hook {} order {}",
            w.surface, w.node, w.origin, w.attach, w.order
        );
    }
}
