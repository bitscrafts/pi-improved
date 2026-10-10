//! Spec 98: the observability seam and the ban on raw prints fail the gate.
//!
//! Rules: every crate depends on `pii-observe` and uses its contract in
//! `src/` (R1a/R1b) unless it is in the checked-in exemption registry (R1c,
//! spec 102: pinned to a sanctioned set, and a crate with a real seam may not
//! stay listed); no file holds more raw `eprintln!`/`println!`/`dbg!`/`print!`
//! than its per-file budget in the checked-in allowlist (R1d); every `pub const` in the name registry is listed in `ALL` (R1e).
//! The lint fails closed: unreadable inputs are findings, never skips.

// reason: see lint_files.rs — binary-internal helpers; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::lint_obs_io::{io_finding, io_imports, module_of};

const CRATES: &str = "crates";
const ALLOWLIST: &str = "xtask/observability-allowlist.txt";
const NAMES: &str = "crates/pii-observe/src/names.rs";
const EXEMPTIONS: &str = "xtask/observability-exemptions.txt";
/// The only crates that may be exempt; a registry entry outside this set is a finding.
///
/// Spec 109 R8 shrank it from four: `pii-proto` (the async handshake client) and
/// `pii-uds` (the one client `UnixStream::connect`) own I/O failures and now emit
/// through the contract. `pii-abi` has no I/O; `pii-observe` IS the contract.
const SANCTIONED: &[&str] = &["pii-abi", "pii-observe"];

/// The contract crate itself: never required to depend on itself.
const CONTRACT_CRATE: &str = "pii-observe";
/// Reason used when the sanctioned set is honoured without a usable registry.
const SANCTIONED_REASON: &str = "sanctioned exemption (registry unusable or entry missing)";
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

/// True when `src` carries a real seam: a `pii_observe::` path in code.
fn has_seam(src: &str) -> bool {
    src.lines()
        .any(|l| l.split("//").next().unwrap_or("").contains("pii_observe::"))
}

/// Parses the budget allowlist: `path  # reason  # N`. Returns `(path, N)` or one
/// message per malformed line (no reason, non-numeric count, duplicate path).
fn parse_budgets(text: &str) -> Result<Vec<(String, usize)>, Vec<String>> {
    let (mut entries, mut errs): (Vec<(String, usize)>, Vec<String>) = (Vec::new(), Vec::new());
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = i + 1;
        let (head, count) = line.rsplit_once('#').unwrap_or((line, ""));
        let Ok(n) = count.trim().parse::<usize>() else {
            errs.push(format!(
                "{ALLOWLIST}:{at}: `{line}` is not `path  # reason  # N`"
            ));
            continue;
        };
        let (path, reason) = head.split_once('#').unwrap_or((head, ""));
        let path = path.trim();
        if reason.trim().is_empty() {
            errs.push(format!(
                "{ALLOWLIST}:{at}: entry `{path}` has no `# reason`"
            ));
        } else if entries.iter().any(|(p, _)| p == path) {
            errs.push(format!("{ALLOWLIST}:{at}: duplicate entry `{path}`"));
        } else {
            entries.push((path.to_owned(), n));
        }
    }
    if errs.is_empty() {
        Ok(entries)
    } else {
        Err(errs)
    }
}

/// Parses the exemption registry: `crate  # reason`. Returns `(crate, reason)` or
/// one message per bad line (no reason, unsanctioned crate, duplicate).
fn parse_exemptions(text: &str) -> Result<Vec<(String, String)>, Vec<String>> {
    let (mut entries, mut errs): (Vec<(String, String)>, Vec<String>) = (Vec::new(), Vec::new());
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = i + 1;
        let (name, reason) = line.split_once('#').unwrap_or((line, ""));
        let (name, reason) = (name.trim(), reason.trim());
        if reason.is_empty() {
            errs.push(format!("{EXEMPTIONS}:{at}: `{name}` has no `# reason`"));
        } else if !SANCTIONED.contains(&name.split_once(':').map_or(name, |(c, _)| c)) {
            errs.push(format!(
                "{EXEMPTIONS}:{at}: `{name}` is not in the sanctioned exemption set ({})",
                SANCTIONED.join(", ")
            ));
        } else if entries.iter().any(|(n, _)| n == name) {
            errs.push(format!("{EXEMPTIONS}:{at}: duplicate entry `{name}`"));
        } else {
            entries.push((name.to_owned(), reason.to_owned()));
        }
    }
    if errs.is_empty() {
        Ok(entries)
    } else {
        Err(errs)
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
    budget_total: usize,
    registry_size: usize,
    /// Module-scoped exemptions (`crate:module`) that covered a real I/O module.
    used_modules: BTreeSet<String>,
}

