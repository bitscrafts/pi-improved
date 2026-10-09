//! Build identity for `sys.status` (spec 99 R3): the git revision the binary
//! was built from and the target triple, exported as compile-time env vars.
//! A build with no git checkout reports `unknown` rather than inventing one.

// reason: a build script talks to cargo over stdout by design.
#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::disallowed_methods
)]
// reason (disallowed_methods): a build script runs before any pii Ctx/config
// exists, so `Command` and `env::var` are the only available forms.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let rev = git(&["rev-parse", "--short=12", "HEAD"]).filter(|r| !r.is_empty());
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let revision = rev.map_or_else(
        || "unknown".to_owned(),
        |r| if dirty { format!("{r}-dirty") } else { r },
    );
    println!("cargo:rustc-env=PII_GIT_REV={revision}");
    println!(
        "cargo:rustc-env=PII_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=build.rs");
}
