//! Spec 379: `pii docs {stale|verify} --book <user-admin|arch-dev> [--root <dir>]` keeps the manuals honest.
//!
//! `stale` prints one line per chapter whose stamp commit differs from HEAD, or whose documented paths changed since that
//! commit; `nothing-stale` when none does. `verify` proves each chapter's claims: every backticked path must exist and
//! every `pii <verb>` must be a verb the binary answers. Only a pass rewrites the book's stamp to HEAD (R4): a failure
//! leaves the old stamp, so the book stays visibly stale. The book's stamp is the `commit` on its `main.tex` title page.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: pii docs {stale|verify} --book <user-admin|arch-dev> [--root <dir>]";
const BOOKS: [&str; 2] = ["user-admin", "arch-dev"];
/// Path prefixes a chapter may name in backticks; a bare word with a slash is not a claim about the tree.
const PATH_PREFIXES: [&str; 6] = [
    "crates/", "graphs/", "specs/", "_docs/", "scripts/", "xtask/",
];
/// Every top-level verb the binary answers (main.rs dispatch); a chapter's `pii <verb>` must be one of these.
const VERBS: &[&str] = &[
    "--version",
    "-p",
    "approve",
    "backup",
    "cron",
    "daemon",
    "deploy-card",
    "disk",
    "docs",
    "graph",
    "heal",
    "health",
    "incidents",
    "index",
    "lane",
    "learn",
    "loop",
    "memory",
    "metrics",
    "mqtt",
    "net",
    "project",
    "queue",
    "restore",
    "run",
    "sandbox-exec",
    "skill",
    "soak",
    "sop",
    "store",
    "sys",
    "treat",
    "tui",
    "worker",
    "worker-exec",
];

/// What one chapter claims: the repository paths it names and the `pii <verb>` words it writes.
#[derive(Debug, Default, PartialEq, Eq)]
struct Claims {
    paths: Vec<String>,
    verbs: Vec<String>,
}

/// Reads every `\texttt{...}` span of a chapter. Escapes the manuals use (`\_`, `{}`) are undone first.
fn claims(tex: &str) -> Claims {
    let mut out = Claims::default();
    for span in tex.split("\\texttt{").skip(1) {
        let Some(end) = span.find('}') else { continue };
        let text = span[..end].replace("\\_", "_").replace("{}", "");
        if let Some(verb) = text.strip_prefix("pii ") {
            if let Some(word) = verb.split_whitespace().next() {
                out.verbs.push(word.to_owned());
            }
        } else if !text.contains(' ') && PATH_PREFIXES.iter().any(|p| text.starts_with(p)) {
            out.paths.push(text);
        }
    }
    out
}

/// The book's stamp: the hex after `commit \texttt{` on its title page.
fn stamp(main_tex: &str) -> Option<String> {
    let rest = main_tex.split("commit \\texttt{").nth(1)?;
    Some(rest.split('}').next()?.trim().to_owned())
}

/// Runs `git -C <root> <args>`, returning trimmed stdout when it succeeds.
#[allow(clippy::disallowed_methods)] // reason: a local git read of the manuals' stamp, not a spawned worker
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The chapter files of one book, sorted so the report is stable.
fn chapters(root: &Path, book: &str) -> Vec<PathBuf> {
    let dir = root.join(format!("_docs/manuals/src/{book}/src/chapters"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|x| x == "tex"));
    files.sort();
    files
}

/// Whether a chapter is stale: its stamp is not HEAD, or a path it documents changed since the stamp.
fn is_stale(root: &Path, stamp: &str, head: &str, paths: &[String]) -> bool {
    let Some(full) = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{stamp}^{{commit}}"),
        ],
    ) else {
        return true;
    };
    if full != head {
        return true;
    }
    if paths.is_empty() {
        return false;
    }
    let mut args = vec!["diff", "--name-only", stamp, "HEAD", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(root, &args).is_some_and(|changed| !changed.is_empty())
}