fn read_budgets(root: &Path, t: &mut Tally) -> BTreeMap<String, usize> {
    match fs::read_to_string(root.join(ALLOWLIST)) {
        Ok(text) => match parse_budgets(&text) {
            Ok(e) => e.into_iter().collect(),
            Err(errs) => {
                t.findings.extend(errs);
                BTreeMap::new()
            }
        },
        Err(e) => {
            t.findings
                .push(format!("{ALLOWLIST}:1: allowlist unreadable: {e}"));
            BTreeMap::new()
        }
    }
}

/// The exemption reason for `name`, or `None` when the crate must carry a real seam.
///
/// The sanctioned set is honoured even when the registry file is unusable: one bad line
/// (or a missing file) must never fabricate an R1a/R1b failure for every sanctioned crate.
fn exempt_reason(registry: &[(String, String)], name: &str) -> Option<String> {
    registry
        .iter()
        .find(|(c, _)| c == name)
        .map(|(_, r)| r.clone())
        .or_else(|| {
            SANCTIONED
                .contains(&name)
                .then(|| String::from(SANCTIONED_REASON))
        })
}

fn read_exemptions(root: &Path, t: &mut Tally) -> Vec<(String, String)> {
    match fs::read_to_string(root.join(EXEMPTIONS)) {
        Ok(text) => match parse_exemptions(&text) {
            Ok(e) => e,
            Err(errs) => {
                t.findings.extend(errs);
                Vec::new()
            }
        },
        Err(e) => {
            t.findings
                .push(format!("{EXEMPTIONS}:1: registry unreadable: {e}"));
            Vec::new()
        }
    }
}

fn scan(root: &Path) -> Tally {
    let mut t = Tally {
        findings: Vec::new(),
        files: 0,
        exempt: Vec::new(),
        sites: 0,
        budget_total: 0,
        registry_size: 0,
        used_modules: BTreeSet::new(),
    };
    let budgets = read_budgets(root, &mut t);
    t.budget_total = budgets.values().sum();
    let registry = read_exemptions(root, &mut t);
    t.registry_size = registry.len();
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
    let mut scanned = BTreeSet::new();
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
        let reason = exempt_reason(&registry, &name);
        scan_crate(
            root,
            &dir,
            &name,
            &budgets,
            reason.as_deref(),
            &registry,
            &mut t,
        );
        scanned.insert(name);
    }
    for (c, _) in &registry {
        let krate = c.split_once(':').map_or(c.as_str(), |(k, _)| k);
        if !scanned.contains(krate) {
            t.findings.push(format!(
                "{EXEMPTIONS}:1: exempt crate `{c}` not found under {CRATES}/"
            ));
        } else if c.contains(':') && !t.used_modules.contains(c) {
            t.findings.push(format!(
                "{EXEMPTIONS}:1: module entry `{c}` covers no module that imports I/O — remove it"
            ));
        }
    }
    match fs::read_to_string(root.join(NAMES)) {
        Ok(src) => t.findings.extend(check_registry(NAMES, &src)),
        Err(e) => t
            .findings
            .push(format!("{NAMES}:1: registry unreadable: {e}")),
    }
    t
}

/// R8: an exempt crate's module that imports I/O is a finding unless a
/// module-scoped registry entry covers it.
fn check_io_module(
    name: &str,
    shown: &str,
    src: &str,
    registry: &[(String, String)],
    t: &mut Tally,
) {
    let Some(line) = io_imports(src).first().copied() else {
        return;
    };
    let rel = shown.replace(&format!("{CRATES}/{name}/src/"), "");
    let module = module_of(&rel);
    let entry = format!("{name}:{module}");
    if registry.iter().any(|(c, _)| *c == entry) {
        t.used_modules.insert(entry);
    } else {
        t.findings.push(io_finding(name, &module, line));
    }
}

