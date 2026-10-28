//! Spec 306: every `call` has an entry counter, so no subgraph can be entered without bound. A call's head
//! `<call>__in` counts its entries; the (`max_entries`+1)-th is refused at dispatch and the run fails naming the
//! call. The default is 8. The tests run real loops and count real entries through a counting gate node.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, RunEvent, Terminal};
use pii_core::{Budget, CapKind, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
mod common;

use pii_graph::fragment;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::run;
use pii_store::Store;

/// The sub-graph every call inlines: two transforms, exit `b`.
const SUB: &str = r#"
name = "docs"
exit = "b"

[[node]]
id = "a"
kind = "transform"
value = "a"

[[node]]
id = "b"
kind = "transform"
value = "b"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"
"#;

/// The gate kind: it counts its own runs and takes `pass` once it has run `pass_at` times, `fail` before that.
struct GateFactory {
    hits: Arc<AtomicUsize>,
}

struct Gate {
    hits: Arc<AtomicUsize>,
    pass_at: usize,
}

impl GraphNode for Gate {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let n = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
        let port = if n >= self.pass_at { "pass" } else { "fail" };
        Ok(Outcome::Next {
            port: PortName::from(port),
            value: serde_json::json!(n),
        })
    }
}

impl NodeFactory for GateFactory {
    fn kind(&self) -> &'static str {
        "gate_n"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("pass"), PortName::from("fail")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let pass_at = cfg
            .get("pass_at")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| PiiError::Invalid("gate_n needs 'pass_at'".into()))?;
        Ok(Box::new(Gate {
            hits: Arc::clone(&self.hits),
            pass_at: usize::try_from(pass_at).unwrap_or(usize::MAX),
        }))
    }
}

/// The builtin registry plus a fresh `gate_n`, and the counter that gate reports into.
fn registry() -> (NodeRegistry, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(GateFactory {
        hits: Arc::clone(&hits),
    }));
    (reg, hits)
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

/// One `call` node of graph `graph` pinned to `sha`, with `extra` lines (its `max_entries`, or a bad value).
fn call_of(id: &str, graph: &str, sha: &str, extra: &str) -> String {
    format!(
        "[[node]]\nid = \"{id}\"\nkind = \"call\"\ngraph = \"{graph}\"\nsha256 = \"{sha}\"\ncaps = []\n{extra}\n"
    )
}

/// A call of `docs` with `extra` lines (its `max_entries`, or a bad value).
fn call_with(id: &str, extra: &str) -> String {
    call_of(id, "docs", &pin(SUB), extra)
}

/// A call of `docs` with an optional `max_entries`.
fn call(id: &str, max: Option<u32>) -> String {
    call_with(
        id,
        &max.map_or(String::new(), |m| format!("max_entries = {m}")),
    )
}

/// The host: `top (loop_budget, entry) -> <calls in order> -> gate`, where `gate` passes to `done` once it has run
/// `pass_at` times and fails back to `top`. `top` is the only node the loop re-enters, so it has one inbound edge.
fn looped(calls: &[String], chain: &[&str], budget: u32, pass_at: u32) -> String {
    let mut src = format!(
        "name = \"host\"\nentry = \"top\"\n\n[[node]]\nid = \"top\"\nkind = \"transform\"\nvalue = \"t\"\nloop_budget = {budget}\n\n"
    );
    for c in calls {
        src.push_str(c);
        src.push('\n');
    }
    let _ = write!(
        src,
        "[[node]]\nid = \"gate\"\nkind = \"gate_n\"\npass_at = {pass_at}\n\n\
         [[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n"
    );
    let mut path: Vec<&str> = vec!["top"];
    path.extend_from_slice(chain);
    path.push("gate");
    for pair in path.windows(2) {
        let _ = write!(
            src,
            "[[edge]]\nfrom = \"{}\"\nport = \"ok\"\nto = \"{}\"\nkind = \"data\"\n\n",
            pair[0], pair[1]
        );
    }
    src.push_str(
        "[[edge]]\nfrom = \"gate\"\nport = \"pass\"\nto = \"done\"\nkind = \"data\"\n\n\
         [[edge]]\nfrom = \"gate\"\nport = \"fail\"\nto = \"top\"\nkind = \"data\"\n",
    );
    src
}

