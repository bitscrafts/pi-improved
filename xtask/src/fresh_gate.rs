//! `cargo xtask fresh-gate` (spec 84): the CI-equivalent run. Exports HEAD with
//! `git archive` into a unique `fresh-gate-<pid>` dir, optionally overlays the
//! working tree's uncommitted changes (`--worktree`) and single files
//! (`--overlay src=dest`), then runs `quality-gate.sh` there with an isolated
//! `CARGO_TARGET_DIR`. A leftover untracked dir in the dev tree can no longer
//! make the gate green. The export is kept on failure and removed on success.

// reason: dev tooling shells out to git/bash, like `run_cmd` in main.rs.
#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::workspace_root;

const BASE: &str = "/projects/_tmp";
const PREFIX: &str = "fresh-gate-";

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Opts {
    pub worktree: bool,
    pub overlays: Vec<(PathBuf, PathBuf)>,
}

/// Parses `[--worktree] [--overlay src=dest]...`; `dest` is relative to the
/// export root and may not escape it.
pub fn parse(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        worktree: false,
        overlays: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--worktree" => opts.worktree = true,
            "--overlay" => {
                let v = it.next().ok_or("--overlay needs src=dest")?;
                let (src, dest) = v.split_once('=').ok_or("--overlay needs src=dest")?;
                let dest = Path::new(dest);
                let escapes = dest.is_absolute()
                    || dest.components().any(|c| matches!(c, Component::ParentDir));
                if src.is_empty() || dest.as_os_str().is_empty() || escapes {
                    return Err(format!("bad --overlay `{v}`: dest must be a relative path"));
                }
                opts.overlays.push((PathBuf::from(src), dest.to_path_buf()));
            }
            other => return Err(format!("unknown fresh-gate argument `{other}`")),
        }
    }
    Ok(opts)
}

/// Failed gate steps (`FAIL: <step>`) and failing test names
/// (`test <name> ... FAILED`) found in the gate output.
pub fn summarize(output: &str) -> (Vec<String>, Vec<String>) {
    let mut steps = Vec::new();
    let mut tests = Vec::new();
    for line in output.lines() {
        if let Some(step) = line.strip_prefix("FAIL: ") {
            steps.push(step.trim().to_string());
        } else if let Some(name) = line
            .strip_prefix("test ")
            .and_then(|r| r.strip_suffix(" ... FAILED"))
        {
            tests.push(name.trim().to_string());
        }
    }
    (steps, tests)
}

/// Cleanup guard: true only for a non-empty `fresh-gate-*` directory name
/// directly under `base`. Anything else is refused (fail-safe).
pub fn safe_to_remove(base: &Path, dir: &Path) -> bool {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    !base.as_os_str().is_empty()
        && name.starts_with(PREFIX)
        && name.len() > PREFIX.len()
        && dir.parent() == Some(base)
}

/// Minimum free MB at the export base: `FRESH_GATE_MIN_FREE_MB`, default 12 288 (12 GiB).
///
/// Spec 110 (F7) measured why 2 048 was wrong: a gate run that MEASURES coverage builds an
/// instrumented tree inside the export (~7.8 GiB) on top of the plain one, so a coverage-measuring
/// export needs **>= 11.4 GiB** free — with 11 GB free it drove the volume to ZERO for ~3 minutes and
/// left an 11.4 GB failed export behind. The default now refuses before that can happen. The export
/// also skips the coverage step by default (`run_gate`), so the coverage build is not paid twice.
pub fn min_free_mb(env: Option<&str>) -> u64 {
    env.and_then(|v| v.trim().parse().ok()).unwrap_or(12_288)
}

/// Available MB from `df -Pm <path>` output (4th column of the data row).
pub fn parse_df_free_mb(out: &str) -> Option<u64> {
    out.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()
}

fn free_mb(base: &Path) -> Result<u64, String> {
    let out = Command::new("df")
        .args(["-Pm"])
        .arg(base)
        .output()
        .map_err(|e| format!("df: {e}"))?;
    parse_df_free_mb(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| format!("could not read free space of {}", base.display()))
}

/// Existing `fresh-gate-*` dirs under `base` other than `keep`.
pub fn stale_exports(base: &Path, keep: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p != keep && p.is_dir() && safe_to_remove(base, p))
        .collect()
}

fn git_files(root: &Path, args: &[&str]) -> Result<Vec<PathBuf>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {args:?} failed"));
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
        .collect())
}

fn copy_into(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    std::fs::copy(src, dest)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dest.display()))
}