fn scan_crate(
    root: &Path,
    dir: &Path,
    name: &str,
    budgets: &BTreeMap<String, usize>,
    exempt: Option<&str>,
    registry: &[(String, String)],
    t: &mut Tally,
) {
    let rel = format!("{CRATES}/{name}");
    let manifest = match fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(m) => m,
        Err(e) => {
            t.findings
                .push(format!("{rel}/Cargo.toml:1: unreadable: {e}"));
            return;
        }
    };
    if !manifest.lines().any(|l| l.trim() == "[package]") {
        t.findings.push(format!(
            "{rel}/Cargo.toml:1: unparsable: no `[package]` table"
        ));
        return;
    }
    let src_dir = dir.join("src");
    if !src_dir.is_dir() {
        t.findings
            .push(format!("{rel}/src:1: crate {name} has no src/"));
        return;
    }
    let mut files = Vec::new();
    rs_files(&src_dir, &mut files, &mut t.findings);
    let (mut uses, mut seam) = (false, false);
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
        if exempt.is_some() && name != CONTRACT_CRATE {
            check_io_module(name, &shown, &src, registry, t);
        }
        uses |= uses_contract(&src);
        seam |= has_seam(&src);
        let sites = print_sites(&src);
        t.sites += sites.len();
        let budget = budgets.get(&shown).copied().unwrap_or(0);
        if sites.len() > budget {
            let lines: Vec<String> = sites.iter().map(usize::to_string).collect();
            t.findings.push(format!(
                "{shown}:{}: {} raw print macros exceed the budget of {budget} (use the pii-observe seam, or budget with a reason); sites at lines {}",
                sites.first().copied().unwrap_or(1),
                sites.len(),
                lines.join(",")
            ));
        }
    }
    if let Some(reason) = exempt {
        if seam {
            t.findings.push(format!(
                "{rel}/src:1: dead exemption: crate {name} is in {EXEMPTIONS} but uses `pii_observe::` — remove the exemption"
            ));
        }
        t.exempt.push((name.to_owned(), reason.to_owned()));
        return;
    }
    let has_dep = manifest.lines().any(|l| {
        let l = l.trim();
        l.strip_prefix("pii-observe")
            .is_some_and(|r| r.trim_start().starts_with(['=', '.']))
    });
    // A crate is never required to depend on ITSELF (the contract crate is the
    // dependency floor), and the sanctioned set is honoured even when the registry
    // file is unreadable or carries an invalid entry — otherwise one bad line would
    // fabricate an R1a/R1b failure for every sanctioned crate.
    let is_contract = name == CONTRACT_CRATE;
    if !has_dep && !is_contract {
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
            "lint-observability: {} files checked, {} raw prints within a per-file budget total of {}, registry size {} ({} crates exempt), 0 findings",
            t.files,
            t.sites,
            t.budget_total,
            t.registry_size,
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
    fn budget_skips_comments_and_blank_lines() {
        let e = parse_budgets("# header\n\na.rs  # why  # 3\nb.rs # also # 9\n").unwrap();
        assert_eq!(e, vec![("a.rs".to_owned(), 3), ("b.rs".to_owned(), 9)]);
    }

    #[test]
    fn budget_entry_without_reason_or_count_is_rejected() {
        for s in [
            "a.rs\n",
            "a.rs  # 3\n",
            "a.rs  #   # 3\n",
            "a.rs  # why\n",
            "a.rs  # why  # x\n",
        ] {
            assert_eq!(parse_budgets(s).unwrap_err().len(), 1, "{s:?}");
        }
    }

    #[test]
    fn budget_duplicates_are_detected() {
        let e = parse_budgets("a.rs # w # 1\na.rs # w # 2\n");
        assert!(e.unwrap_err()[0].contains("duplicate"));
    }

    #[test]
    fn registry_parses_crate_and_reason() {
        let e = parse_exemptions("pii-abi  # traits only\n\n# note\n").unwrap();
        assert_eq!(e, vec![("pii-abi".to_owned(), "traits only".to_owned())]);
    }

    #[test]
    fn registry_rejects_missing_reason_unknown_crate_and_fifth_entry() {
        assert!(parse_exemptions("pii-abi\n").is_err());
        assert!(parse_exemptions("pii-abi  #  \n").is_err());
        let unknown = parse_exemptions("pii-core  # nope\n").unwrap_err();
        assert!(unknown[0].contains("pii-core") && unknown[0].contains("sanctioned"));
        assert!(parse_exemptions("pii-abi # a\npii-abi # b\n").is_err());
    }

    #[test]
    fn dead_exemption_is_a_seam_use_not_a_contract_word() {
        assert!(has_seam("use pii_observe::Event;"));
        assert!(!has_seam("// pii_observe::Event\nstruct Event;"));
        assert!(!has_seam("fn f() {}"));
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

    #[test]
    fn sanctioned_set_is_honoured_without_a_usable_registry() {
        // An empty registry stands for "unreadable or invalid file".
        for c in SANCTIONED {
            let r = exempt_reason(&[], c);
            assert_eq!(
                r.as_deref(),
                Some(SANCTIONED_REASON),
                "sanctioned crate {c}"
            );
        }
        // A crate outside the sanctioned set is never exempt without a registry entry.
        assert!(exempt_reason(&[], "pii-core").is_none());
        // A registry entry wins over the fallback, so the reason stays honest.
        let reg = vec![(String::from("pii-abi"), String::from("types only"))];
        assert_eq!(
            exempt_reason(&reg, "pii-abi").as_deref(),
            Some("types only")
        );
        // pii-proto and pii-uds own I/O and are no longer sanctioned (spec 109 R8).
        assert!(exempt_reason(&[], "pii-proto").is_none());
        assert!(exempt_reason(&[], "pii-uds").is_none());
    }

    #[test]
    fn contract_crate_is_never_required_to_depend_on_itself() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/lint-obs-scan"));
        assert!(
            fs::create_dir_all(root.join("crates/pii-observe/src")).is_ok(),
            "create scratch tree"
        );
        assert!(
            fs::write(
                root.join("crates/pii-observe/Cargo.toml"),
                "[package]\nname = \"pii-observe\"\n",
            )
            .is_ok(),
            "write manifest"
        );
        assert!(
            fs::write(
                root.join("crates/pii-observe/src/lib.rs"),
                "//! contract crate\n"
            )
            .is_ok(),
            "write lib"
        );
        let t = scan(root);
        assert!(
            !t.findings
                .iter()
                .any(|f| f.contains("crate pii-observe does not depend on pii-observe")),
            "the contract crate must never be asked to depend on itself: {:?}",
            t.findings
        );
    }

    /// Builds a scratch workspace holding one crate `pii-abi` with `lib_rs`, and `exemptions`.
    fn io_tree(tag: &str, lib_rs: &str, exemptions: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp").join(tag);
        assert!(fs::create_dir_all(root.join("crates/pii-abi/src")).is_ok());
        assert!(fs::create_dir_all(root.join("xtask")).is_ok());
        let w = |p: &str, c: &str| assert!(fs::write(root.join(p), c).is_ok(), "write {p}");
        w(
            "crates/pii-abi/Cargo.toml",
            "[package]\nname = \"pii-abi\"\n",
        );
        w("crates/pii-abi/src/lib.rs", lib_rs);
        w("xtask/observability-exemptions.txt", exemptions);
        root
    }

    fn io_findings(t: &Tally) -> Vec<&String> {
        t.findings
            .iter()
            .filter(|f| f.contains("imports I/O") || f.contains("module entry"))
            .collect()
    }

    #[test]
    fn exempt_types_only_crate_is_accepted_near_miss() {
        let root = io_tree(
            "io-near-miss",
            "pub struct A;\npub const B: u8 = 1;\n",
            "pii-abi  # types only\n",
        );
        assert!(io_findings(&scan(&root)).is_empty());
    }

    #[test]
    fn exempt_crate_importing_tokio_turns_the_lint_red() {
        let root = io_tree(
            "io-red",
            "use tokio::net::UnixStream;\npub struct A;\n",
            "pii-abi  # types only\n",
        );
        let t = scan(&root);
        let f = io_findings(&t);
        assert_eq!(f.len(), 1, "{:?}", t.findings);
        assert!(
            f[0].contains("pii-abi") && f[0].contains("lib.rs:1"),
            "{}",
            f[0]
        );
    }

    #[test]
    fn module_scoped_entry_covers_a_dormant_io_module_and_stale_ones_are_flagged() {
        let covered = io_tree(
            "io-module-ok",
            "use std::io::Write;\n",
            "pii-abi  # types\npii-abi:lib  # dormant: sink trait, never driven here\n",
        );
        assert!(io_findings(&scan(&covered)).is_empty());
        let stale = io_tree(
            "io-module-stale",
            "pub struct A;\n",
            "pii-abi  # types\npii-abi:lib  # nothing dormant here\n",
        );
        let t = scan(&stale);
        let f = io_findings(&t);
        assert_eq!(f.len(), 1, "{:?}", t.findings);
        assert!(f[0].contains("pii-abi:lib"));
    }

    #[test]
    fn module_entry_for_an_unsanctioned_crate_is_rejected() {
        assert!(parse_exemptions("pii-proto:client  # no\n").is_err());
        assert!(parse_exemptions("pii-abi:lib  # ok\n").is_ok());
    }
}
