//! `xtask heal-collect <sig>` (S1.4): the ONLY way a fix leaves the sandbox.
//! Writes the worktree diff to `_tmp/heal-reviews/<sig>.patch` and prints
//! the seed's original line for human verification.
//!
//! R2 (spec 31): refuses (exit 1, no `.patch` written) a diff that touches a
//! protected path or net-removes a test/assert line — the worktree is a full
//! checkout, so without this check the cheapest "fix" is deleting the
//! assertion that proves the fix worked. The refusal is recorded in the
//! heal-collect ledger as `HealState::Escalated` (R3).

use std::path::Path;

use crate::heal_seed::SeedRecord;
use pii_store::heal::{ApprovalDecision, HealStore};

fn tmp_path(rel: &str) -> std::path::PathBuf {
    crate::workspace_root().join("_tmp").join(rel)
}

/// Runs `heal-collect <sig>` against the default (live) ledger/reviews/heal-db
/// paths.
///
/// # Errors
/// Unknown signature, empty patch, or write failure.
pub fn run(sig: &str) -> Result<(), String> {
    run_at(
        sig,
        &tmp_path("heal-seeds.json"),
        &tmp_path("heal-reviews"),
        &tmp_path("heal-collect.redb"),
    )
}

/// [`run`], parameterized over the seed ledger / reviews dir / heal-attempt
/// db paths (spec 34: lets promotion tests drive the exact same collection
/// logic against scratch paths under `_tmp/`, never the live ledger).
///
/// # Errors
/// Unknown signature, empty patch, or write failure.
#[allow(clippy::disallowed_methods)] // reason: heal-collect is dev tooling
// that shells git diff (same as main::run_cmd / rss_gate).
pub fn run_at(
    sig: &str,
    seeds_path: &Path,
    reviews_dir: &Path,
    heal_db_path: &Path,
) -> Result<(), String> {
    let ledger: Vec<SeedRecord> = serde_json::from_str(
        &std::fs::read_to_string(seeds_path).map_err(|e| format!("ledger: {e}"))?,
    )
    .map_err(|e| format!("ledger parse: {e}"))?;
    let seed = ledger
        .iter()
        .find(|s| s.sig == sig)
        .ok_or_else(|| format!("heal-collect: no seed with sig '{sig}'"))?;
    let wt = &seed.worktree;
    let dir = reviews_dir;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let patch_path = dir.join(format!("{sig}.patch"));

    // git -C <worktree> diff > _tmp/heal-reviews/<sig>.patch
    let out = std::process::Command::new("git")
        .args(["-C", wt, "diff"])
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    let patch = String::from_utf8_lossy(&out.stdout).into_owned();
    if patch.trim().is_empty() {
        // Revert-type repair: the seeded break was restored exactly, so the
        // worktree is pristine and there is nothing to collect — that IS the
        // successful outcome (Slice 1's seeded target is a practice defect).
        // Verify the restoration honestly before declaring success: the
        // seeded file must contain the recorded original line again.
        let full = Path::new(wt).join(&seed.broken_file);
        let restored = std::fs::read_to_string(&full).ok().is_some_and(|src| {
            usize::try_from(seed.broken_line.saturating_sub(1))
                .is_ok_and(|idx| src.lines().nth(idx) == Some(&seed.original_line))
        });
        if restored {
            let clean = std::process::Command::new("git")
                .args(["-C", wt, "status", "--porcelain"])
                .output()
                .is_ok_and(|o| o.stdout.is_empty());
            if clean {
                println!(
                    "heal-collect: repair RESTORED the seeded break exactly — worktree is pristine, no patch artifact (revert-type repair). Verified: {}:{} == recorded original.",
                    seed.broken_file, seed.broken_line
                );
                return Ok(());
            }
        }
        return Err(String::from(
            "heal-collect: worktree diff is empty and the seeded break was NOT cleanly restored (no fix?)",
        ));
    }
    if let Some(reason) = refusal_reason(&patch) {
        record_escalation(&seed.sig, &reason, heal_db_path);
        eprintln!("heal-collect: REFUSED — {reason}");
        return Err(format!("heal-collect: refused — {reason}"));
    }

    std::fs::write(&patch_path, &patch).map_err(|e| format!("write patch: {e}"))?;

    println!("heal-collect: patch={}", patch_path.display());
    println!("heal-collect: original_line={}", seed.original_line);
    println!(
        "heal-collect: seed sig={} worktree={} break={}:{}",
        seed.sig, seed.worktree, seed.broken_file, seed.broken_line
    );
    Ok(())
}

