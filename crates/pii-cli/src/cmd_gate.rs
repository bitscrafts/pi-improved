//! `pii gate run [--routine] [--dry-run]` (spec 438 R1-R3, R6): the gate's preflight. Before the first step it reads the
//! free space on the build volume and checks the tools the steps need, and refuses in one line when either is short.
//! The floor is written here once; `scripts/autonomy/cycle.sh` keeps its own check until this one runs first.

// reason: the preflight reads the wall clock for artefact ages and shells out to `du`, `df` and `sh`; it is a
// diagnostic and a runner of the gate steps, not a replayed component.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::cmd_disk::{df, du_bytes};

/// Free space the gate needs on the build volume, in GB: the coverage step writes 37 GB, the rest is room for the other
/// steps and the tree (spec 438 R1). Change it only with a new measurement, in the spec.
const DISK_FLOOR_GB: u64 = 60;

/// Exit code of a refused run: the preflight said no and no step ran (spec 438 R3). A failed gate exits 1.
const REFUSED: u8 = 75;

/// Tools the steps need that are not part of the toolchain (spec 438 R6).
const CARGO_TOOLS: &[&str] = &["cargo-llvm-cov"];

/// The gate's step list (spec 146), read for `--dry-run` and for the run.
const STEPS_FILE: &str = "gate-steps.txt";

/// `.cargo/config.toml` is where the C and C++ compilers are required (spec 35); the preflight asks it, not a list.
const CARGO_CONFIG: &str = ".cargo/config.toml";

/// What the preflight found: free space on the build volume, the biggest thing under `_tmp/`, and the tools that are
/// absent.
struct Preflight {
    free_gb: u64,
    mount: String,
    biggest: Option<(String, u64)>,
    missing: Vec<String>,
}

impl Preflight {
    fn disk_low(&self) -> bool {
        self.free_gb < DISK_FLOOR_GB
    }

    /// The one-line refusal, or `None` when the gate may start.
    fn refusal(&self) -> Option<String> {
        if !self.disk_low() && self.missing.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if self.disk_low() {
            let biggest = self.biggest.as_ref().map_or_else(String::new, |(p, gb)| {
                format!("; largest under _tmp/: {p} {gb} GB")
            });
            parts.push(format!(
                "refused for disk: {} GB free on {}, floor {DISK_FLOOR_GB} GB{biggest}",
                self.free_gb, self.mount
            ));
        }
        if !self.missing.is_empty() {
            parts.push(format!("missing tools: {}", self.missing.join(", ")));
        }
        Some(format!("pii gate: {}", parts.join("; ")))
    }
}

#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_gate(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut dry_run = false;
    let mut run = false;
    for arg in args {
        if arg == "run" {
            run = true;
        } else if arg == "--routine" {
            // the routine profile is the only one; the flag is accepted so the queue can name it
        } else if arg == "--dry-run" {
            dry_run = true;
        } else {
            run = false;
            break;
        }
    }
    if !run {
        warn!("usage: pii gate run [--routine] [--dry-run]");
        return ExitCode::from(2);
    }
    let repo = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            warn!("pii gate: cannot read the current directory: {e}");
            return ExitCode::from(2);
        }
    };
    let pf = preflight(&repo);
    println_line(&format!(
        "gate preflight: {} GB free on {} (floor {DISK_FLOOR_GB} GB)",
        pf.free_gb, pf.mount
    ));
    if let Some(refusal) = pf.refusal() {
        warn!("{refusal}");
        return ExitCode::from(REFUSED);
    }
    let steps = read_steps(&repo);
    if dry_run {
        for (label, _) in &steps {
            println_line(&format!("would run: {label}"));
        }
        return ExitCode::SUCCESS;
    }
    // Every step runs even after an earlier failure (gate-steps.txt header).
    let mut failed = 0_usize;
    for (label, cmd) in &steps {
        let ok = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&repo)
            .status()
            .is_ok_and(|s| s.success());
        println_line(&format!("{} {label}", if ok { "PASS" } else { "FAIL" }));
        if !ok {
            failed += 1;
        }
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        println_line(&format!("gate: {failed} of {} steps failed", steps.len()));
        ExitCode::FAILURE
    }
}

fn preflight(repo: &Path) -> Preflight {
    let (mount, free_mb) = df(repo).unwrap_or_else(|| ("?".to_owned(), 0));
    Preflight {
        free_gb: free_mb / 1024,
        mount,
        biggest: biggest_under_tmp(repo),
        missing: missing_tools(repo),
    }
}

/// The largest entry directly under `_tmp/`, in whole GB, named relative to the repository.
fn biggest_under_tmp(repo: &Path) -> Option<(String, u64)> {
    let rd = std::fs::read_dir(repo.join("_tmp")).ok()?;
    rd.flatten()
        .filter_map(|e| {
            let bytes = du_bytes(&e.path())?;
            Some((format!("_tmp/{}", e.file_name().to_string_lossy()), bytes))
        })
        .max_by_key(|(_, bytes)| *bytes)
        .map(|(name, bytes)| (name, bytes / 1_073_741_824))
}

/// The tools that are not on `PATH`, and `llvm-tools` when the active toolchain has no `llvm-cov`.
fn missing_tools(repo: &Path) -> Vec<String> {
    let mut missing: Vec<String> = CARGO_TOOLS
        .iter()
        .filter(|t| !on_path(t))
        .map(|t| (*t).to_owned())
        .collect();
    if !llvm_tools_present() {
        missing.push("llvm-tools".to_owned());
    }
    for compiler in required_compilers(repo) {
        if !on_path(&compiler) {
            missing.push(compiler);
        }
    }
    missing
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// `llvm-cov` ships in the toolchain's `lib/rustlib/<target>/bin` once the `llvm-tools` component is installed.
fn llvm_tools_present() -> bool {
    let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    std::fs::read_dir(sysroot.join("lib/rustlib"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|t| t.path().join("bin/llvm-cov").is_file())
}

/// The compilers `.cargo/config.toml` names in its `[env]` table (`CC`, `CXX`).
fn required_compilers(repo: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(repo.join(CARGO_CONFIG)).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            matches!(key.trim(), "CC" | "CXX").then(|| value.trim().trim_matches('"').to_owned())
        })
        .collect()
}

/// `(label, command)` for each step in `gate-steps.txt`; `#` lines and blank lines are not steps.
fn read_steps(repo: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(repo.join(STEPS_FILE)).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('|'))
        .map(|(label, cmd)| (label.to_owned(), cmd.to_owned()))
        .collect()
}

fn println_line(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}