fn cmd_stale(root: &Path, book: &str) -> ExitCode {
    let main_tex =
        std::fs::read_to_string(root.join(format!("_docs/manuals/src/{book}/src/main.tex")));
    let Some(stamp) = main_tex.ok().as_deref().and_then(stamp) else {
        crate::warn!("docs: {book} has no `commit` stamp on its title page");
        return ExitCode::from(2);
    };
    let Some(head) = git(root, &["rev-parse", "HEAD"]) else {
        crate::warn!("docs: {} is not a git repository", root.display());
        return ExitCode::from(2);
    };
    let mut any = false;
    for chapter in chapters(root, book) {
        let tex = std::fs::read_to_string(&chapter).unwrap_or_default();
        let paths = claims(&tex).paths;
        if is_stale(root, &stamp, &head, &paths) {
            any = true;
            let name = chapter
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            crate::say!("{book} chapters/{name}");
        }
    }
    if !any {
        crate::say!("nothing-stale");
    }
    ExitCode::SUCCESS
}

/// Every claim a book's chapters make, checked against the tree and the verb list. Returns the failures, one line each.
fn failures(root: &Path, book: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for chapter in chapters(root, book) {
        let name = chapter
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let claims = claims(&std::fs::read_to_string(&chapter).unwrap_or_default());
        for path in claims.paths {
            if !root.join(&path).exists() {
                bad.push(format!("{book} chapters/{name}: no such path `{path}`"));
            }
        }
        for verb in claims.verbs {
            if !VERBS.contains(&verb.as_str()) {
                bad.push(format!("{book} chapters/{name}: no such verb `pii {verb}`"));
            }
        }
    }
    bad
}

fn cmd_verify(root: &Path, book: &str) -> ExitCode {
    let bad = failures(root, book);
    if !bad.is_empty() {
        for line in &bad {
            crate::warn!("docs: {line}");
        }
        return ExitCode::from(1);
    }
    let Some(head) = git(root, &["rev-parse", "HEAD"]) else {
        crate::warn!("docs: {} is not a git repository", root.display());
        return ExitCode::from(2);
    };
    let main = root.join(format!("_docs/manuals/src/{book}/src/main.tex"));
    let Ok(tex) = std::fs::read_to_string(&main) else {
        crate::warn!("docs: read {}", main.display());
        return ExitCode::from(2);
    };
    let Some(old) = stamp(&tex) else {
        crate::warn!("docs: {book} has no `commit` stamp on its title page");
        return ExitCode::from(2);
    };
    // R4: the stamp moves only here, after every claim has passed.
    let stamped = tex.replacen(
        &format!("commit \\texttt{{{old}}}"),
        &format!("commit \\texttt{{{head}}}"),
        1,
    );
    if let Err(e) = std::fs::write(&main, stamped) {
        crate::warn!("docs: write {}: {e}", main.display());
        return ExitCode::from(2);
    }
    crate::say!("verified {book} stamp={head}");
    ExitCode::SUCCESS
}

/// `pii docs {stale|verify} --book <book> [--root <dir>]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_docs(args: &mut std::env::ArgsOs) -> ExitCode {
    let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let mut sub = None;
    let mut book = None;
    let mut root = PathBuf::from(".");
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--book" => book = it.next().cloned(),
            "--root" => root = it.next().map_or_else(|| root.clone(), PathBuf::from),
            other if sub.is_none() => sub = Some(other.to_owned()),
            _ => {
                crate::warn!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(book) = book.filter(|b| BOOKS.contains(&b.as_str())) else {
        crate::warn!("{USAGE}");
        return ExitCode::from(2);
    };
    match sub.as_deref() {
        Some("stale") => cmd_stale(&root, &book),
        Some("verify") => cmd_verify(&root, &book),
        _ => {
            crate::warn!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{claims, stamp};

    #[test]
    fn claims_read_paths_and_verbs_and_skip_prose() {
        let c = claims(
            "a \\texttt{crates/x/y.rs} and \\texttt{pii graph lint} and \\texttt{-{}-yes} and \\texttt{a b}",
        );
        assert_eq!(c.paths, vec!["crates/x/y.rs".to_owned()]);
        assert_eq!(c.verbs, vec!["graph".to_owned()]);
    }

    #[test]
    fn stamp_reads_the_title_page_commit() {
        assert_eq!(
            stamp("\\date{Version 0.2.0 \\textperiodcentered{} commit \\texttt{4249d5b2fa85}}")
                .as_deref(),
            Some("4249d5b2fa85")
        );
    }
}
