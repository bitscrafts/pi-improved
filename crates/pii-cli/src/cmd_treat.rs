//! `pii treat export|import|apply|list` (spec 349): a treatment bundle carries one proven fix (the patch, its
//! reproducer, and the evidence) from one installation to another. The bundle is a directory: `manifest.json`,
//! `fix.patch`, `repro.sh`. Nothing here contacts a remote; the importing machine proves the fix again itself.

/// Output through the std handles, not the raw print macros the observability lint bans (spec 98). The macro form
/// keeps every call site's format string EXACTLY as it was — a mechanical rewrite that turned `println!` into a
/// function call silently stopped interpolating the braces, and a test caught it (2026-11-02).
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
    }};
}

/// One line on stderr, same seam. Every refusal in this module goes through here.
macro_rules! warn {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, $($arg)*);
    }};
}

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Output, Stdio};

use pii_store::heal::{HealAttempt, HealState, HealStore};
use sha2::{Digest as _, Sha256};

/// The bundle layout this binary reads and writes.
const FORMAT_VERSION: u64 = 1;
/// Keys in the attempt's `extra` bag that hold the treatment's bytes and provenance.
const PATCH_KEY: &str = "treatment_patch";
const REPRO_KEY: &str = "treatment_reproducer";
const BASE_KEY: &str = "treatment_base_revision";
const SOURCE_KEY: &str = "treatment_source";
const IMPORTED_KEY: &str = "treatment_imported_ms";
const BEFORE_KEY: &str = "treatment_apply_before";
const AFTER_KEY: &str = "treatment_apply_after";

/// `pii treat export|import|apply|list`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_treat(args: &mut std::env::ArgsOs) -> ExitCode {
    let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    match rest
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["export", sig, "--out", out] => export(sig, Path::new(out)),
        ["import", path] => import(Path::new(path)),
        ["apply", sig] => apply(sig, false),
        ["apply", sig, "--yes"] => apply(sig, true),
        ["list"] => list(),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    warn!("usage: pii treat export <signature> --out <dir>");
    warn!("       pii treat import <dir>");
    warn!("       pii treat apply <signature> [--yes]");
    warn!("       pii treat list");
    ExitCode::from(2)
}

/// The local heal store: `PII_STATE_DIR` first, then the daemon's default state dir.
fn open_store() -> Result<HealStore, ExitCode> {
    let state_dir = pii_core::env_var("PII_STATE_DIR").map_or_else(
        || pii_server::daemon::config::state_dir(&pii_core::env_var),
        std::path::PathBuf::from,
    );
    HealStore::open(&state_dir.join("heal.redb")).map_err(|e| {
        warn!("cannot open the heal store: {e}");
        ExitCode::from(1)
    })
}

/// Runs `git` in the current repository, feeding `input` on stdin when given.
// reason: the operator's own repository is the subject of `pii treat`; there is no replayed Ctx in this process.
#[allow(clippy::disallowed_methods)]
fn git(args: &[&str], input: Option<&[u8]>) -> Result<Output, String> {
    let mut child = Command::new("git")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(bytes)
            .map_err(|e| format!("cannot write to git: {e}"))?;
    }
    child
        .wait_with_output()
        .map_err(|e| format!("git failed: {e}"))
}

