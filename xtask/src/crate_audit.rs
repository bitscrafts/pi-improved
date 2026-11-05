//! Spec 387: `cargo xtask crate-audit` measures every crate against the 450-line budget. `--ratchet` refuses any crate
//! whose over-budget count grew past its checked-in baseline; debt may fall and may not rise. The baseline moves only
//! through `--update-baseline`, which writes it on first creation and afterwards refuses any raise.
//!
//! The count is physical lines (`wc -l`) over every `.rs` file of the crate, `src/` and `tests/` alike, `target/`
//! skipped. That is stricter than `lint-files` (effective lines, `src/` only): a comment-heavy file can pass that gate
//! and still be counted here. The gap is deliberate, and the spec's `// filler` fixtures are comment lines.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use xtask::workspace_root;

const BUDGET: usize = 450;
const BASELINE: &str = "_docs/05-operations/crate-audit-baseline.tsv";

/// One crate's measurement. `over` names every file over the budget with its physical line count.
struct Row {
    name: String,
    src_files: usize,
    src_loc: usize,
    largest: (String, usize),
    test_files: usize,
    tests: usize,
    over: Vec<(String, usize)>,
}

/// Entry point for `cargo xtask crate-audit [--ratchet | --update-baseline]`.
pub fn run(args: &[String]) -> Result<(), Vec<String>> {
    let mut ratchet = false;
    let mut update = false;
    for arg in args {
        match arg.as_str() {
            "--ratchet" => ratchet = true,
            "--update-baseline" => update = true,
            other => return Err(vec![format!("crate-audit: unknown arg '{other}'")]),
        }
    }
    let root = workspace_root();
    let rows = measure_all(&root).map_err(|e| vec![e])?;
    print_report(&rows);
    if update {
        update_baseline(&root, &rows)
    } else if ratchet {
        check_ratchet(&root, &rows)
    } else {
        Ok(())
    }
}

fn measure_all(root: &Path) -> Result<Vec<Row>, String> {
    let mut names: Vec<String> = read_dir(&root.join("crates"))?
        .into_iter()
        .filter(|p| p.join("Cargo.toml").is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names.iter().map(|name| measure(root, name)).collect()
}

fn measure(root: &Path, name: &str) -> Result<Row, String> {
    let dir = root.join("crates").join(name);
    let mut files = Vec::new();
    rs_files(&dir, &mut files)?;
    files.sort();
    let mut row = Row {
        name: name.to_owned(),
        src_files: 0,
        src_loc: 0,
        largest: (String::from("<none>"), 0),
        test_files: 0,
        tests: 0,
        over: Vec::new(),
    };
    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("crate-audit: cannot read {}: {e}", path.display()))?;
        let lines = text.lines().count();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        row.tests += text
            .lines()
            .filter(|l| l.trim_start().starts_with("#[test]"))
            .count();
        if lines > BUDGET {
            row.over.push((rel.clone(), lines));
        }
        match path.strip_prefix(&dir).ok().and_then(|p| p.iter().next()) {
            Some(top) if top == "src" => {
                row.src_files += 1;
                row.src_loc += lines;
                if lines > row.largest.1 {
                    row.largest = (rel, lines);
                }
            }
            Some(top) if top == "tests" => row.test_files += 1,
            _ => {}
        }
    }
    Ok(row)
}

/// Every `.rs` file under `dir`, recursively, with `target/` skipped. An unreadable directory is an error, not a skip.
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for path in read_dir(dir)? {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rs_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn read_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("crate-audit: cannot read {}: {e}", dir.display()))?;
    entries
        .map(|e| {
            e.map(|e| e.path())
                .map_err(|e| format!("crate-audit: cannot list {}: {e}", dir.display()))
        })
        .collect()
}