/// `git archive HEAD | tar -x -C dir` — reads the repo, never touches the tree.
fn export_head(root: &Path, dir: &Path) -> Result<(), String> {
    // NAME THE REASON. This step died three times tonight with the bare message "git archive failed", and that message is
    // why 161 parked: the export works by hand (RC=0), so the failure is environmental - a missing destination, a git
    // ownership complaint, a lock - and the caller could not tell which. Capture git's and tar's own words and pass them
    // on; a step that can only say FAIL is the same defect as a node that emits no event.
    let out = Command::new("bash")
        .args([
            "-c",
            "set -o pipefail; git -C \"$1\" archive HEAD | tar -x -C \"$2\"",
            "_",
        ])
        .arg(root)
        .arg(dir)
        .output()
        .map_err(|e| format!("export: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.trim();
        return Err(format!(
            "git archive failed (rc={:?}, root={}, dir={}): {}",
            out.status.code(),
            root.display(),
            dir.display(),
            if why.is_empty() { "no stderr from git or tar" } else { why }
        ));
    }
    Ok(())
}

fn overlay_worktree(root: &Path, dir: &Path) -> Result<(), String> {
    // modified + untracked-not-ignored; deleted files are skipped (gone).
    for rel in git_files(root, &["ls-files", "-z", "-m", "-o", "--exclude-standard"])? {
        let src = root.join(&rel);
        if src.is_file() {
            copy_into(&src, &dir.join(&rel))?;
        }
    }
    Ok(())
}

/// Runs the gate in `dir`, streaming output while capturing it.
fn run_gate(dir: &Path) -> Result<(bool, String), String> {
    // The export's job is the hermetic-tree model, not a second measurement of the coverage floor the
    // repo gate already enforces — and that measurement costs ~7.8 GiB (spec 110 R5/F7). So the export
    // skips it BY DEFAULT and says so out loud; `FRESH_GATE_COVERAGE=1` measures it inside the export too.
    let covev = if std::env::var("FRESH_GATE_COVERAGE").ok().as_deref() == Some("1") {
        println!(
            "fresh-gate: MEASURING coverage inside the export (FRESH_GATE_COVERAGE=1; needs >= 11.4 GiB free)"
        );
        "0"
    } else {
        println!(
            "fresh-gate: the export skips the coverage step (it re-measures a floor the repo gate enforces \
             and needs ~7.8 GiB); set FRESH_GATE_COVERAGE=1 to measure it here too"
        );
        "1"
    };
    // The export's TREE must be fresh; its COMPILER OUTPUT does not have to be. Measured 2026-10-15: because
    // this line pins CARGO_TARGET_DIR to the export's own empty directory, every attempt paid a full cold
    // workspace build and overrode any target the caller had exported — a spec that took minutes on 2026-10-10
    // took 40-60 minutes after commit 4938209 introduced the export into the loop. FRESH_GATE_TARGET=<dir>
    // reuses one warm cache: the first run pays the cold build once and later runs resume incrementally. The
    // tree stays fresh either way, which is the guarantee the export exists for.
    let target = match std::env::var("FRESH_GATE_TARGET") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => dir.join("target"),
    };
    let mut child = Command::new("bash")
        .args(["-c", "bash quality-gate.sh 2>&1"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", &target)
        .env("QG_SKIP_COVERAGE", covev)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gate: {e}"))?;
    let mut captured = String::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            println!("{line}");
            captured.push_str(&line);
            captured.push('\n');
        }
    }
    let status = child.wait().map_err(|e| format!("gate wait: {e}"))?;
    Ok((status.success(), captured))
}

/// Entry point for `cargo xtask fresh-gate`.
pub fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: cargo xtask fresh-gate [--worktree] [--overlay src=dest]...");
        println!("fresh-tree gate: git-archive HEAD export + quality-gate.sh (CI-equivalent)");
        return Ok(());
    }
    let opts = parse(args)?;
    let root = workspace_root();
    let base = PathBuf::from(BASE);
    let dir = base.join(format!("{PREFIX}{}", std::process::id()));
    std::fs::create_dir_all(&base).map_err(|e| format!("mkdir {BASE}: {e}"))?;
    let free = free_mb(&base)?;
    let min = min_free_mb(std::env::var("FRESH_GATE_MIN_FREE_MB").ok().as_deref());
    if free < min {
        return Err(format!(
            "fresh-gate refused: only {free} MB free at {BASE}, need {min} MB \
             (override with FRESH_GATE_MIN_FREE_MB)"
        ));
    }
    // keep at most one failed export: drop older ones before creating this one.
    for old in stale_exports(&base, &dir) {
        if safe_to_remove(&base, &old) {
            std::fs::remove_dir_all(&old).map_err(|e| format!("retention: {e}"))?;
        }
    }
    std::fs::create_dir(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    println!("fresh-gate: export dir {} ({free} MB free)", dir.display());
    export_head(&root, &dir)?;
    if opts.worktree {
        overlay_worktree(&root, &dir)?;
    }
    for (src, dest) in &opts.overlays {
        copy_into(src, &dir.join(dest))?;
    }
    let (ok, output) = run_gate(&dir)?;
    if ok {
        if safe_to_remove(&base, &dir) {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("cleanup: {e}"))?;
        }
        println!("fresh-gate: PASS");
        return Ok(());
    }
    let (steps, tests) = summarize(&output);
    // A failing gate must leave its evidence where it can be read. Until 2026-10-10 the gate reported
    // only the failing test NAMES, so an intermittent test cost a reproduction hunt (four clean
    // full-suite rounds, two failures) before anyone could see the panic message. The capture is the
    // only place that message exists; write it and echo the failing block.
    let log = dir.join("gate-output.log");
    let _ = std::fs::write(&log, &output);
    let report = failure_report(&output);
    if !report.is_empty() {
        println!("--- failing test output (also in {})", log.display());
        println!("{report}");
    }
    Err(format!(
        "fresh-gate FAILED; failed steps: {steps:?}; failing tests: {tests:?}; full output at {}; export kept at {}",
        log.display(),
        dir.display()
    ))
}

