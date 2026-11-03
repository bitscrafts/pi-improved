//! Spec 349: a treatment travels. `pii treat export|import|apply|list` moves one proven fix (patch + reproducer +
//! evidence) between installations as a directory bundle, and proves it again on the importing machine before the
//! tree is left changed. Each test runs the built `pii` binary against its own git repository and state dir.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and creates git repositories.
#![allow(clippy::disallowed_methods)]
// reason: the test names are the spec's CONTROL_ names, which are not snake case by design.
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pii_store::heal::{ApprovalDecision, HealAttempt, HealState, HealStore};

const SIG: &str = "refusal-repeated:channel.poll.failed";
const PATCH: &str =
    "diff --git a/app.txt b/app.txt\n--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
const REPRO: &str = "grep -q '^fixed$' app.txt\n";

fn scratch(prefix: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .unwrap()
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A repository whose `app.txt` says `content`, committed; returns the HEAD revision.
fn repo_with(dir: &Path, content: &str) -> String {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "treat@example.invalid"]);
    git(dir, &["config", "user.name", "treat"]);
    std::fs::write(dir.join("app.txt"), content).unwrap();
    git(dir, &["add", "app.txt"]);
    git(dir, &["commit", "-q", "-m", "base"]);
    git(dir, &["rev-parse", "HEAD"])
}

fn pii(args: &[&str], cwd: &Path, state: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .current_dir(cwd)
        .env("PII_RUNTIME_DIR", state)
        .env("PII_STATE_DIR", state)
        .env("PII_CONNECT_TIMEOUT_MS", "500")
        .output()
        .unwrap()
}

fn rc(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A healed attempt as the heal store would hold it after a green re-run: the patch and reproducer bytes are
/// carried in the attempt's `extra` bag, and `patch_hash` is the store's hash of the patch.
fn seed(state: &Path, patch: &str, gate_after: Option<&str>) -> HealStore {
    let store = HealStore::open(&state.join("heal.redb")).unwrap();
    let mut extra = serde_json::Map::new();
    extra.insert(String::from("treatment_patch"), patch.into());
    extra.insert(String::from("treatment_reproducer"), REPRO.into());
    store
        .put_attempt(&HealAttempt {
            signature_hash: SIG.to_owned(),
            gate: String::from("quality-gate"),
            state: HealState::Fixed,
            first_seen_ms: 1,
            last_seen_ms: 2,
            attempts: Vec::new(),
            attempt_count: 1,
            patch_hash: Some(pii_store::hash(patch.as_bytes())),
            gate_before: Some(String::from("fail")),
            gate_after: gate_after.map(String::from),
            decision: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra,
        })
        .unwrap();
    store
}

fn bundle_of(root: &Path) -> PathBuf {
    root.join("bundle")
}

/// Export the seeded attempt from `repo` into `<root>/bundle` and return that path.
fn export_bundle(repo: &Path, state: &Path, root: &Path) -> PathBuf {
    let out = bundle_of(root);
    let res = pii(
        &["treat", "export", SIG, "--out", out.to_str().unwrap()],
        repo,
        state,
    );
    assert_eq!(rc(&res), 0, "export failed: {}", stderr(&res));
    out
}

#[test]
fn export_refuses_a_fix_with_no_gate_after() {
    let root = scratch("treat-noproof");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    drop(seed(&state, PATCH, None));

    let out = pii(
        &[
            "treat",
            "export",
            SIG,
            "--out",
            root.path().join("b").to_str().unwrap(),
        ],
        &repo,
        &state,
    );
    assert_eq!(rc(&out), 2, "{}", stderr(&out));
    assert!(stderr(&out).contains("gate_after"), "{}", stderr(&out));
}

#[test]
fn CONTROL_export_writes_a_verifiable_bundle() {
    let root = scratch("treat-export");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    drop(seed(&state, PATCH, Some("pass")));

    let bundle = export_bundle(&repo, &state, root.path());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).unwrap())
            .unwrap();
    let sha = Command::new("sha256sum")
        .arg(bundle.join("fix.patch"))
        .output()
        .unwrap();
    let want = String::from_utf8_lossy(&sha.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(manifest["patch_sha256"], want.as_str());
    assert_eq!(manifest["signature"], SIG);
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["gate_after"], "pass");
    assert!(bundle.join("repro.sh").exists());
}

