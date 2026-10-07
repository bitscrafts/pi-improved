//! Spec 74 R1: containment enforced AT THE CALLER PATH (spec 62's worktree
//! join flow, seen from outside `pii-tools`).
//!
//! Drives the REAL `pii_tools::worktree::WorktreeTool` — the tool the heal
//! join flow provisions — against a fresh git-repo workspace, feeding a
//! path-traversal payload (`../../../etc`). On pristine HEAD the join is
//! REFUSED before anything is provisioned.
//!
//! Guard verdict (honest, mirroring 043 §3): with `validate_signature`
//! intact, every reachable payload that would escape `dir.join(sig)` is
//! refused by the validator before the join — so a call-site-only revert is
//! NOT observable through any tool input. The validator and the post-join
//! containment check guard AS A PAIR; the mutation this test is proven
//! against stubs the validator AND reverts the call site (043's `m_two.py`).
//! Under that pair-mutation this test is red; on HEAD it is green. A
//! containment-only revert leaves the suite green — that is recorded in the
//! mutation log, not claimed as a pass.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_tools::{Tool, worktree::WorktreeTool};
use serde_json::json;

/// Crate-local `/_tmp` scratch root (never /tmp).
fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

/// A context granting only `PROC` for `/bin/sh` — the argv0 a worktree run
/// grants (same shape as the tool's own unit-test fixture in pii-tools).
fn proc_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
    Ctx::root(caps, Budget::unlimited())
}

/// A temp dir that is a real git repo with one commit on `HEAD`
/// (`worktree add ... HEAD` needs it). One-off fixture SETUP spawns git
/// directly rather than through `Ctx::proc()` — scaffolding, not the tool
/// under test (same rationale as the pii-tools fixture, which carries this
/// exact allow).
#[allow(clippy::disallowed_methods)]
fn git_repo(prefix: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(tmp_root())
        .unwrap();
    let canon = dunce::canonicalize(dir.path()).expect("canonicalize fixture");
    let run = |cmd: &str| {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&canon)
            .output()
            .expect("spawn git (fixture setup)");
        assert!(
            out.status.success(),
            "fixture `{cmd}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run("git init -q -b main .");
    run("git config user.email t@t && git config user.name t");
    std::fs::write(canon.join("seed.txt"), "seed\n").unwrap();
    run("git add seed.txt && git commit -q -m seed");
    (dir, canon)
}

/// R1: the caller path refuses a path-traversal payload as an Invalid
/// (guard-shaped) error. Under the pair-mutation (validator stubbed + call
/// site reverted) the payload reaches git and the error is a non-guard
/// `Internal`, which fails this assertion — that is the RED teeth.
#[test]
fn worktree_join_refuses_path_traversal_payload() {
    let (_tmp, canon) = git_repo("wt74-join-");
    let ctx = proc_ctx();
    let tool = WorktreeTool { workspace: canon };
    let err = tool
        .call(&ctx, &json!({ "signature": "../../../etc" }))
        .expect_err("the traversal payload must be refused");
    assert!(
        matches!(err, PiiError::Invalid(_)),
        "refusal must be the Invalid guard error, got: {err}"
    );
    assert!(
        err.to_string().contains("signature"),
        "refusal must name the signature guard, got: {err}"
    );
}
