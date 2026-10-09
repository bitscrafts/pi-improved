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
    let status = Command::new("bash")
        .args([
            "-c",
            "set -o pipefail; git -C \"$1\" archive HEAD | tar -x -C \"$2\"",
            "_",
        ])
        .arg(root)
        .arg(dir)
        .status()
        .map_err(|e| format!("export: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "git archive failed".to_string())
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
    let mut child = Command::new("bash")
        .args(["-c", "bash quality-gate.sh 2>&1"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
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
    std::fs::create_dir(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    println!("fresh-gate: export dir {}", dir.display());
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
    Err(format!(
        "fresh-gate FAILED; failed steps: {steps:?}; failing tests: {tests:?}; export kept at {}",
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
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
