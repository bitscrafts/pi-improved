//! Spec 379 R1: `pii docs stale --book <user-admin|arch-dev>` names each chapter whose stamp commit is behind HEAD
//! or whose documented paths changed since that commit. A tree where nothing moved answers `nothing-stale`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::process::Command;

/// Runs `git` in `dir` with a fixed identity, returning its trimmed stdout.
#[allow(clippy::disallowed_methods)] // reason: test fixture drives git to build a real history
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A book with one chapter that names `crates/app/src/lib.rs`, stamped at the first commit.
fn book(dir: &Path) -> String {
    git(dir, &["init", "-q"]);
    std::fs::create_dir_all(dir.join("crates/app/src")).unwrap();
    std::fs::write(dir.join("crates/app/src/lib.rs"), "pub fn a() {}\n").unwrap();
    let book = dir.join("_docs/manuals/src/user-admin/src");
    std::fs::create_dir_all(book.join("chapters")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "first"]);
    let stamp = git(dir, &["rev-parse", "--short=12", "HEAD"]);
    std::fs::write(
        book.join("main.tex"),
        format!("\\date{{Version 0.2.0 \\textperiodcentered{{}} build 2026-11-04 \\textperiodcentered{{}} commit \\texttt{{{stamp}}}}}\n"),
    )
    .unwrap();
    std::fs::write(
        book.join("chapters/01-app.tex"),
        "\\chapter{App}\nThe library is \\texttt{crates/app/src/lib.rs}.\n",
    )
    .unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "book"]);
    stamp
}

#[allow(clippy::disallowed_methods)] // reason: test fixture runs the real pii binary
fn stale(root: &Path, book: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["docs", "stale", "--book", book, "--root"])
        .arg(root)
        .output()
        .expect("pii runs")
}

#[test]
fn stale_inventory_names_a_chapter() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    book(dir);
    // Touch a path the chapter documents, after the stamp.
    std::fs::write(dir.join("crates/app/src/lib.rs"), "pub fn b() {}\n").unwrap();
    git(dir, &["commit", "-qam", "change"]);
    let out = stale(dir, "user-admin");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text.contains("01-app.tex"),
        "the stale chapter is named: {text}"
    );
}

#[test]
fn nothing_stale_prints_the_sentinel() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    book(dir);
    // Stamp the book at HEAD, uncommitted: the stamp is the commit the manual describes, and nothing has moved since.
    let head = git(dir, &["rev-parse", "--short=12", "HEAD"]);
    let main = dir.join("_docs/manuals/src/user-admin/src/main.tex");
    std::fs::write(
        &main,
        format!("\\date{{Version 0.2.0 \\textperiodcentered{{}} commit \\texttt{{{head}}}}}\n"),
    )
    .unwrap();
    let out = stale(dir, "user-admin");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "nothing-stale");
}

#[test]
fn unknown_book_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let out = stale(tmp.path(), "no-such-book");
    assert!(!out.status.success(), "an unknown book must exit non-zero");
}