/// Writes `docs` (and any extra graphs) under a fresh temp dir; the caller keeps the returned dir alive.
fn load(host: &str, graphs: &[(&str, &str)]) -> Result<fragment::Woven, PiiError> {
    let root = tempfile::tempdir().expect("tempdir");
    let dir = root.path().join("graphs");
    std::fs::create_dir_all(&dir).expect("mkdir graphs");
    for (name, src) in graphs {
        std::fs::write(dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    let reg = registry().0;
    let fragments: PathBuf = root.path().join("fragments");
    fragment::load_host_with_graphs(host, &fragments, std::slice::from_ref(&dir), &reg)
}

/// Loads a host that calls `docs`, runs it, and returns the terminal and the gate's run count.
fn run_looped(host: &str, graphs: &[(&str, &str)]) -> (Terminal, usize) {
    let woven = load(host, graphs).expect("load");
    let (reg, hits) = registry();
    let root = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&root.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let terminal = run::run_with_validate(
        &store,
        &woven.spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .expect("run");
    (terminal, hits.load(Ordering::SeqCst))
}

fn ctx(run: &pii_abi::RunId) -> Ctx {
    Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(RecordingObserver::new()) as Arc<dyn Observer>,
        TraceCtx {
            run: run.clone(),
            span: 0,
        },
    )
}

fn reason(terminal: &Terminal) -> String {
    match terminal {
        Terminal::Failure { error } => error.clone(),
        other => panic!("expected a failed run, got {other:?}"),
    }
}

#[test]
fn a_call_in_a_loop_stops_after_max_entries() {
    let host = looped(&[call("docs", Some(3))], &["docs"], 20, 10);
    let (terminal, gate_runs) = run_looped(&host, &[("docs", SUB)]);
    let why = reason(&terminal);
    assert!(why.contains("call 'docs'"), "{why}");
    assert!(why.contains('3'), "{why}");
    assert_eq!(gate_runs, 3, "exactly 3 entries ran: {why}");
}

#[test]
fn control_a_loop_that_needs_two_entries_passes() {
    let host = looped(&[call("docs", Some(3))], &["docs"], 20, 2);
    let (terminal, _) = run_looped(&host, &[("docs", SUB)]);
    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
}

#[test]
fn boundary_exactly_max_entries_succeeds() {
    let host = looped(&[call("docs", Some(3))], &["docs"], 20, 3);
    let (terminal, gate_runs) = run_looped(&host, &[("docs", SUB)]);
    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
    assert_eq!(gate_runs, 3);
}

#[test]
fn the_default_is_eight() {
    // The loop budget (20) is above 8, so the call's default limit is what fires.
    let host = looped(&[call("docs", None)], &["docs"], 20, 100);
    let (terminal, gate_runs) = run_looped(&host, &[("docs", SUB)]);
    let why = reason(&terminal);
    assert!(why.contains("call 'docs'") && why.contains("(8)"), "{why}");
    assert_eq!(gate_runs, 8);
}

#[test]
fn a_single_call_is_fine_at_one() {
    let host = format!(
        "name = \"host\"\nentry = \"start\"\n\n[[node]]\nid = \"start\"\nkind = \"transform\"\nvalue = \"go\"\n\n\
         {}\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n\
         [[edge]]\nfrom = \"start\"\nport = \"ok\"\nto = \"docs\"\nkind = \"data\"\n\n\
         [[edge]]\nfrom = \"docs\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n",
        call("docs", Some(1))
    );
    let (terminal, _) = run_looped(&host, &[("docs", SUB)]);
    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
}

#[test]
fn a_bad_value_is_refused_at_load() {
    for bad in ["0", "-1", "\"x\"", "1.5"] {
        let host = looped(
            &[call_with("docs", &format!("max_entries = {bad}"))],
            &["docs"],
            20,
            2,
        );
        let err = load(&host, &[("docs", SUB)])
            .expect_err("a bad max_entries must be refused")
            .to_string();
        assert!(err.contains("graph.call.max_entries"), "{bad}: {err}");
    }
}

#[test]
fn an_enclosing_loop_does_not_reset_the_count() {
    // The host's loop re-enters the call each pass; the call's own count runs across all of them, and the loop
    // budget (20) never fires first.
    let host = looped(&[call("docs", Some(3))], &["docs"], 20, 100);
    let (terminal, gate_runs) = run_looped(&host, &[("docs", SUB)]);
    let why = reason(&terminal);
    assert!(!why.contains("loop_budget"), "{why}");
    assert_eq!(gate_runs, 3);
}

#[test]
fn two_calls_count_apart() {
    // Each call enters 3 times over 3 passes: with one shared counter (6 entries) the run would fail.
    let host = looped(
        &[call("a", Some(3)), call("b", Some(3))],
        &["a", "b"],
        20,
        3,
    );
    let (terminal, gate_runs) = run_looped(&host, &[("docs", SUB)]);
    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
    assert_eq!(gate_runs, 3);
}

#[test]
fn a_nested_call_names_its_full_path() {
    // `outer` loops and holds `inner`, which allows 2 entries; the third names `outer__inner`.
    let outer = format!(
        "name = \"outer\"\nentry = \"ot\"\nexit = \"og\"\n\n\
         [[node]]\nid = \"ot\"\nkind = \"transform\"\nvalue = \"o\"\nloop_budget = 20\njoin = \"any\"\n\n\
         {}\n[[node]]\nid = \"ogate\"\nkind = \"gate_n\"\npass_at = 100\n\n\
         [[node]]\nid = \"og\"\nkind = \"transform\"\nvalue = \"og\"\n\n\
         [[edge]]\nfrom = \"ot\"\nport = \"ok\"\nto = \"inner\"\nkind = \"data\"\n\n\
         [[edge]]\nfrom = \"inner\"\nport = \"ok\"\nto = \"ogate\"\nkind = \"data\"\n\n\
         [[edge]]\nfrom = \"ogate\"\nport = \"pass\"\nto = \"og\"\nkind = \"data\"\n\n\
         [[edge]]\nfrom = \"ogate\"\nport = \"fail\"\nto = \"ot\"\nkind = \"data\"\n",
        call_with("inner", "max_entries = 2")
    );
    let host = looped(
        &[call_of("outer", "outer", &pin(&outer), "")],
        &["outer"],
        20,
        100,
    );
    let (terminal, _) = run_looped(&host, &[("docs", SUB), ("outer", &outer)]);
    let why = reason(&terminal);
    assert!(why.contains("outer__inner"), "{why}");
}

#[test]
fn the_count_survives_a_resume() {
    // The run enters the call twice (max 2), then its journal is cut after the second gate pass and resumed. The
    // replay must rebuild the two entries, so the resumed run refuses its third entry before the gate runs again.
    let host = looped(&[call("docs", Some(2))], &["docs"], 20, 100);
    let woven = load(&host, &[("docs", SUB)]).expect("load");
    let (reg, hits) = registry();
    let root = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&root.path().join("db.redb")).expect("store open");
    let first = store.next_run_id().expect("run id");
    let full = run::run_with_validate(
        &store,
        &woven.spec,
        &reg,
        &ctx(&first),
        &first,
        serde_json::json!({}),
        true,
    )
    .expect("run");
    assert!(reason(&full).contains("(2)"), "{full:?}");
    let journal = store.read_journal(&first).expect("journal");
    let mut gate_passes = 0;
    let cut = journal
        .iter()
        .position(|(_, ev)| {
            if matches!(ev, RunEvent::NodeFinished { node, .. } if node.as_str() == "gate") {
                gate_passes += 1;
            }
            gate_passes == 2
        })
        .expect("the gate finishes twice");
    let prefix: Vec<RunEvent> = journal[1..=cut].iter().map(|(_, ev)| ev.clone()).collect();
    let resumed = common::seed_run(&store, &woven.spec, &prefix);
    let terminal = run::resume(&store, &reg, &ctx(&resumed), &resumed, None).expect("resume");
    let why = reason(&terminal);
    assert!(why.contains("(2)"), "{why}");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        2,
        "the resumed run never re-enters the gate"
    );
}