fn print_report(rows: &[Row]) {
    println!(
        "{:<12} {:>4} {:>8} {:>6} {:>7} {:>5}  largest src file",
        "crate", "src", "src_loc", "tests", "#[test]", "over"
    );
    for r in rows {
        println!(
            "{:<12} {:>4} {:>8} {:>6} {:>7} {:>5}  {} ({})",
            r.name,
            r.src_files,
            r.src_loc,
            r.test_files,
            r.tests,
            r.over.len(),
            r.largest.0,
            r.largest.1
        );
    }
    let sum = |f: fn(&Row) -> usize| rows.iter().map(f).sum::<usize>();
    println!(
        "{:<12} {:>4} {:>8} {:>6} {:>7} {:>5}",
        "total",
        sum(|r| r.src_files),
        sum(|r| r.src_loc),
        sum(|r| r.test_files),
        sum(|r| r.tests),
        sum(|r| r.over.len())
    );
    let mut over: Vec<&Row> = rows.iter().filter(|r| !r.over.is_empty()).collect();
    if over.is_empty() {
        println!("no file over {BUDGET} lines");
        return;
    }
    over.sort_by(|a, b| {
        b.over
            .len()
            .cmp(&a.over.len())
            .then_with(|| a.name.cmp(&b.name))
    });
    println!("over the {BUDGET}-line budget, worst crate first:");
    for r in over {
        for (path, lines) in &r.over {
            println!("  {:<12} {path} ({lines})", r.name);
        }
    }
}

fn read_baseline(root: &Path) -> Result<BTreeMap<String, usize>, String> {
    let path = root.join(BASELINE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "crate-audit: no baseline at {BASELINE} ({e}); create it with `crate-audit --update-baseline`"
        )
    })?;
    let mut map = BTreeMap::new();
    for line in text.lines().skip(1) {
        let (name, count) = line
            .split_once('\t')
            .ok_or_else(|| format!("crate-audit: malformed baseline row {line:?}"))?;
        let count = count
            .trim()
            .parse::<usize>()
            .map_err(|e| format!("crate-audit: malformed baseline count in {line:?}: {e}"))?;
        map.insert(name.trim().to_owned(), count);
    }
    Ok(map)
}

/// Crates whose over-budget count exceeds their baseline. A crate with no baseline row is held to zero.
fn grown(rows: &[Row], baseline: &BTreeMap<String, usize>) -> Vec<String> {
    rows.iter()
        .filter_map(|r| {
            let base = baseline.get(&r.name).copied().unwrap_or(0);
            (r.over.len() > base).then(|| {
                let files: Vec<String> = r
                    .over
                    .iter()
                    .map(|(p, l)| format!("{p} ({l} lines)"))
                    .collect();
                format!(
                    "crate {}: {} over-budget file(s) against a baseline of {base}: {}",
                    r.name,
                    r.over.len(),
                    files.join(", ")
                )
            })
        })
        .collect()
}

fn check_ratchet(root: &Path, rows: &[Row]) -> Result<(), Vec<String>> {
    let baseline = read_baseline(root).map_err(|e| vec![e])?;
    let grown = grown(rows, &baseline);
    if grown.is_empty() {
        println!("crate-audit: ratchet holds, no crate above its baseline");
        Ok(())
    } else {
        Err(grown)
    }
}

fn update_baseline(root: &Path, rows: &[Row]) -> Result<(), Vec<String>> {
    let path = root.join(BASELINE);
    if path.is_file() {
        let baseline = read_baseline(root).map_err(|e| vec![e])?;
        let raised = grown(rows, &baseline);
        if !raised.is_empty() {
            return Err(raised
                .into_iter()
                .map(|g| format!("refused, the audit never raises a baseline: {g}"))
                .collect());
        }
    }
    write_baseline(&path, rows).map_err(|e| vec![e])?;
    println!("crate-audit: wrote {BASELINE} ({} crates)", rows.len());
    Ok(())
}

fn write_baseline(path: &Path, rows: &[Row]) -> Result<(), String> {
    let mut body = String::from("crate\tover_budget\n");
    for r in rows {
        let _ = writeln!(body, "{}\t{}", r.name, r.over.len());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("crate-audit: cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, body)
        .map_err(|e| format!("crate-audit: cannot write {}: {e}", path.display()))
}