/// Promotion collect-on-allow (spec 34 R4): records `decision` in the heal
/// ledger REGARDLESS of `allow`/`deny` — the ledger must show a denied
/// promotion just as durably as an approved one — then, ONLY when
/// `decision.allow`, collects the patch via [`run_at`].
///
/// T3 (spec 34's acceptance test) is exactly the ordering this function
/// exists to enforce: a plausible-but-wrong implementation collects the
/// patch BEFORE the gate resolves. Here the ledger write happens first and
/// unconditionally, then a `deny` returns before `run_at` — and therefore
/// before `reviews_dir`/the worktree diff — is ever touched, so a denied
/// promotion can never leave a `.patch` artifact behind.
///
/// # Errors
/// The decision is `deny` (no patch written, by design — see above), or
/// [`run_at`]'s own errors (unknown signature, empty diff, refused diff) on
/// `allow`.
pub fn collect_with_decision(
    sig: &str,
    seeds_path: &Path,
    reviews_dir: &Path,
    heal_db_path: &Path,
    decision: &ApprovalDecision,
) -> Result<(), String> {
    record_decision_in_ledger(heal_db_path, sig, decision)?;
    if !decision.allow {
        return Err(format!(
            "heal-collect: promotion denied (approval {}) — no patch written for sig '{sig}'",
            decision.approval_id
        ));
    }
    run_at(sig, seeds_path, reviews_dir, heal_db_path)
}

/// [`collect_with_decision`], reading the decision from a real
/// [`pii_store::approval::ApprovalStore`] at `approval_db_path` — the CLI
/// entry point (`cargo xtask promote-collect`) so the recorded decision is
/// always the one the `gate` node's human approver actually made, bound to
/// `approval_id`, never re-typed by hand.
///
/// # Errors
/// The approval id is unknown or not yet decided, the store cannot be
/// opened, or [`collect_with_decision`]'s own errors.
pub fn collect_from_approval(
    sig: &str,
    approval_id: &str,
    approval_db_path: &Path,
    by: Option<&str>,
) -> Result<(), String> {
    let approvals = pii_store::approval::ApprovalStore::open(approval_db_path)
        .map_err(|e| format!("heal-collect: open approval db: {e}"))?;
    let approval = approvals
        .get_approval(approval_id)
        .map_err(|e| format!("heal-collect: read approval: {e}"))?
        .ok_or_else(|| format!("heal-collect: unknown approval '{approval_id}'"))?;
    let Some(allow) = approval.decision else {
        return Err(format!(
            "heal-collect: approval '{approval_id}' has not been decided yet"
        ));
    };
    let decision = ApprovalDecision {
        allow,
        approval_id: approval_id.to_owned(),
        by: by.unwrap_or("unknown").to_owned(),
        ts_ms: approval.decided_at_ms,
    };
    collect_with_decision(
        sig,
        &tmp_path("heal-seeds.json"),
        &tmp_path("heal-reviews"),
        &tmp_path("heal-collect.redb"),
        &decision,
    )
}

