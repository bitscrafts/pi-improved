//! `cargo xtask` dev-tools runner: deps-check, lint-files, ci.

#![forbid(unsafe_code)]
// reason: the gate runner reports violations on stderr; the output
// abstractions replacing direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]
// reason: spec 67 qualified the `std::println` disallowed-macro rule; xtask
// dev-tooling intentionally prints to stdout (see lib.rs for the full note).
#![allow(clippy::disallowed_macros)]
#![allow(clippy::too_long_first_doc_paragraph)]
// reason: dev-tooling doc comments carry the full spec-34 context in one
// paragraph; splitting them would hide the load-bearing detail.
// The private `mod` items below link the same source files as the lib,
// whose `pub` items (spec 72: the integration tests drive the real
// `reachability` gate) are then "unreachable" from this bin — by design.
#![allow(unreachable_pub)]

mod bench;
pub mod cli_parse;
mod coverage_floor;
mod crate_audit;
mod deps;
mod disk;
mod doc_drift;
mod fresh_gate;
pub mod heal_collect;
mod heal_prune;
pub mod heal_seed;
mod lint_files;
mod lint_graph_kinds;
mod lint_obs_io;
mod lint_observability;
mod lint_scratch;
mod memory_eval;
pub mod net_check;
pub mod promote_build;
mod proto_gen;
mod reachability;
mod rss_gate;
mod silent_noop;
mod test_inventory;

use std::process::ExitCode;

use xtask::workspace_root;

use cli_parse::{
    parse_mem_eval, parse_promote_collect, parse_report_flag, parse_rss_gate, parse_worktree_flag,
};

const USAGE: &str = "usage: cargo xtask <deps-check|net-check|lint-files|lint-scratch|lint-observability|lint-graph-kinds [--root <dir>]|coverage-floor <summary.json> [--floor N]|silent-noop-check|reachability-check|doc-drift-check [--modules]|doc-drift-inventory|memory-eval|rss-gate|heal-seed|heal-collect <sig>|promote-build [--worktree P]|promote-collect <sig> <approval_id> [--approval-db P] [--by NAME]|heal-prune [--all]|fresh-gate [--worktree] [--overlay src=dest]|proto-gen|proto-check|bench-gate [--budget-ms N]|crate-audit [--ratchet|--update-baseline]|ci>";