#[test]
fn a_tampered_patch_is_refused_and_named() {
    let root = scratch("treat-tamper");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    drop(seed(&state, PATCH, Some("pass")));
    let bundle = export_bundle(&repo, &state, root.path());

    // One byte flipped: `fixed` becomes `fixee`.
    let patch = std::fs::read_to_string(bundle.join("fix.patch")).unwrap();
    std::fs::write(bundle.join("fix.patch"), patch.replace("+fixed", "+fixee")).unwrap();

    let import_state = root.path().join("import-state");
    std::fs::create_dir_all(&import_state).unwrap();
    let out = pii(
        &["treat", "import", bundle.to_str().unwrap()],
        &repo,
        &import_state,
    );
    assert_eq!(rc(&out), 2, "{}", stderr(&out));
    assert!(stderr(&out).contains("patch_sha256"), "{}", stderr(&out));
    let store = HealStore::open(&import_state.join("heal.redb")).unwrap();
    assert!(store.get_attempt(SIG).unwrap().is_none());
}

#[test]
fn a_bundle_from_another_base_is_refused_with_both_revisions() {
    let root = scratch("treat-base");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let head = repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    drop(seed(&state, PATCH, Some("pass")));
    let bundle = export_bundle(&repo, &state, root.path());

    let foreign = "0123456789abcdef0123456789abcdef01234567";
    let manifest = std::fs::read_to_string(bundle.join("manifest.json")).unwrap();
    std::fs::write(
        bundle.join("manifest.json"),
        manifest.replace(&head, foreign),
    )
    .unwrap();

    let out = pii(
        &["treat", "import", bundle.to_str().unwrap()],
        &repo,
        &state,
    );
    assert_eq!(rc(&out), 2, "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains(foreign), "{err}");
    assert!(err.contains(&head), "{err}");
}

#[test]
fn an_imported_treatment_that_flips_its_reproducer_is_verified() {
    // A treatment carries the revision it was built on, and import verifies that revision exists HERE. A gate
    // export is `git archive` and holds no `.git`, so this test cannot make that claim in the export; it says so
    // loudly instead of passing vacuously or failing for the wrong reason (the same rule as lane_run_parity).
    if !Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.git")
        .exists()
    {
        // say it out loud through the std handle (the lint bans the raw print macros), then leave the claim
        // unproven HERE rather than passing vacuously or failing for the wrong reason
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "SKIP an_imported_treatment_that_flips_its_reproducer_is_verified: export tree has no .git"
        );
        return;
    }
    let root = scratch("treat-verify");
    let origin = root.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    repo_with(&origin, "broken\n");
    let state_a = root.path().join("state-a");
    std::fs::create_dir_all(&state_a).unwrap();
    drop(seed(&state_a, PATCH, Some("pass")));
    let bundle = export_bundle(&origin, &state_a, root.path());

    // The importing machine: the same base, a clean tree, and an operator's approval.
    let target = root.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    repo_with(&target, "broken\n");
    let state_b = root.path().join("state-b");
    std::fs::create_dir_all(&state_b).unwrap();
    let imp = pii(
        &["treat", "import", bundle.to_str().unwrap()],
        &target,
        &state_b,
    );
    assert_eq!(rc(&imp), 0, "{}", stderr(&imp));
    let store = HealStore::open(&state_b.join("heal.redb")).unwrap();
    store
        .record_decision(
            SIG,
            "quality-gate",
            ApprovalDecision {
                allow: true,
                approval_id: String::from("operator"),
                by: String::from("operator"),
                ts_ms: 3,
            },
        )
        .unwrap();
    drop(store);

    let app = pii(&["treat", "apply", SIG], &target, &state_b);
    assert_eq!(rc(&app), 0, "{}{}", stdout(&app), stderr(&app));
    assert_eq!(
        std::fs::read_to_string(target.join("app.txt")).unwrap(),
        "fixed\n"
    );
    let store = HealStore::open(&state_b.join("heal.redb")).unwrap();
    assert_eq!(
        store.get_attempt(SIG).unwrap().unwrap().state,
        HealState::Verified
    );
}

