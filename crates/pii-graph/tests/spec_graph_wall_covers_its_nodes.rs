//! Spec 290: the tracked `spec` graph's wall limit covers the coders it runs. A run whose coder outlives the default
//! `max_wall_ms` (600 000 ms) fails at `verify` with the agent's work complete, so `[limits]` must carry a wall at least
//! the sum of every coder's `timeout_ms` in the expanded graph (the `docs` coder is inlined). The same relation holds for
//! every other tracked graph with a coder, unless it is listed in `CALLED_ONLY` with the reason it is exempt.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::fragment::{load_host_with_graphs, sha256_hex};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphLimits, GraphSpec};

/// Graphs with a coder that are exempt from the relation, by file name, each with the reason it is only meant to be
/// called. Nothing else is exempt: a failing graph not listed here is a finding.
const CALLED_ONLY: &[(&str, &str)] = &[(
    "docs.toml",
    "spec 272: the docs step is a `call` inlined into `spec`, which holds the run's wall limit",
)];

/// Stand-in for the coder kind: ports `ok` and `err`, and it never runs.
struct StubCoder;

impl NodeFactory for StubCoder {
    fn kind(&self) -> &'static str {
        "coder"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(Never))
    }
}

/// Stand-in for the judge kind: ports `ok` and `fail`, and it never runs.
struct StubJudge;

impl NodeFactory for StubJudge {
    fn kind(&self) -> &'static str {
        "judge"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("fail")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(Never))
    }
}

struct Never;

impl GraphNode for Never {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        panic!("the graph is only loaded here, never run");
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(StubCoder));
    reg.register(Arc::new(StubJudge));
    reg
}

/// The repo's `graphs/` directory: where `spec.toml` and the `docs` graph it calls live.
fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn read_graph(name: &str) -> String {
    std::fs::read_to_string(graphs_dir().join(name)).unwrap()
}

/// Loads a graph text the way the daemon loads a host: `call` nodes inlined from `graphs/`, `attach` from its fragments.
fn load(src: &str) -> GraphSpec {
    load_host_with_graphs(
        src,
        &graphs_dir().join("fragments"),
        &[graphs_dir()],
        &registry(),
    )
    .unwrap()
    .spec
}

/// The tracked `spec.toml`, loaded.
fn tracked_spec() -> GraphSpec {
    load(&read_graph("spec.toml"))
}

/// The sum of every `coder` node's `timeout_ms` in a loaded, expanded graph: the most the graph's coders may spend.
fn coder_timeout_sum(spec: &GraphSpec) -> u64 {
    spec.nodes
        .iter()
        .filter(|n| n.kind == "coder")
        .map(|n| {
            n.cfg["timeout_ms"]
                .as_u64()
                .expect("a coder node has an integer timeout_ms")
        })
        .sum()
}

/// Why the graph's wall limit does not cover its coders, or `None` when it does.
fn wall_gap(spec: &GraphSpec) -> Option<String> {
    let sum = coder_timeout_sum(spec);
    let wall = spec.limits.max_wall_ms;
    (wall < sum).then(|| format!("max_wall_ms {wall} is below the coders' timeout_ms sum {sum}"))
}

/// Whether the graph keeps the default wall limit (no `max_wall_ms` written).
fn is_default_wall(spec: &GraphSpec) -> bool {
    spec.limits.max_wall_ms == GraphLimits::default().max_wall_ms
}

/// `src` without its `max_wall_ms` line, with `ms` written in its place under `[limits]`.
fn with_wall(src: &str, ms: Option<u64>) -> String {
    let mut kept = String::new();
    for line in src.lines().filter(|l| !l.starts_with("max_wall_ms")) {
        kept.push_str(line);
        kept.push('\n');
    }
    match ms {
        None => kept,
        // The wall line goes right after the spend limit, whatever its value: this used to hard-code 1000000, so
        // changing the spend policy (4.00 USD, 2026-11-06) silently turned the control into a no-op that failed.
        Some(ms) => {
            let mut out = String::new();
            for line in kept.lines() {
                out.push_str(line);
                out.push('\n');
                if line.starts_with("max_micro_usd = ") {
                    out.push_str(&format!("max_wall_ms = {ms}\n"));
                }
            }
            out
        }
    }
}

/// Every `.toml` file under `dir`, recursively, as (file name, text), in path order.
fn graph_files(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            out.extend(graph_files(&path));
        } else if path.extension().is_some_and(|x| x == "toml") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

