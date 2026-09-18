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

/// `cargo xtask heal-collect`/`promote-collect`: the ONLY way a repair patch
/// leaves a disposable worktree sandbox.
pub mod heal_collect;
/// `cargo xtask heal-seed`: seed record shape shared with `heal_collect`.
pub mod heal_seed;
/// `cargo xtask promote-build`: build + dev-daemon soak step (spec 34 R2).
pub mod promote_build;
