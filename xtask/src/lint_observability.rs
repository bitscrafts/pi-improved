//! Spec 98: the observability seam and the ban on raw prints fail the gate.
//!
//! Rules: every crate depends on `pii-observe` and uses its contract in
//! `src/` (R1a/R1b) unless it carries a named exemption (R1c); no raw
//! `eprintln!`/`println!`/`dbg!`/`print!` outside the checked-in allowlist
//! (R1d); every `pub const` in the name registry is listed in `ALL` (R1e).
//! The lint fails closed: unreadable inputs are findings, never skips.

// reason: see lint_files.rs — binary-internal helpers; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CRATES: &str = "crates";
const ALLOWLIST: &str = "xtask/observability-allowlist.txt";
const NAMES: &str = "crates/pii-observe/src/names.rs";
const EXEMPT_MARK: &str = "// observability: none";
const CONTRACT_WORDS: &[&str] = &["Observer", "Observable", "Event", "Measure", "Health"];
const PRINT_MACROS: &[&str] = &["eprintln", "println", "dbg", "print"];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True when `word` occurs in `line` bounded by non-identifier characters.
fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let after = line[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// True when `src` uses the contract (a contract type or a `pii_observe::` path).
fn uses_contract(src: &str) -> bool {
    src.lines().any(|l| {
        let code = l.split("//").next().unwrap_or("");
        code.contains("pii_observe::") || CONTRACT_WORDS.iter().any(|w| has_word(code, w))
    })
}

/// Line numbers (1-based) holding a raw print macro; comment lines are skipped.
fn print_sites(src: &str) -> Vec<usize> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| {
            !l.trim_start().starts_with("//")
                && PRINT_MACROS.iter().any(|m| {
                    l.match_indices(m).any(|(i, _)| {
                        !l[..i].chars().next_back().is_some_and(is_ident)
                            && l[i + m.len()..].starts_with('!')
                    })
                })
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Parses the allowlist: `path:line  # reason`. Returns entries or one message
/// per malformed line (an entry without a reason is malformed).
fn parse_allowlist(text: &str) -> Result<Vec<String>, Vec<String>> {
    let (mut entries, mut errs) = (Vec::new(), Vec::new());
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (site, reason) = line.split_once('#').unwrap_or((line, ""));
        let site = site.trim();
        if reason.trim().is_empty() {
            errs.push(format!(
                "{ALLOWLIST}:{}: entry `{site}` has no `# reason`",
                i + 1
            ));
        } else if site
            .rsplit_once(':')
            .is_none_or(|(_, n)| n.parse::<usize>().is_err())
        {
            errs.push(format!(
                "{ALLOWLIST}:{}: `{site}` is not `path:line`",
                i + 1
            ));
        } else {
            entries.push(site.to_owned());
        }
    }
    if errs.is_empty() {
        Ok(entries)
    } else {
        Err(errs)
    }
}

/// Entries appearing more than once.
fn duplicates(entries: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut dups = BTreeSet::new();
    for e in entries {
        if !seen.insert(e.as_str()) {
            dups.insert(e.clone());
        }
    }
    dups.into_iter().collect()
}

/// Outcome of scanning a crate's entry files for `// observability: none`.
#[derive(Debug, PartialEq, Eq)]
enum Exemption {
    None,
    Reason(String),
    Invalid(String),
}

/// Parses exemption lines across the crate's entry-file sources.
fn parse_exemption(krate: &str, sources: &[&str]) -> Exemption {
    let marks: Vec<&str> = sources
        .iter()
        .flat_map(|s| s.lines())
        .map(str::trim)
        .filter(|l| l.starts_with(EXEMPT_MARK))
        .collect();
    match marks.as_slice() {
        [] => Exemption::None,
        [one] => {
            let reason = one[EXEMPT_MARK.len()..]
                .trim_start_matches(|c: char| c.is_whitespace() || c == '—' || c == '-')
                .trim();
            if reason.is_empty() {
                Exemption::Invalid(format!("crate {krate}: `{EXEMPT_MARK}` needs a reason"))
            } else {
                Exemption::Reason(reason.to_owned())
            }
        }
        _ => Exemption::Invalid(format!("crate {krate}: more than one `{EXEMPT_MARK}` line")),
    }
}

/// Checks the registry: every `pub const` (except `ALL`) is listed in `ALL`
/// and every `ALL` entry is a bare identifier defined in this module.
fn check_registry(path: &str, src: &str) -> Vec<String> {
    let mut consts: Vec<(usize, String)> = Vec::new();
    let (mut all, mut in_all, mut all_seen) = (Vec::new(), false, false);
    for (i, l) in src.lines().enumerate() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("pub const ") {
            let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
            if name == "ALL" {
                in_all = true;
                all_seen = true;
                continue;
            }
            consts.push((i + 1, name));
        } else if in_all {
            if t.starts_with("];") {
                in_all = false;
            } else if !t.is_empty() && !t.starts_with("//") {
                all.push((i + 1, t.trim_end_matches(',').trim().to_owned()));
            }
        }
    }
    let mut out = Vec::new();
    if !all_seen {
        out.push(format!("{path}:1: no `pub const ALL` registry found"));
    }
    for (line, name) in &consts {
        if !all.iter().any(|(_, a)| a == name) {
            out.push(format!("{path}:{line}: `{name}` is not listed in ALL"));
        }
    }
    for (line, a) in &all {
        if !consts.iter().any(|(_, c)| c == a) {
            out.push(format!(
                "{path}:{line}: ALL entry `{a}` is not a pub const of this module"
            ));
        }
    }
    out
}

