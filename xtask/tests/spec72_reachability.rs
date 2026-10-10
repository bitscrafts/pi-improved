//! Spec 72 TDD Contract — integration tests for the reachability gate.
//!
//! The three acceptance rows run the REAL check (the same `reachability::run`
//! `cargo xtask reachability-check` and `quality-gate.sh` execute) against a
//! scratch `git archive` export of this workspace, never the live tree:
//!
//! | Test | Scenario | Expected |
//! |---|---|---|
//! | `reachability_fails_when_callers_deleted` | both real callers commented out | exit != 0, message names the orphaned symbol |
//! | `reachability_passes_on_head` | unmodified export | exit 0 |
//! | `comments_are_not_callers` | comment containing `.invoke(` in an unrelated crate is the only textual match | exit != 0 — the comment does not satisfy |
//!
//! The scratch export is built under `_tmp/` (project-relative, resolved
//! from `CARGO_MANIFEST_DIR`, never a system directory) and is deleted at
//! the end of each test. The real callers muted are exactly
//! a source file resolved at test time (`pii_model::resolve_with_transport(`, historically in
//! `crates/pii-server/src/lib.rs`, moved to `observed_provider.rs` by spec 129)
//! and `crates/pii-server/src/node_kinds/nodes/worker.rs` (`sup.invoke(`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::indexing_slicing)] // tests index scratch fixtures
#![allow(clippy::disallowed_methods)]
// reason: dev-tooling spawns `git archive`/`tar` like the bin does (the
// Phase-1 Ctx::proc ban targets agent-runtime code, not xtask tooling —
// same exemption main.rs carries for run_cmd).

use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::reachability::{SEED, run};

/// Workspace root, project-relative (spec 72 R3: no host paths in-tree).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives at the workspace root")
        .to_path_buf()
}

/// Builds a fresh scratch export of the current workspace tree via
/// `git archive` (spec 72: NEVER mutate the real tree mid-check). The
/// export carries no `.git`, so the check cannot be confused by history.
fn scratch_export(tag: &str) -> PathBuf {
    let root = workspace_root()
        .join("_tmp")
        .join("spec72-reachability-integration")
        .join(tag);
    // Reuse the same scratch tag deterministically: a leftover from a
    // earlier failed run must never satisfy a later assertion.
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("scratch dir under _tmp/");

    // `git archive HEAD | tar -x` — a clean snapshot of the committed tree
    // (which includes the working implementation, since the worker's tree
    // is the source of truth for this check's basis).
    // A `git archive` export (the fresh-gate tree) has no `.git`: snapshot the
    // source files themselves, which are exactly HEAD there.
    let mut snapshot = Command::new("git");
    if workspace_root().join(".git").exists() {
        snapshot.args(["archive", "HEAD"]);
    } else {
        snapshot = Command::new("tar");
        snapshot.args(["-c", "--exclude=./target", "--exclude=./_tmp", "."]);
    }
    let archive = snapshot
        .current_dir(workspace_root())
        .output()
        .expect("snapshot of the workspace runs");
    assert!(
        archive.status.success(),
        "git archive HEAD failed: {}",
        String::from_utf8_lossy(&archive.stderr)
    );
    let mut extract = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&root)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("tar extracts the archive");
    {
        use std::io::Write as _;
        let mut stdin = extract.stdin.take().expect("tar reads stdin");
        stdin
            .write_all(&archive.stdout)
            .expect("pipe archive to tar");
    }
    let out = extract.wait_with_output().expect("tar finishes");
    assert!(
        out.status.success(),
        "tar extract failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    root
}

/// Comments out the real caller line in `rel` (keeps every other line, so
/// nothing but the call site is muted). Panics when the expected needle is
/// not found — the mutation must be proven to have landed.
fn mute_caller(export: &Path, rel: &str, needle: &str) {
    let file = export.join(rel);
    let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    let mut muted = String::with_capacity(src.len());
    let mut found = false;
    for line in src.lines() {
        if !found && line.contains(needle) {
            muted.push_str("// [spec72 test] muted: ");
            found = true;
        }
        muted.push_str(line);
        muted.push('\n');
    }
    assert!(found, "mutation failed: {needle} not found in {rel}");
    std::fs::write(&file, muted).unwrap_or_else(|e| panic!("write {rel}: {e}"));
}

/// The two real external callers (spec 72 R1): the only evidence the gate
/// depends on. Paths relative to the workspace root.
///
/// `CALLER_WORKER` is a fixed path; `CALLER_LIB` is **resolved at test time**
/// instead of pinned. A refactor may move the call without changing the claim —
/// spec 129 moved `resolve_with_transport` out of `lib.rs` into
/// `observed_provider.rs`, and a pinned path made this test fail with
/// "mutation failed: … not found" even though the check was still observing a
/// real caller. The needle must still exist in exactly one non-test file under
/// the crate; if it does not, the test fails loudly because the reachability
/// check would then have no real caller to observe.
const CALLER_WORKER: (&str, &str) = (
    "crates/pii-server/src/node_kinds/nodes/worker.rs",
    "sup.invoke(",
);
const CALLER_LIB_NEEDLE: &str = "pii_model::resolve_with_transport(";
const CALLER_LIB_HISTORICAL: &str = "crates/pii-server/src/lib.rs";

