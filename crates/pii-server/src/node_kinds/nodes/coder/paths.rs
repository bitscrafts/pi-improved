//! Where a coder node's agent binaries live, and which programs a run may exec (spec 207, 209, 215, 223).
//! Split out of `coder.rs` to keep it under the effective-line limit; the functions are re-exported there.

use std::path::{Path, PathBuf};

use super::super::criteria::{self, Table};

/// The agent binary directory: the first `PATH` entry holding both `pi` and `claude`.
///
/// Resolved at runtime, never a literal, so the spawn and the `proc` allowlist (`run_scope`) always agree.
/// `None` when no entry holds both: the caller must refuse rather than guess a directory.
#[must_use]
pub fn agent_bin_dir() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .find(|dir| dir.join("pi").is_file() && dir.join("claude").is_file())
}

/// The agent program paths under `bin_dir`: the only programs a `coder` node may exec (spec 207).
#[must_use]
pub fn agent_binaries(bin_dir: &Path) -> Vec<String> {
    ["pi", "claude"]
        .iter()
        .map(|agent| bin_dir.join(agent).display().to_string())
        .collect()
}

/// The programs a run's scope admits (spec 207, 209, 215, 223): `/bin/sh`, the agent binaries, podman and the
/// criteria table's programs.
///
/// Dispatch hands each of them to a `coder` node only, and a node names only the one it execs.
#[must_use]
pub fn run_programs(bin_dir: &Path, podman: &Path, criteria: &Table) -> Vec<String> {
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin_dir));
    argv0.push(podman.display().to_string());
    argv0.extend(criteria::programs(criteria));
    argv0
}
