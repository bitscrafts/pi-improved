//! `cargo xtask` dev-tools runner: deps-check, lint-files, ci.

#![forbid(unsafe_code)]
// reason: the gate runner reports violations on stderr; the output
// abstractions replacing direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod bench;
mod deps;
mod doc_drift;
mod lint_files;
mod memory_eval;
pub mod net_check;
mod proto_gen;
mod reachability;
mod rss_gate;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <deps-check|net-check|lint-files|memory-eval|rss-gate|proto-gen|proto-check|bench-gate [--budget-ms N]|ci>";

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

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1);
    let rest: Vec<String> = std::env::args().skip(2).collect();
    match cmd.as_deref() {
        Some("deps-check") => report(deps::run()),
        Some("net-check") => report(net_check::run()),
        Some("reachability-check") => report(reachability::run(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )),
        Some("doc-drift-check") => report(doc_drift::run(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )),
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
        Some("lint-files") => report(lint_files::run()),
        Some("proto-gen") => report(proto_gen::generate().map_err(|e| vec![e])),
        Some("proto-check") => report(proto_gen::check()),
        Some("bench-gate") => match bench::run_cli(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("ci") => {
            let deps_ok = report(deps::run());
            let net_ok = report(net_check::run());
            let lint_ok = report(lint_files::run());
            let proto_ok = report(proto_gen::check());
            let bench_ok = report(bench::run_cli(&rest).map_err(|e| vec![e]));
            // F8 (opus review): `cargo xtask ci` must run the full gate set —
            // fmt, clippy, tests — not just deps/lint-files/bench. This is
            // the local runner that §1.6 calls "the real gate".
            let fmt_ok = run_cmd("cargo", &["fmt", "--all", "--check"]);
            let clippy_ok = run_cmd(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            );
            let test_ok = run_cmd("cargo", &["test", "--workspace"]);
            if deps_ok == ExitCode::SUCCESS
                && net_ok == ExitCode::SUCCESS
                && lint_ok == ExitCode::SUCCESS
                && proto_ok == ExitCode::SUCCESS
                && bench_ok == ExitCode::SUCCESS
                && fmt_ok == ExitCode::SUCCESS
                && clippy_ok == ExitCode::SUCCESS
                && test_ok == ExitCode::SUCCESS
            {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Parses `memory-eval --corpus <f> --queries <f> [--top-k N]` (F4.16).
fn parse_mem_eval(rest: &[String]) -> Result<(String, String, usize), String> {
    let mut corpus = String::new();
    let mut queries = String::new();
    let mut top_k = 10usize;
    let mut i = 0usize;
    while i < rest.len() {
        match rest.get(i).map(String::as_str) {
            Some("--corpus") => {
                i += 1;
                corpus = rest.get(i).cloned().ok_or("--corpus needs a value")?;
            }
            Some("--queries") => {
                i += 1;
                queries = rest.get(i).cloned().ok_or("--queries needs a value")?;
            }
            Some("--top-k") => {
                i += 1;
                top_k = rest
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--top-k needs a number")?;
            }
            other => return Err(format!("unknown memory-eval arg '{other:?}'")),
        }
        i += 1;
    }
    if corpus.is_empty() || queries.is_empty() {
        return Err(String::from("memory-eval needs --corpus and --queries"));
    }
    Ok((corpus, queries, top_k))
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

    let corpus =
        std::fs::read_to_string("/projects/pi-improved/fixtures/phase3/memory/corpus.jsonl")
            .expect("corpus fixture");
    let queries =
        std::fs::read_to_string("/projects/pi-improved/fixtures/phase3/memory/queries.jsonl")
            .expect("queries fixture");
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

/// Parses `rss-gate [--secs N] [--interval N] [--max-mb N] [--runs N]` (F5.14).
fn parse_rss_gate(rest: &[String]) -> Result<(u64, u64, u64, u32), String> {
    let mut secs = 300u64;
    let mut interval = 10u64;
    let mut max_mb = 50u64;
    let mut runs = 3000u32;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let next_u64 = |name: &str, it: &mut std::slice::Iter<'_, String>| -> Result<u64, String> {
            it.next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{name} needs a number"))
        };
        match arg.as_str() {
            "--secs" => secs = next_u64("--secs", &mut it)?,
            "--interval" => interval = next_u64("--interval", &mut it)?,
            "--max-mb" => max_mb = next_u64("--max-mb", &mut it)?,
            "--runs" => {
                let v = next_u64("--runs", &mut it)?;
                runs = v.try_into().map_err(|_| "--runs too large")?;
            }
            other => return Err(format!("unknown rss-gate arg '{other}'")),
        }
    }
    Ok((secs, interval, max_mb, runs))
}