/// Records `decision` in the heal ledger at `heal_db_path` (R4).
fn record_decision_in_ledger(
    heal_db_path: &Path,
    sig: &str,
    decision: &ApprovalDecision,
) -> Result<(), String> {
    let store =
        HealStore::open(heal_db_path).map_err(|e| format!("heal-collect: open heal db: {e}"))?;
    store
        .record_decision(sig, "promotion", decision.clone())
        .map_err(|e| format!("heal-collect: record decision: {e}"))?;
    Ok(())
}

/// R2 (spec 31): does `path` (a `git diff`-relative path, e.g.
/// `specs/31-x.md`) fall under a protected prefix? Shared list with
/// `run_caps`'s `fs_write` deny wiring (`pii_core::PROTECTED_RELATIVE_PATHS`)
/// so the capability and the pre-write refusal never drift apart.
fn is_protected_path(path: &str) -> bool {
    pii_core::PROTECTED_RELATIVE_PATHS
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}

/// `true` if the (unprefixed) diff line content is a test/assertion marker
/// worth tracking for a net-removal (R2).
fn is_test_or_assert_line(content: &str) -> bool {
    content.contains("#[test]")
        || content.contains("assert!")
        || content.contains("assert_eq!")
        || content.contains("assert_ne!")
}

/// Scans a unified `git diff` for a protected-path touch or a net removal of
/// a test/assert line, or an added `#[ignore]` (R2). Returns the refusal
/// reason naming the offending file (and, for assertions, the removed line)
/// when the diff must be refused; `None` when it is clean.
fn refusal_reason(diff: &str) -> Option<String> {
    let mut current_path: Option<&str> = None;
    let mut protected_hit: Option<&str> = None;
    let mut assert_delta: i64 = 0;
    let mut removed_assert: Option<(&str, String)> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let path = rest.split(" b/").next().unwrap_or(rest);
            current_path = Some(path);
            if protected_hit.is_none() && is_protected_path(path) {
                protected_hit = Some(path);
            }
            continue;
        }
        let Some(path) = current_path else { continue };
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(content) = line.strip_prefix('-') {
            if is_test_or_assert_line(content) {
                assert_delta -= 1;
                if removed_assert.is_none() {
                    removed_assert = Some((path, content.trim().to_owned()));
                }
            }
        } else if let Some(content) = line.strip_prefix('+') {
            if content.contains("#[ignore") {
                return Some(format!("{path}: adds #[ignore] — {}", content.trim()));
            }
            if is_test_or_assert_line(content) {
                assert_delta += 1;
            }
        }
    }

    if let Some(path) = protected_hit {
        return Some(format!("{path}: touches a protected path"));
    }
    if assert_delta < 0
        && let Some((path, removed_line)) = removed_assert
    {
        return Some(format!(
            "{path}: net-removes a test/assert line — {removed_line}"
        ));
    }
    None
}

