//! Library surface for `xtask`'s dev-tooling modules that need to be
//! reachable from another crate's tests without spawning a subprocess (spec
//! 34: `pii-server`'s promotion tests drive the exact same `heal-collect`
//! logic `cargo xtask heal-collect`/`promote-collect` runs in production,
//! against scratch paths under `_tmp/`, never the live ledger).
//!
//! `main.rs` also declares these as private `mod` items (dev-binary needs no
//! external visibility); both compile the same source files independently —
//! ordinary Rust package-with-lib-and-bin shape, not a duplicated file.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)]
// reason: spec 67 qualified the `std::println` disallowed-macro rule; these
// dev-tooling subcommands intentionally report their results on stdout like
// the rest of xtask (main.rs carries the same allow with the Phase-1 note).
// The clippy::print_stdout lint never fired on this code (it only covers
// literal-string macro heads), so this is the first time these prints are
// linted; an allow here documents the existing intent rather than hiding a
// new violation.
#![allow(clippy::disallowed_macros)]
// reason: same exemption as main.rs — this is dev tooling that reports
// progress/refusals on stdout/stderr, not the agent runtime.
#![allow(clippy::too_long_first_doc_paragraph)]
// reason: dev-tooling doc comments carry the full spec-34 context (paths,
// invariants, refusal semantics) in one paragraph; splitting them would
// hide the load-bearing detail.

/// Resolves the workspace root from `xtask`'s own manifest dir, so tooling
/// works from any checkout or `git archive` export path.
#[must_use]
pub fn workspace_root() -> std::path::PathBuf {
    // THE REPOSITORY YOU ARE STANDING IN IS THE ONE YOU MEAN. `CARGO_MANIFEST_DIR` is baked at COMPILE time, and a
    // shared `CARGO_TARGET_DIR` happily reuses a binary built in a different checkout: measured 2026-11-02, a
    // `fresh-gate --worktree` run from /projects/pi-improved reused an `xtask` built in /tmp/clone-head, archived
    // the CLONE's HEAD, and reported three findings that the working tree had already fixed — a gate that measured
    // a tree nobody was editing, and said so with a straight face.
    let baked = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let Ok(cwd) = std::env::current_dir() else {
        return baked;
    };
    let mut cur = cwd.as_path();
    loop {
        if cur.join("Cargo.toml").is_file() && cur.join("xtask").join("Cargo.toml").is_file() {
            if cur != baked {
                eprintln!(
                    "xtask: using the repository at {} (this binary was built at {}); set the same CARGO_TARGET_DIR \
                     for the repo you are in if that is a surprise",
                    cur.display(),
                    baked.display()
                );
            }
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return baked,
        }
    }
}

/// `cargo xtask <flags>`: CLI parsing shared by the bin and the lib's own
/// tests (`lint_files`' unit tests parse the `--report` flag through it).
#[cfg(test)]
pub mod cli_parse;
/// `cargo xtask crate-audit` and `lint-files`: the one physical line counter and per-file ratchet (spec 402).
#[cfg_attr(not(test), allow(unreachable_pub))]
pub mod file_size;
/// `cargo xtask heal-collect`/`promote-collect`: the ONLY way a repair patch
/// leaves a disposable worktree sandbox.
pub mod heal_collect;
/// `cargo xtask heal-seed`: seed record shape shared with `heal_collect`.
pub mod heal_seed;
/// `cargo xtask lint-files`: re-exported so `reachability` (spec 72 R2
/// reuses the effective-line skipping) compiles identically in the lib and
/// the bin.
#[cfg_attr(not(test), allow(unreachable_pub))]
pub mod lint_files;
/// `cargo xtask lint-scratch`: public so `tests/` can feed `check_source`
/// controls (spec 136 R5).
#[cfg_attr(not(test), allow(unreachable_pub))]
pub mod lint_scratch;
/// `cargo xtask promote-build`: build + dev-daemon soak step (spec 34 R2).
pub mod promote_build;
/// `cargo xtask reachability-check`: the declared-capability caller gate
/// (spec 72). Public so the integration tests in `tests/` can drive the
/// REAL check over scratch exports — the same `run` the binary and
/// `quality-gate.sh` execute.
pub mod reachability;