/// The part of a gate run's output that says *why* a test failed: the `failures:` block (test names
/// plus their `panicked at` text), or the `---- <test> stdout ----` sections. Bounded, and empty when
/// the run failed without naming a test (a build error, a lint).
pub fn failure_report(output: &str) -> String {
    const MAX: usize = 60;
    let lines: Vec<&str> = output.lines().collect();
    let start = lines
        .iter()
        .position(|l| {
            l.trim_start().starts_with("failures:")
                || (l.starts_with("---- ") && l.ends_with(" stdout ----"))
        })
        .or_else(|| lines.iter().position(|l| l.contains("panicked at")));
    let Some(start) = start else {
        return String::new();
    };
    lines
        .iter()
        .skip(start)
        .take(MAX)
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn failure_report_shows_the_panic_and_stays_empty_without_one() {
        let out = "PASS: fmt\ntest a::b ... ok\ntest lanes::x ... FAILED\n\nfailures:\n\n---- lanes::x stdout ----\nthread 'lanes::x' panicked at crates/pii-server/src/daemon/mod.rs:1:1:\nrun r1 never finished\n";
        let r = failure_report(out);
        assert!(
            r.contains("---- lanes::x stdout ----"),
            "names the test: {r}"
        );
        assert!(r.contains("never finished"), "carries the panic text: {r}");
        // control: a run that failed without a test failure (build or lint) yields nothing to echo
        assert_eq!(
            failure_report("PASS: fmt\nerror: could not compile `xtask`\n"),
            ""
        );
    }

    #[test]
    fn parse_flags_and_overlay() {
        let o = parse(&s(&["--worktree", "--overlay", "/x/a.rs=crates/b.rs"])).unwrap();
        assert!(o.worktree);
        assert_eq!(
            o.overlays,
            vec![(PathBuf::from("/x/a.rs"), PathBuf::from("crates/b.rs"))]
        );
    }

    #[test]
    fn parse_rejects_escaping_or_malformed() {
        assert!(parse(&s(&["--overlay", "a=../b"])).is_err());
        assert!(parse(&s(&["--overlay", "a=/abs"])).is_err());
        assert!(parse(&s(&["--overlay", "noequals"])).is_err());
        assert!(parse(&s(&["--overlay"])).is_err());
        assert!(parse(&s(&["--bogus"])).is_err());
    }

    #[test]
    fn summarize_names_steps_and_tests() {
        let out =
            "PASS: fmt\ntest a::b ... ok\ntest lanes::x ... FAILED\nFAIL: test (lib/bins/tests)\n";
        let (steps, tests) = summarize(out);
        assert_eq!(steps, vec!["test (lib/bins/tests)"]);
        assert_eq!(tests, vec!["lanes::x"]);
    }

    #[test]
    fn df_parse_reads_available_column() {
        let out = "Filesystem 1048576-blocks Used Available Capacity Mounted on\n/dev/x 607697 512456 68418 89% /projects\n";
        assert_eq!(parse_df_free_mb(out), Some(68418));
        assert_eq!(parse_df_free_mb("garbage"), None);
        assert_eq!(parse_df_free_mb(""), None);
    }

    #[test]
    fn min_free_default_and_override() {
        assert_eq!(min_free_mb(None), 12_288);
        assert_eq!(min_free_mb(Some("999999")), 999_999);
        assert_eq!(min_free_mb(Some("junk")), 12_288);
    }

    #[test]
    fn stale_exports_excludes_current_and_foreign() {
        let t = tempfile_dir();
        for n in ["fresh-gate-1", "fresh-gate-2", "fresh-gate-9", "other"] {
            std::fs::create_dir(t.join(n)).unwrap();
        }
        let keep = t.join("fresh-gate-9");
        let mut got = stale_exports(&t, &keep);
        got.sort();
        assert_eq!(got, vec![t.join("fresh-gate-1"), t.join("fresh-gate-2")]);
        std::fs::remove_dir_all(&t).unwrap();
    }

    fn tempfile_dir() -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("_tmp")
            .join(format!("fg-stale-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn guard_refuses_unexpected_paths() {
        let base = Path::new("/projects/_tmp");
        assert!(safe_to_remove(base, &base.join("fresh-gate-42")));
        assert!(!safe_to_remove(base, &base.join("fresh-gate-")));
        assert!(!safe_to_remove(base, &base.join("other")));
        assert!(!safe_to_remove(base, Path::new("/projects/fresh-gate-42")));
        assert!(!safe_to_remove(base, Path::new("")));
        assert!(!safe_to_remove(Path::new(""), Path::new("fresh-gate-1")));
    }
}