fn report(result: Result<(), Vec<String>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(violations) => {
            for line in violations {
                eprintln!("error: {line}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Runs a subcommand and reports its exit status (F8: `xtask ci` shells out
/// to fmt/clippy/test). Dev-tooling, so the Command ban does not apply.
#[allow(clippy::disallowed_methods)]
fn run_cmd(prog: &str, args: &[&str]) -> ExitCode {
    match std::process::Command::new(prog).args(args).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: failed to run {prog}: {e}");
            ExitCode::from(2)
        }
    }
}

/// Spec 379 criterion: `graph lint <file>` is the static check `pii graph lint` runs, reached through the binary so
/// xtask keeps no graph engine of its own.
fn graph_lint(rest: &[String]) -> ExitCode {
    let mut args = vec!["run", "-q", "-p", "pii-cli", "--bin", "pii", "--", "graph"];
    args.extend(rest.iter().map(String::as_str));
    run_cmd("cargo", &args)
}

/// Runs the full local gate set (F8): deps/net/lint/proto/bench plus
/// fmt/clippy/test — "the real gate" per §1.6. Split out of `main` to keep
/// it under the 80-line function-length lint.
fn run_ci(rest: &[String]) -> ExitCode {
    let deps_ok = report(deps::run());
    let net_ok = report(net_check::run());
    let lint_ok = report(lint_files::run());
    let proto_ok = report(proto_gen::check());
    let bench_ok = report(bench::run_cli(rest).map_err(|e| vec![e]));
    // E4 (spec 22): the two self-improvement gates the Phase-4 plan called
    // "the strongest machinery in this repo" must be enforced by CI, not
    // opt-in. They catch the silent-no-op class (declared config key with no
    // reader, event with no producer, RPC with no dispatch arm).
    let reach_ok = report(reachability::run(&workspace_root()));
    let drift_ok = report(doc_drift::run(&workspace_root()));
    let silent_noop_ok = report(silent_noop::run(&workspace_root(), false));
    // Spec 67 (R3): delegate fmt/clippy/build/test to quality-gate.sh —
    // it isolates clippy and doctests into their own CARGO_TARGET_DIR
    // (the F-01 fix) so this shared-target-dir race cannot recur here.
    // The gates above stay local: quality-gate.sh does not run deps/net/
    // lint/proto/bench/reachability/drift/silent-noop.
    let cargo_gate_ok = run_cmd("bash", &["quality-gate.sh"]);
    if deps_ok == ExitCode::SUCCESS
        && net_ok == ExitCode::SUCCESS
        && lint_ok == ExitCode::SUCCESS
        && proto_ok == ExitCode::SUCCESS
        && bench_ok == ExitCode::SUCCESS
        && reach_ok == ExitCode::SUCCESS
        && drift_ok == ExitCode::SUCCESS
        && silent_noop_ok == ExitCode::SUCCESS
        && cargo_gate_ok == ExitCode::SUCCESS
    {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[allow(clippy::too_many_lines)] // reason: one argv dispatch arm per xtask subcommand
fn main() -> ExitCode {
    let cmd = std::env::args().nth(1);
    let rest: Vec<String> = std::env::args().skip(2).collect();
    match cmd.as_deref() {
        Some("deps-check") => report(deps::run()),
        Some("net-check") => {
            // Spec 325 amendment: a gate step that says nothing on success cannot be observed — a criterion
            // written as `tail -1 | grep -q PASS` then fails on a correct tree (measured), and a reader cannot
            // tell "passed" from "did not run". State the verdict like every other step does.
            let r = net_check::run();
            if r.is_ok() {
                println!("PASS: net-check");
            }
            report(r)
        }
        Some("reachability-check") => report(reachability::run(&workspace_root())),
        Some("doc-drift-check") => {
            // Spec 374 R3: `--modules` checks the module headers of the tree instead of HANDOFF.md.
            if std::env::args().any(|a| a == "--modules") {
                report(doc_drift::run_modules(&workspace_root()))
            } else {
                report(doc_drift::run(&workspace_root()))
            }
        }
        Some("doc-drift-inventory") => {
            // Spec 374 R1: prints the undocumented modules, one per line; nothing printed means nothing to document.
            for module in doc_drift::inventory(&workspace_root()) {
                println!("{module}");
            }
            ExitCode::SUCCESS
        }
        Some("silent-noop-check") => {
            let want_list = std::env::args().any(|a| a == "--list");
            report(silent_noop::run(&workspace_root(), want_list))
        }
        Some("rss-gate") => match parse_rss_gate(&rest) {
            Ok((secs, interval, max_mb, runs)) => match rss_gate::run(secs, interval, max_mb, runs)
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("memory-eval") => match parse_mem_eval(&rest) {
            Ok((c, q, k)) => match memory_eval::run(&c, &q, k) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("heal-seed") => match heal_seed::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("heal-collect") => {
            let Some(sig) = rest.first() else {
                eprintln!("error: heal-collect needs a <sig>");
                return ExitCode::from(2);
            };
            match heal_collect::run(sig) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("promote-build") => {
            let worktree = parse_worktree_flag(&rest);
            match promote_build::run(worktree) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("promote-collect") => match parse_promote_collect(&rest) {
            Ok((sig, approval_id, approval_db, by)) => {
                match heal_collect::collect_from_approval(
                    &sig,
                    &approval_id,
                    &approval_db,
                    by.as_deref(),
                ) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(2)
            }
        },
        Some("heal-prune") => {
            let all = rest.iter().any(|a| a == "--all");
            match heal_prune::run(all) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("lint-files") => {
            // spec 69: `--report [N]` prints top-N by effective lines and
            // always exits 0; without the flag the pass/fail gate is unchanged.
            match parse_report_flag(&rest) {
                Ok(Some(n)) => report(lint_files::run_report(n)),
                Ok(None) => report(lint_files::run()),
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Some("lint-scratch") => report(lint_scratch::run()),
        Some("lint-graph-kinds") => report(lint_graph_kinds::run(&rest)),
        Some("crate-audit") => report(crate_audit::run(&rest)),
        Some("coverage-floor") => report(coverage_floor::run(&rest)),
        Some("lint-observability") => report(lint_observability::run()),
        Some("proto-gen") => report(proto_gen::generate().map_err(|e| vec![e])),
        Some("proto-check") => report(proto_gen::check()),
        Some("bench-gate") => match bench::run_cli(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("fresh-gate") => match fresh_gate::run(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("disk") => match disk::run(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Some("ci") => run_ci(&rest),
        Some("graph") => graph_lint(&rest),
        Some("test-inventory") => {
            let check = rest.iter().any(|a| a == "--check");
            report(test_inventory::run(&workspace_root(), check))
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// spec's exit criterion.
#[cfg(test)]
mod top_level_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing)] // tests assert on indexed fixture shapes
    use crate::memory_eval;
    use crate::net_check;

    #[test]
    fn net_check_rejects_socket_outside_pii_net() {
        let src = concat!(
            "fn f() {\n    let _ = std::net::TcpStream",
            "::connect(\"h:80\");\n}\n"
        );
        let v = net_check::scan_one("crates/pii-x/src/a.rs", src, net_check::ALLOWLIST);
        assert_eq!(v.len(), 1, "one violation: {v:?}");
        let first = v.first().map_or("", String::as_str);
        assert!(
            first.contains("crates/pii-x/src/a.rs"),
            "names file: {first}"
        );
        assert!(
            first.contains(concat!("TcpStream", "::connect")),
            "names symbol: {first}"
        );
        assert!(first.starts_with("NET-CHECK FAIL: "), "prefix: {first}");
    }

    #[test]
    fn memory_eval_fixture_is_not_rigged() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let (docs, queries) = memory_eval::parse_fixtures(
            root.join("fixtures/phase3/memory/corpus.jsonl")
                .to_str()
                .expect("corpus path"),
            root.join("fixtures/phase3/memory/queries.jsonl")
                .to_str()
                .expect("queries path"),
        )
        .expect("fixtures parse");
        assert!(docs.len() >= 200, "corpus must have >= 200 docs");
        assert_eq!(queries.len(), 20, "exactly 20 queries");
        for q in &queries {
            for rid in &q.relevant {
                let doc_text = docs
                    .iter()
                    .find(|d| &d.id == rid)
                    .map_or(String::new(), |d| d.text.clone());
                assert!(
                    !doc_text.contains(&q.q),
                    "query {:?} appears verbatim in doc {rid}",
                    q.q
                );
            }
        }
    }

    /// R5 (spec 22, E4): the `run_ci` gate set must enforce the two
    /// self-improvement gates — reachability-check and doc-drift-check — not
    /// leave them as opt-in subcommands. Static source assertion (xtask is a
    /// dev-tooling binary, no integration harness). THIS is the guard against
    /// the silent-no-op class regressing.
    #[test]
    fn run_ci_includes_self_improvement() {
        #![allow(clippy::panic)] // reason: test asserts the gate body
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("read own source for run_ci assertion");
        // Isolate the body of `run_ci` (from its signature to the next `\n\nfn`).
        let start = src.find("fn run_ci(").expect("run_ci must exist");
        let body = &src[start..];
        let end = body.find("\nfn ").unwrap_or(body.len());
        let body = &body[..end];
        // E4: both self-improvement gates must be invoked by `run_ci`.
        assert!(
            body.contains("reachability::run"),
            "run_ci must call reachability::run (E4)"
        );
        assert!(
            body.contains("doc_drift::run"),
            "run_ci must call doc_drift::run (E4)"
        );
        // Spec 24 R2: the silent-noop gate (the declared-but-unwired detector)
        // is the third self-improvement gate CI must enforce.
        assert!(
            body.contains("silent_noop::run"),
            "run_ci must call silent_noop::run (spec 24 R2)"
        );
        // USAGE must advertise them so they are not an undiscoverable opt-in.
        let usage = src
            .find("const USAGE: &str = ")
            .expect("USAGE const must exist");
        let usage_end = src[usage..].find(';').expect("USAGE ends with ;");
        let usage = &src[usage..usage + usage_end];
        assert!(
            usage.contains("reachability-check"),
            "USAGE must list reachability-check"
        );
        assert!(
            usage.contains("doc-drift-check"),
            "USAGE must list doc-drift-check"
        );
        assert!(
            usage.contains("silent-noop-check"),
            "USAGE must list silent-noop-check (spec 24 R2)"
        );
    }
}
#[test]
fn deps_check_rejects_rustls() {
    #![allow(clippy::panic)] // reason: test asserts a violation exists
    let m = deps::tests::clean_metadata();
    // The fixture needs a package named `rustls` for the id->name map, as
    // real metadata always carries the dependency's own package entry.
    let with_rustls = deps::tests::push_package(
        m,
        serde_json::json!({"id":"rustls","name":"rustls","version":"0.23.0",
               "dependencies":[]}),
        serde_json::json!([]),
    );
    let m2 = deps::tests::push_package(
        with_rustls,
        serde_json::json!({"id":"tlsx","name":"pii-tlsx","version":"1.0.0",
               "dependencies":[{"name":"rustls","kind":null}]}),
        serde_json::json!([{"pkg":"rustls"}]),
    );
    let errs = match deps::check_all(&m2) {
        Ok(v) => panic!("pii-tlsx -> rustls must be rejected, got ok: {v:?}"),
        Err(e) => e,
    };
    assert!(
        errs.iter()
            .any(|e| e.contains("pii-tlsx") && e.contains("rustls")),
        "rustls confinement violation: {errs:?}"
    );
}
/// Spec 122 R1: wasmtime is a `pii-wasm`-only dependency.
#[cfg(test)]
#[test]
fn deps_check_rejects_wasmtime_outside_pii_wasm() {
    #![allow(clippy::panic)] // reason: test asserts a violation exists
    let m = deps::tests::clean_metadata();
    let with_wt = deps::tests::push_package(
        m,
        serde_json::json!({"id":"wt","name":"wasmtime","version":"48.0.5",
               "dependencies":[]}),
        serde_json::json!([]),
    );
    let m2 = deps::tests::push_package(
        with_wt,
        serde_json::json!({"id":"srv","name":"pii-server","version":"1.0.0",
               "dependencies":[{"name":"wasmtime","kind":null}]}),
        serde_json::json!([{"pkg":"wt"}]),
    );
    let errs = match deps::check_all(&m2) {
        Ok(v) => panic!("pii-server -> wasmtime must be rejected, got ok: {v:?}"),
        Err(e) => e,
    };
    assert!(
        errs.iter()
            .any(|e| e.contains("pii-server") && e.contains("wasmtime")),
        "wasmtime confinement violation: {errs:?}"
    );
}
/// F4.16/exit-criterion test (top-level so `-- --exact` matches the bare path).
#[cfg(test)]
#[test]
fn deps_check_rejects_redb_outside_store() {
    #![allow(clippy::panic)] // reason: test asserts a violation exists
    let m = deps::tests::clean_metadata();
    let m2 = deps::tests::push_package(
        m,
        serde_json::json!({"id":"mem","name":"pii-memory","version":"1.0.0",
               "dependencies":[{"name":"redb","kind":null}]}),
        serde_json::json!([{"pkg":"redb"}]),
    );
    let errs = match deps::check_all(&m2) {
        Ok(v) => panic!("pii-memory -> redb must violate invariant 5, got ok: {v:?}"),
        Err(e) => e,
    };
    assert!(
        errs.iter()
            .any(|e| e.contains("pii-memory") && e.contains("redb")),
        "invariant 5 violation naming pii-memory: {errs:?}"
    );
}
/// F4.17 (top-level so `-- --exact` matches the bare path): the fixture is
/// not rigged — machine-checked corpus/query constraints.
#[test]
fn memory_eval_fixture_is_not_rigged() {
    #![allow(clippy::expect_used)] // test fixture: fixture files must exist
    #![allow(clippy::panic)] // test fixture: missing relevant doc is a hard failure
    use std::collections::BTreeMap;

    let fixtures = workspace_root().join("fixtures/phase3/memory");
    let corpus = std::fs::read_to_string(fixtures.join("corpus.jsonl")).expect("corpus fixture");
    let queries = std::fs::read_to_string(fixtures.join("queries.jsonl")).expect("queries fixture");
    let mut docs: Vec<(String, String)> = Vec::new();
    for line in corpus.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).expect("corpus json");
        docs.push((
            v.get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            v.get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ));
    }
    let mut qs: Vec<(String, Vec<String>)> = Vec::new();
    for line in queries.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).expect("query json");
        let rel = v
            .get("relevant")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        qs.push((
            v.get("q")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            rel,
        ));
    }
    // ≥ 200 docs; exactly 20 queries; 1–3 relevant each.
    assert!(docs.len() >= 200, "corpus must have >= 200 docs");
    assert_eq!(qs.len(), 20, "exactly 20 queries");
    let by_id: BTreeMap<String, String> = docs.iter().cloned().collect();
    for (q, rel) in &qs {
        assert!(
            !rel.is_empty() && rel.len() <= 3,
            "1-3 relevant docs per query"
        );
        for r in rel {
            let text = by_id
                .get(r)
                .unwrap_or_else(|| panic!("relevant doc {r} missing from corpus"));
            assert!(
                !text.contains(q.as_str()),
                "query '{q}' appears verbatim in relevant doc {r} — rigged"
            );
        }
    }
}