/// True when `dir` contains at least one non-directory entry, at any depth.
fn holds_files(dir: &Path) -> bool {
    // Unreadable: report it as non-empty so the Cargo.toml read flags it.
    let Ok(rd) = fs::read_dir(dir) else {
        return true;
    };
    rd.flatten()
        .any(|e| !e.path().is_dir() || holds_files(&e.path()))
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>, errs: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errs.push(format!("{}:1: unreadable directory: {e}", dir.display()));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            rs_files(&p, out, errs);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

struct Tally {
    findings: Vec<String>,
    files: usize,
    exempt: Vec<(String, String)>,
    sites: usize,
}

fn scan(root: &Path) -> Tally {
    let mut t = Tally {
        findings: Vec::new(),
        files: 0,
        exempt: Vec::new(),
        sites: 0,
    };
    let allow = match fs::read_to_string(root.join(ALLOWLIST)) {
        Ok(text) => match parse_allowlist(&text) {
            Ok(e) => e,
            Err(errs) => {
                t.findings.extend(errs);
                Vec::new()
            }
        },
        Err(e) => {
            t.findings
                .push(format!("{ALLOWLIST}:1: allowlist unreadable: {e}"));
            Vec::new()
        }
    };
    for d in duplicates(&allow) {
        t.findings
            .push(format!("{ALLOWLIST}:1: duplicate allowlist entry `{d}`"));
    }
    let allow: BTreeSet<String> = allow.into_iter().collect();
    let mut crate_dirs: Vec<PathBuf> = match fs::read_dir(root.join(CRATES)) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => {
            t.findings.push(format!("{CRATES}:1: unreadable: {e}"));
            return t;
        }
    };
    crate_dirs.sort();
    for dir in crate_dirs {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('_') || name.starts_with('.') {
            continue; // scratch directories, not crates
        }
        if !holds_files(&dir) {
            continue; // stray empty directory tree: nothing to check
        }
        scan_crate(root, &dir, &name, &allow, &mut t);
    }
    match fs::read_to_string(root.join(NAMES)) {
        Ok(src) => t.findings.extend(check_registry(NAMES, &src)),
        Err(e) => t
            .findings
            .push(format!("{NAMES}:1: registry unreadable: {e}")),
    }
    t
}

fn scan_crate(root: &Path, dir: &Path, name: &str, allow: &BTreeSet<String>, t: &mut Tally) {
    let rel = format!("{CRATES}/{name}");
    let manifest = match fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(m) => m,
        Err(e) => {
            t.findings
                .push(format!("{rel}/Cargo.toml:1: unreadable: {e}"));
            return;
        }
    };
    let src_dir = dir.join("src");
    if !src_dir.is_dir() {
        t.findings
            .push(format!("{rel}/src:1: crate {name} has no src/"));
        return;
    }
    let mut files = Vec::new();
    rs_files(&src_dir, &mut files, &mut t.findings);
    let mut uses = false;
    let mut entry_srcs = Vec::new();
    for f in &files {
        t.files += 1;
        let shown = f.strip_prefix(root).unwrap_or(f).display().to_string();
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                t.findings.push(format!("{shown}:1: unreadable: {e}"));
                continue;
            }
        };
        uses |= uses_contract(&src);
        for line in print_sites(&src) {
            t.sites += 1;
            if !allow.contains(&format!("{shown}:{line}")) {
                t.findings.push(format!("{shown}:{line}: raw print macro (use the pii-observe seam, or allowlist with a reason)"));
            }
        }
        if f == &src_dir.join("lib.rs") || f == &src_dir.join("main.rs") {
            entry_srcs.push(src);
        }
    }
    let refs: Vec<&str> = entry_srcs.iter().map(String::as_str).collect();
    match parse_exemption(name, &refs) {
        Exemption::Reason(r) => {
            t.exempt.push((name.to_owned(), r));
            return;
        }
        Exemption::Invalid(m) => {
            t.findings.push(format!("{rel}/src:1: {m}"));
            return;
        }
        Exemption::None => {}
    }
    let has_dep = manifest.lines().any(|l| {
        let l = l.trim();
        l.strip_prefix("pii-observe")
            .is_some_and(|r| r.trim_start().starts_with(['=', '.']))
    });
    if !has_dep {
        t.findings.push(format!(
            "{rel}/Cargo.toml:1: crate {name} does not depend on pii-observe"
        ));
    }
    if !uses {
        t.findings.push(format!(
            "{rel}/src:1: crate {name} makes no use of the pii-observe contract"
        ));
    }
}