fn head() -> Result<String, String> {
    let out = git(&["rev-parse", "HEAD"], None)?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Whether `patch` applies to the current tree (`git apply --check`); the tree is never written.
fn applies_here(patch: &[u8]) -> Result<(), String> {
    let out = git(&["apply", "--check"], Some(patch))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Runs a reproducer in the current directory: its exit status is the verdict, its output the evidence.
// reason: the reproducer is the operator's own check of the operator's own tree, run as a CLI step.
#[allow(clippy::disallowed_methods)]
fn reproduce(script: &str) -> Result<(bool, String), String> {
    let out = Command::new("bash")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("cannot run the reproducer: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text))
}

fn verdict(passed: bool) -> String {
    String::from(if passed { "pass" } else { "fail" })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

// reason: a CLI stamp of wall-clock time; there is no replayed Ctx in this process.
#[allow(clippy::disallowed_methods)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn extra_text(attempt: &HealAttempt, key: &str) -> Option<String> {
    attempt
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn state_name(state: HealState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// `pii treat export <signature> --out <dir>`: refuses an unproven fix, writes a deterministic bundle.
fn export(sig: &str, out: &Path) -> ExitCode {
    let store = match open_store() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let attempt = match store.get_attempt(sig) {
        Ok(Some(a)) => a,
        Ok(None) => {
            warn!("refusing to export {sig}: no heal attempt is recorded for it");
            return ExitCode::from(2);
        }
        Err(e) => {
            warn!("cannot read the heal store: {e}");
            return ExitCode::from(1);
        }
    };
    let refuse = |field: &str| {
        warn!("refusing to export {sig}: no {field} (an unproven fix is not a treatment)");
        ExitCode::from(2)
    };
    let Some(patch_hash) = attempt.patch_hash.clone() else {
        return refuse("patch_hash");
    };
    let Some(gate_after) = attempt.gate_after.clone() else {
        return refuse("gate_after");
    };
    let Some(patch) = extra_text(&attempt, PATCH_KEY) else {
        return refuse("patch");
    };
    let Some(repro) = extra_text(&attempt, REPRO_KEY) else {
        return refuse("reproducer");
    };
    if pii_store::hash(patch.as_bytes()) != patch_hash {
        warn!("refusing to export {sig}: the stored patch does not match patch_hash");
        return ExitCode::from(2);
    }
    let base = match head() {
        Ok(rev) => rev,
        Err(e) => {
            warn!("cannot read HEAD: {e}");
            return ExitCode::from(1);
        }
    };
    // No timestamp of the export is written: `created_ms` is the attempt's own last sighting, so two exports of
    // one attempt are byte-identical.
    let manifest = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "signature": sig,
        "pii_version": env!("CARGO_PKG_VERSION"),
        "base_revision": base,
        "patch_sha256": sha256_hex(patch.as_bytes()),
        "repro_sha256": sha256_hex(repro.as_bytes()),
        "gate_before": attempt.gate_before,
        "gate_after": gate_after,
        "created_ms": attempt.last_seen_ms,
    });
    let written = std::fs::create_dir_all(out)
        .and_then(|()| std::fs::write(out.join("fix.patch"), &patch))
        .and_then(|()| std::fs::write(out.join("repro.sh"), &repro))
        .and_then(|()| {
            let text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
            std::fs::write(out.join("manifest.json"), format!("{text}\n"))
        });
    if let Err(e) = written {
        warn!("cannot write the bundle to {}: {e}", out.display());
        return ExitCode::from(1);
    }
    say!("exported {sig} to {}", out.display());
    ExitCode::SUCCESS
}

/// `pii treat import <dir>`: verifies every check before it records anything, and records `imported` only.
// reason: one verification sequence in the order the spec names it; splitting it would only scatter the order.
#[allow(clippy::too_many_lines)]
fn import(dir: &Path) -> ExitCode {
    let refuse = |why: String| {
        warn!("refusing to import {}: {why}", dir.display());
        ExitCode::from(2)
    };
    let manifest: serde_json::Value = match std::fs::read(dir.join("manifest.json"))
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => return refuse(format!("manifest.json is unreadable ({e})")),
    };
    let format = manifest
        .get("format_version")
        .and_then(serde_json::Value::as_u64);
    if format != Some(FORMAT_VERSION) {
        return refuse(format!(
            "format_version {format:?} is not one this binary knows (it reads {FORMAT_VERSION})"
        ));
    }
    let field = |key: &str| {
        manifest
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let (Some(sig), Some(base)) = (field("signature"), field("base_revision")) else {
        return refuse(String::from(
            "manifest.json lacks signature or base_revision",
        ));
    };
    let (Ok(patch), Ok(repro)) = (
        std::fs::read(dir.join("fix.patch")),
        std::fs::read(dir.join("repro.sh")),
    ) else {
        return refuse(String::from("fix.patch or repro.sh is missing"));
    };
    if field("patch_sha256").as_deref() != Some(sha256_hex(&patch).as_str()) {
        return refuse(String::from(
            "patch_sha256 does not match the bytes of fix.patch",
        ));
    }
    if field("repro_sha256").as_deref() != Some(sha256_hex(&repro).as_str()) {
        return refuse(String::from(
            "repro_sha256 does not match the bytes of repro.sh",
        ));
    }
    let head = match head() {
        Ok(rev) => rev,
        Err(e) => return refuse(format!("cannot read HEAD: {e}")),
    };
    let ancestor =
        git(&["merge-base", "--is-ancestor", &base, &head], None).is_ok_and(|o| o.status.success());
    if !ancestor {
        return refuse(format!(
            "base_revision {base} is not a commit in this repository (HEAD is {head})"
        ));
    }
    let (Ok(patch_text), Ok(repro_text)) = (String::from_utf8(patch), String::from_utf8(repro))
    else {
        return refuse(String::from("the bundle is not UTF-8 text"));
    };
    let store = match open_store() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let now = now_ms();
    let sig_str = sig.clone();
    let mut attempt = match store.get_attempt(&sig) {
        Ok(Some(a)) => a,
        Ok(None) => HealAttempt {
            signature_hash: sig,
            gate: String::from("treatment"),
            state: HealState::Imported,
            first_seen_ms: now,
            last_seen_ms: now,
            attempts: Vec::new(),
            attempt_count: 0,
            patch_hash: None,
            gate_before: None,
            gate_after: None,
            decision: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra: serde_json::Map::new(),
        },
        Err(e) => {
            warn!("cannot read the heal store: {e}");
            return ExitCode::from(1);
        }
    };
    attempt.state = HealState::Imported;
    attempt.patch_hash = Some(pii_store::hash(patch_text.as_bytes()));
    attempt
        .extra
        .insert(PATCH_KEY.to_owned(), patch_text.into());
    attempt
        .extra
        .insert(REPRO_KEY.to_owned(), repro_text.into());
    attempt
        .extra
        .insert(BASE_KEY.to_owned(), base.clone().into());
    attempt
        .extra
        .insert(SOURCE_KEY.to_owned(), dir.display().to_string().into());
    attempt.extra.insert(IMPORTED_KEY.to_owned(), now.into());
    if let Err(e) = store.put_attempt(&attempt) {
        warn!("cannot record the import: {e}");
        return ExitCode::from(1);
    }
    say!(
        "imported {sig_str} from {} (base {base}); state imported, not applied",
        dir.display()
    );
    ExitCode::SUCCESS
}

/// `pii treat apply <signature> [--yes]`: the only act that touches the tree. Checks, proves, and reverts.
// reason: one protocol (check, prove before, apply, prove after, or revert); its steps read in that order on purpose.
#[allow(clippy::too_many_lines)]
fn apply(sig: &str, yes: bool) -> ExitCode {
    let store = match open_store() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut attempt = match store.get_attempt(sig) {
        Ok(Some(a)) => a,
        Ok(None) => {
            warn!("refusing to apply {sig}: no treatment is recorded for it");
            return ExitCode::from(2);
        }
        Err(e) => {
            warn!("cannot read the heal store: {e}");
            return ExitCode::from(1);
        }
    };
    let (Some(patch), Some(repro)) = (
        extra_text(&attempt, PATCH_KEY),
        extra_text(&attempt, REPRO_KEY),
    ) else {
        warn!("refusing to apply {sig}: it carries no patch or no reproducer");
        return ExitCode::from(2);
    };
    // An allowing decision, or `--yes` for an incident this machine opened itself (it has its own run history).
    let allowed =
        attempt.decision.as_ref().is_some_and(|d| d.allow) || (yes && !attempt.attempts.is_empty());
    if !allowed {
        warn!(
            "refusing to apply {sig}: no allowing decision is recorded (--yes is only for an incident this machine opened)"
        );
        return ExitCode::from(2);
    }
    if let Err(stderr) = applies_here(patch.as_bytes()) {
        warn!(
            "refusing to apply {sig}: the patch does not apply cleanly; the tree is untouched\n{stderr}"
        );
        return ExitCode::from(1);
    }
    let before = match reproduce(&repro) {
        Ok(run) => run,
        Err(e) => {
            warn!("{e}");
            return ExitCode::from(1);
        }
    };
    match git(&["apply"], Some(patch.as_bytes())) {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            warn!(
                "git apply failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            return ExitCode::from(1);
        }
        Err(e) => {
            warn!("{e}");
            return ExitCode::from(1);
        }
    }
    let after = match reproduce(&repro) {
        Ok(run) => run,
        Err(e) => {
            warn!("{e}");
            return ExitCode::from(1);
        }
    };
    attempt.gate_before = Some(verdict(before.0));
    attempt.gate_after = Some(verdict(after.0));
    attempt
        .extra
        .insert(BEFORE_KEY.to_owned(), before.1.clone().into());
    attempt
        .extra
        .insert(AFTER_KEY.to_owned(), after.1.clone().into());
    if !before.0 && after.0 {
        attempt.state = HealState::Verified;
        if let Err(e) = store.put_attempt(&attempt) {
            warn!("cannot record the verdict: {e}");
            return ExitCode::from(1);
        }
        say!("verified {sig}: the reproducer failed before and passes after");
        return ExitCode::SUCCESS;
    }
    // The patch did not flip the reproducer here: take it back out, byte for byte.
    match git(&["apply", "-R"], Some(patch.as_bytes())) {
        Ok(out) if out.status.success() => {}
        _ => {
            warn!("could not revert the patch for {sig}; restore the tree by hand");
            return ExitCode::from(1);
        }
    }
    attempt.state = HealState::NotATreatment;
    if let Err(e) = store.put_attempt(&attempt) {
        warn!("cannot record the verdict: {e}");
        return ExitCode::from(1);
    }
    warn!(
        "{sig} is not a treatment here: the reproducer was {} before and {} after; the patch is reverted\n--- before\n{}--- after\n{}",
        verdict(before.0),
        verdict(after.0),
        before.1,
        after.1
    );
    ExitCode::from(1)
}

/// `pii treat list`: every treatment, with a fresh `--check` against this tree (never a remembered answer).
fn list() -> ExitCode {
    let store = match open_store() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let attempts = match store.list_attempts() {
        Ok(a) => a,
        Err(e) => {
            warn!("cannot read the heal store: {e}");
            return ExitCode::from(1);
        }
    };
    for a in attempts.iter().filter(|a| a.extra.contains_key(PATCH_KEY)) {
        let here = match extra_text(a, PATCH_KEY).map(|p| applies_here(p.as_bytes())) {
            Some(Ok(())) => "yes",
            _ => "no",
        };
        say!(
            "{} base={} state={} applies_here={here} gate_before={} gate_after={} patch={}",
            a.signature_hash,
            extra_text(a, BASE_KEY).unwrap_or_else(|| String::from("-")),
            state_name(a.state),
            a.gate_before.as_deref().unwrap_or("-"),
            a.gate_after.as_deref().unwrap_or("-"),
            a.patch_hash.as_deref().unwrap_or("-"),
        );
    }
    ExitCode::SUCCESS
}