/// Whether the graph text declares a `coder` node, read from its TOML (no load needed to decide).
fn has_coder(src: &str) -> bool {
    let doc: toml::Table = toml::from_str(src).unwrap();
    doc.get("node")
        .and_then(|n| n.as_array())
        .is_some_and(|nodes| {
            nodes
                .iter()
                .any(|n| n.get("kind").and_then(|k| k.as_str()) == Some("coder"))
        })
}

/// The names of the graphs that carry a coder whose wall limit does not cover it, excluding `CALLED_ONLY`.
fn wall_failures(graphs: &[(String, String)]) -> Vec<String> {
    graphs
        .iter()
        .filter(|(name, src)| {
            has_coder(src)
                && !CALLED_ONLY.iter().any(|(exempt, _)| exempt == name)
                && wall_gap(&load(src)).is_some()
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[test]
fn spec_graph_wall_covers_its_coders() {
    // (a): the run's wall limit is at least what its coders may spend (the `code` node and the inlined `docs` coder).
    let spec = tracked_spec();
    assert_eq!(wall_gap(&spec), None, "graphs/spec.toml [limits]");
}

#[test]
fn spec_graph_wall_is_not_the_default() {
    // (b): the default 600 000 ms is the bug this spec fixes: a coder past ten minutes fails the run at `verify`.
    assert!(!is_default_wall(&tracked_spec()));
}

#[test]
fn CONTROL_without_the_wall_line_the_spec_graph_fails_a_and_b() {
    // (c): the same two assertions, run on the tracked text with its `max_wall_ms` line removed.
    let spec = load(&with_wall(&read_graph("spec.toml"), None));
    assert!(wall_gap(&spec).is_some(), "the default wall must fail (a)");
    assert!(is_default_wall(&spec), "the default wall must fail (b)");
}

#[test]
fn CONTROL_the_wall_boundary_is_the_coder_sum() {
    // (d): one millisecond under the sum fails, and the sum itself passes.
    let sum = coder_timeout_sum(&load(&with_wall(&read_graph("spec.toml"), Some(0))));
    assert!(sum > 0, "the tracked graph has coders");
    let under = load(&with_wall(&read_graph("spec.toml"), Some(sum - 1)));
    assert!(wall_gap(&under).is_some(), "sum - 1 must fail");
    let at = load(&with_wall(&read_graph("spec.toml"), Some(sum)));
    assert_eq!(wall_gap(&at), None, "sum must pass");
}

#[test]
fn CONTROL_docs_toml_is_the_digest_spec_toml_pins() {
    // (e): this change did not edit the called graph. The pin is read from `spec.toml`, not hard-coded here.
    //
    // The pin is the one belonging to the `docs` CALL, not the first `sha256` in the file: spec 385 gave
    // spec.toml a methodology route, so two other pins now precede it and this control compared the
    // docs file against the sdd digest.
    let pin = read_graph("spec.toml")
        .lines()
        .scan(false, |after_docs, line| {
            if line.trim() == "graph = \"docs\"" {
                *after_docs = true;
            }
            Some((*after_docs, line))
        })
        .filter(|(after, _)| *after)
        .find_map(|(_, l)| l.trim().strip_prefix("sha256 = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("spec.toml pins the docs graph by sha256")
        .to_string();
    assert_eq!(sha256_hex(read_graph("docs.toml").as_bytes()), pin);
}

#[test]
fn every_tracked_coder_graph_covers_its_coders() {
    // R3: every tracked graph with a coder that runs alone. A failing name is listed in the message.
    let failures = wall_failures(&graph_files(&graphs_dir()));
    assert!(
        failures.is_empty(),
        "graphs whose max_wall_ms is below their coders' timeout sum (findings, not exceptions): {failures:?}"
    );
}

#[test]
fn CONTROL_a_called_only_exception_is_by_name_not_by_content() {
    // R3 control: `docs.toml` passes only under its exempt name. Under any other name the same text fails.
    let docs = read_graph("docs.toml");
    assert!(has_coder(&docs), "docs.toml carries a coder");
    assert_eq!(
        wall_failures(&[("docs.toml".into(), docs.clone())]),
        Vec::<String>::new()
    );
    assert_eq!(
        wall_failures(&[("docs-copy.toml".into(), docs)]),
        vec!["docs-copy.toml".to_string()]
    );
}