/// Records the refusal in the heal-collect ledger (R3): the attempt moves to
/// `HealState::Escalated` with `reason` in the run's `detail`. Best-effort —
/// a ledger failure must not hide the refusal itself (the exit-1 above is the
/// binding contract; this is bookkeeping on top of it).
#[allow(clippy::disallowed_methods)] // reason: heal-collect is dev tooling;
// SystemTime is the honest epoch-ms source (same precedent as heal-seed/
// heal-prune — no Ctx exists in this standalone xtask binary).
fn record_escalation(sig: &str, reason: &str, heal_db_path: &Path) {
    let Ok(store) = HealStore::open(heal_db_path) else {
        return;
    };
    let ts_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(0);
    let _ = store.record_run(
        sig,
        "heal-collect",
        pii_store::heal::HealAttemptRun {
            run_id: sig.to_owned(),
            outcome: pii_store::heal::HealState::Escalated,
            ts_ms,
            detail: reason.to_owned(),
        },
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{is_protected_path, refusal_reason};

    /// T2 (spec 31, TDD contract): a diff that removes one `assert_eq!` line
    /// must be refused, naming the file and the removed assertion.
    #[test]
    fn heal_collect_refuses_test_deletion() {
        let diff = "diff --git a/crates/pii-core/src/x.rs b/crates/pii-core/src/x.rs\n\
index 111..222 100644\n\
--- a/crates/pii-core/src/x.rs\n\
+++ b/crates/pii-core/src/x.rs\n\
@@ -10,7 +10,6 @@ fn foo() {\n\
     let x = 1;\n\
-    assert_eq!(x, 1);\n\
     let y = 2;\n\
 }\n";
        let reason = refusal_reason(diff).expect("must refuse a removed assert_eq!");
        assert!(reason.contains("crates/pii-core/src/x.rs"), "{reason}");
        assert!(reason.contains("assert_eq!(x, 1)"), "{reason}");
    }

    /// R2 acceptance: a diff touching `specs/` is refused even when it
    /// doesn't touch any test/assert line at all.
    #[test]
    fn heal_collect_refuses_protected_path_touch() {
        let diff = "diff --git a/specs/31-protected-paths.md b/specs/31-protected-paths.md\n\
index 111..222 100644\n\
--- a/specs/31-protected-paths.md\n\
+++ b/specs/31-protected-paths.md\n\
@@ -1 +1 @@\n\
-old\n\
+new\n";
        let reason = refusal_reason(diff).expect("must refuse a protected-path touch");
        assert!(reason.contains("specs/31-protected-paths.md"), "{reason}");
    }

    /// R2 acceptance: adding `#[ignore]` to a test is refused even though no
    /// line is net-removed.
    #[test]
    fn heal_collect_refuses_added_ignore() {
        let diff = "diff --git a/crates/pii-core/src/x.rs b/crates/pii-core/src/x.rs\n\
index 111..222 100644\n\
--- a/crates/pii-core/src/x.rs\n\
+++ b/crates/pii-core/src/x.rs\n\
@@ -1,3 +1,4 @@\n\
 #[test]\n\
+#[ignore]\n\
 fn t() {\n\
     assert!(true);\n\
 }\n";
        let reason = refusal_reason(diff).expect("must refuse an added #[ignore]");
        assert!(reason.contains("#[ignore]"), "{reason}");
    }

    /// An ordinary source diff that neither touches a protected path nor
    /// net-removes an assertion must be allowed through.
    #[test]
    fn heal_collect_allows_clean_source_diff() {
        let diff = "diff --git a/crates/pii-core/src/x.rs b/crates/pii-core/src/x.rs\n\
index 111..222 100644\n\
--- a/crates/pii-core/src/x.rs\n\
+++ b/crates/pii-core/src/x.rs\n\
@@ -1,3 +1,3 @@\n\
-fn foo() -> u32 { 1 }\n\
+fn foo() -> u32 { 2 }\n\
 fn bar() {}\n";
        assert!(refusal_reason(diff).is_none());
    }

    /// A diff that removes an `assert_eq!` and adds an equivalent one back
    /// (e.g. reformatted) is NOT a net removal.
    #[test]
    fn heal_collect_allows_reformatted_assert() {
        let diff = "diff --git a/crates/pii-core/src/x.rs b/crates/pii-core/src/x.rs\n\
index 111..222 100644\n\
--- a/crates/pii-core/src/x.rs\n\
+++ b/crates/pii-core/src/x.rs\n\
@@ -1,3 +1,3 @@\n\
-    assert_eq!(x,1);\n\
+    assert_eq!(x, 1);\n\
 fn bar() {}\n";
        assert!(refusal_reason(diff).is_none());
    }

    #[test]
    fn protected_path_prefix_matching() {
        assert!(is_protected_path("specs"));
        assert!(is_protected_path("specs/31-x.md"));
        assert!(is_protected_path("xtask/src/main.rs"));
        assert!(is_protected_path("quality-gate.sh"));
        assert!(is_protected_path(".github/workflows/ci.yml"));
        assert!(!is_protected_path("crates/pii-core/src/x.rs"));
        assert!(!is_protected_path("quality-gate.sh.bak"));
    }
}