/// Runs the lint from the workspace root.
///
/// # Errors
/// One `path:line: message` per finding.
pub fn run() -> Result<(), Vec<String>> {
    let t = scan(Path::new("."));
    for (c, r) in &t.exempt {
        println!("lint-observability: exempt {c}: {r}");
    }
    if t.findings.is_empty() {
        println!(
            "lint-observability: {} files checked, {} print sites allowlisted, {} crates exempt, 0 findings",
            t.files,
            t.sites,
            t.exempt.len()
        );
        Ok(())
    } else {
        println!(
            "lint-observability: {} files checked, {} findings",
            t.files,
            t.findings.len()
        );
        Err(t.findings)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn allowlist_skips_comments_and_blank_lines() {
        let e = parse_allowlist("# header\n\na.rs:3  # why\nb.rs:9 # also why\n").unwrap();
        assert_eq!(e, vec!["a.rs:3", "b.rs:9"]);
    }

    #[test]
    fn allowlist_entry_without_reason_is_rejected() {
        assert_eq!(parse_allowlist("a.rs:3\n").unwrap_err().len(), 1);
        assert_eq!(parse_allowlist("a.rs:3  #   \n").unwrap_err().len(), 1);
    }

    #[test]
    fn allowlist_entry_must_be_path_line() {
        assert!(parse_allowlist("a.rs  # why\n").is_err());
        assert!(parse_allowlist("a.rs:x  # why\n").is_err());
    }

    #[test]
    fn duplicates_are_detected() {
        let v = vec!["a:1".to_owned(), "b:2".to_owned(), "a:1".to_owned()];
        assert_eq!(duplicates(&v), vec!["a:1"]);
        assert!(duplicates(&v[..2]).is_empty());
    }

    #[test]
    fn exemption_with_reason_is_accepted() {
        let s = "//! x\n// observability: none — pure data crate\n";
        assert_eq!(
            parse_exemption("c", &[s]),
            Exemption::Reason("pure data crate".into())
        );
    }

    #[test]
    fn bare_exemption_is_a_finding() {
        for s in [
            "// observability: none\n",
            "// observability: none — \n",
            "// observability: none -\n",
        ] {
            assert!(
                matches!(parse_exemption("c", &[s]), Exemption::Invalid(m) if m.contains("crate c")),
                "{s:?}"
            );
        }
    }

    #[test]
    fn absent_or_duplicate_exemption() {
        assert_eq!(parse_exemption("c", &["fn x() {}"]), Exemption::None);
        let two = "// observability: none — a\n// observability: none — b\n";
        assert!(matches!(
            parse_exemption("c", &[two]),
            Exemption::Invalid(_)
        ));
    }

    #[test]
    fn print_sites_found_and_comments_skipped() {
        let s = "fn f() {\n    eprintln!(\"x\");\n    // println!(\"y\");\n    let a = dbg!(1);\n    myprint!(2);\n    print!(\"z\");\n}\n";
        assert_eq!(print_sites(s), vec![2, 4, 6]);
    }

    #[test]
    fn contract_use_detected_by_word_not_substring() {
        assert!(uses_contract("use pii_observe::Event;"));
        assert!(uses_contract("impl Observer for X {}"));
        assert!(!uses_contract("struct EventLoop; // Observer"));
    }

    #[test]
    fn registry_requires_every_const_in_all() {
        let ok = "pub const A: &str = \"a\";\npub const ALL: &[&str] = &[\n    A,\n];\n";
        assert!(check_registry("n.rs", ok).is_empty());
        let missing = "pub const A: &str = \"a\";\npub const B: &str = \"a\";\npub const ALL: &[&str] = &[\n    A,\n];\n";
        let f = check_registry("n.rs", missing);
        assert_eq!(f.len(), 1);
        assert!(f[0].contains("`B`") && f[0].starts_with("n.rs:2:"));
    }

    #[test]
    fn registry_all_entry_must_be_local_const() {
        let s =
            "pub const A: &str = \"a\";\npub const ALL: &[&str] = &[\n    A,\n    other::X,\n];\n";
        assert_eq!(check_registry("n.rs", s).len(), 1);
    }
}