#[test]
fn CONTROL_a_treatment_that_does_not_flip_is_reverted() {
    let root = scratch("treat-revert");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    // A reproducer that is already green on this tree: the patch proves nothing here.
    let store = HealStore::open(&state.join("heal.redb")).unwrap();
    let mut extra = serde_json::Map::new();
    extra.insert(String::from("treatment_patch"), PATCH.into());
    extra.insert(
        String::from("treatment_reproducer"),
        "grep -q '^broken$' app.txt\n".into(),
    );
    store
        .put_attempt(&HealAttempt {
            signature_hash: SIG.to_owned(),
            gate: String::from("quality-gate"),
            state: HealState::Fixed,
            first_seen_ms: 1,
            last_seen_ms: 2,
            attempts: Vec::new(),
            attempt_count: 1,
            patch_hash: Some(pii_store::hash(PATCH.as_bytes())),
            gate_before: Some(String::from("pass")),
            gate_after: Some(String::from("pass")),
            decision: Some(ApprovalDecision {
                allow: true,
                approval_id: String::from("operator"),
                by: String::from("operator"),
                ts_ms: 3,
            }),
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra,
        })
        .unwrap();
    drop(store);

    let before = std::fs::read(repo.join("app.txt")).unwrap();
    let out = pii(&["treat", "apply", SIG], &repo, &state);
    assert_ne!(rc(&out), 0, "{}", stdout(&out));
    assert_eq!(std::fs::read(repo.join("app.txt")).unwrap(), before);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    let store = HealStore::open(&state.join("heal.redb")).unwrap();
    assert_eq!(
        store.get_attempt(SIG).unwrap().unwrap().state,
        HealState::NotATreatment
    );
}

#[test]
fn a_patch_that_does_not_apply_leaves_the_tree_untouched() {
    let root = scratch("treat-conflict");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // The patch expects `broken`; this tree says `other`.
    repo_with(&repo, "other\n");
    std::fs::write(repo.join("scratch.txt"), "untracked\n").unwrap();
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let store = HealStore::open(&state.join("heal.redb")).unwrap();
    let mut extra = serde_json::Map::new();
    extra.insert(String::from("treatment_patch"), PATCH.into());
    extra.insert(String::from("treatment_reproducer"), REPRO.into());
    store
        .put_attempt(&HealAttempt {
            signature_hash: SIG.to_owned(),
            gate: String::from("quality-gate"),
            state: HealState::Imported,
            first_seen_ms: 1,
            last_seen_ms: 2,
            attempts: Vec::new(),
            attempt_count: 0,
            patch_hash: Some(pii_store::hash(PATCH.as_bytes())),
            gate_before: None,
            gate_after: Some(String::from("pass")),
            decision: Some(ApprovalDecision {
                allow: true,
                approval_id: String::from("operator"),
                by: String::from("operator"),
                ts_ms: 3,
            }),
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra,
        })
        .unwrap();
    drop(store);

    let status = git(&repo, &["status", "--porcelain"]);
    let out = pii(&["treat", "apply", SIG], &repo, &state);
    assert_ne!(rc(&out), 0, "{}", stdout(&out));
    assert_eq!(git(&repo, &["status", "--porcelain"]), status);
    assert_eq!(
        std::fs::read_to_string(repo.join("app.txt")).unwrap(),
        "other\n"
    );
}

#[test]
fn export_is_deterministic() {
    let root = scratch("treat-det");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    repo_with(&repo, "broken\n");
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    drop(seed(&state, PATCH, Some("pass")));

    let first = export_bundle(&repo, &state, &root.path().join("one"));
    let second = export_bundle(&repo, &state, &root.path().join("two"));
    for file in ["manifest.json", "fix.patch", "repro.sh"] {
        assert_eq!(
            std::fs::read(first.join(file)).unwrap(),
            std::fs::read(second.join(file)).unwrap(),
            "{file} differs between two exports"
        );
    }
}