/// Resolve the file that holds the `resolve_with_transport` caller: the
/// historical location when it still holds it, else the single non-test source
/// file under `crates/pii-server/src` that does.
fn caller_lib(export: &Path) -> (String, String) {
    let relative = |p: &Path| {
        p.strip_prefix(export)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    let holds = |p: &Path| {
        std::fs::read_to_string(p).is_ok_and(|s| s.lines().any(|l| l.contains(CALLER_LIB_NEEDLE)))
    };
    let historical = export.join(CALLER_LIB_HISTORICAL);
    if holds(&historical) {
        return (
            CALLER_LIB_HISTORICAL.to_string(),
            CALLER_LIB_NEEDLE.to_string(),
        );
    }
    fn walk(dir: &Path, needle: &str, hits: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, needle, hits);
            } else if p.extension().is_some_and(|x| x == "rs")
                && !p.to_string_lossy().contains("tests")
                && std::fs::read_to_string(&p).is_ok_and(|s| s.lines().any(|l| l.contains(needle)))
            {
                hits.push(p);
            }
        }
    }
    let mut hits = Vec::new();
    walk(
        &export.join("crates/pii-server/src"),
        CALLER_LIB_NEEDLE,
        &mut hits,
    );
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one non-test caller of {CALLER_LIB_NEEDLE}: {hits:?}"
    );
    (relative(&hits[0]), CALLER_LIB_NEEDLE.to_string())
}

/// TDD row 1: with BOTH real callers deleted (commented out) the check
/// must go RED — exit != 0 — and every failure message must name the
/// orphaned symbol. This is the check the 043 report proved blind (D3).
#[test]
fn reachability_fails_when_callers_deleted() {
    let export = scratch_export("both-muted");
    let lib = caller_lib(&export);
    mute_caller(&export, &lib.0, &lib.1);
    mute_caller(&export, CALLER_WORKER.0, CALLER_WORKER.1);

    let err = run(&export).expect_err("both callers muted must be RED");
    assert_eq!(
        err.len(),
        SEED.len(),
        "one message per orphaned entry: {err:?}"
    );
    for seed in SEED {
        let named = err.iter().any(|m| m.contains(seed.label));
        assert!(
            named,
            "failure output must name the orphaned symbol {}: {err:?}",
            seed.label
        );
    }
    let _ = std::fs::remove_dir_all(&export);
}

/// TDD row 2: the check passes on an unmodified export of HEAD (exit 0).
#[test]
fn reachability_passes_on_head() {
    let export = scratch_export("head");
    run(&export).expect("HEAD export must pass the reachability gate");
    let _ = std::fs::remove_dir_all(&export);
}

/// TDD row 3: a comment containing `.invoke(` in an unrelated crate is
/// NOT a caller. Both real callers are muted and the ONLY textual match
/// left for the `Supervisor::invoke` needle shape is a comment — the check
/// must still fail (the comment does not satisfy it).
#[test]
fn comments_are_not_callers() {
    let export = scratch_export("comment-shape");
    let lib = caller_lib(&export);
    mute_caller(&export, &lib.0, &lib.1);
    mute_caller(&export, CALLER_WORKER.0, CALLER_WORKER.1);
    // A comment carrying the needle shape, in an unrelated crate, as the
    // only remaining textual occurrence of `.invoke(` in the search dir.
    let decoy = export.join("crates/pii-server/src/queue.rs");
    std::fs::write(&decoy, "// a shape note: something.invoke( could go here\n").unwrap();

    let err = run(&export).expect_err("a comment is not a caller");
    assert!(
        err.iter().any(|m| m.contains("Supervisor::invoke")),
        "the comment must not satisfy the gate: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&export);
}

/// Spec 87 (N2): with both real callers muted, a call that lives only in a
/// `#[cfg(test)]` item (the stand-in sits outside `declare_dir`) is NOT a
/// caller — the cfg(test) state must carry across lines.
#[test]
fn cfg_test_body_is_not_a_caller() {
    let export = scratch_export("cfg-test-standin");
    let lib = caller_lib(&export);
    mute_caller(&export, &lib.0, &lib.1);
    mute_caller(&export, CALLER_WORKER.0, CALLER_WORKER.1);
    let decoy = export.join("crates/pii-server/src/queue.rs");
    std::fs::write(
        &decoy,
        "#[cfg(test)]\n#[allow(dead_code)]\nmod x {\n    fn f() { let _ = pii_model::resolve_with_transport(\"x\", todo!()); }\n}\n",
    )
    .unwrap();
    let err = run(&export).expect_err("a cfg(test)-only reference is not a caller");
    assert!(
        err.iter().any(|m| m.contains("resolve_with_transport")),
        "the stand-in must not satisfy the gate: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&export);
}
