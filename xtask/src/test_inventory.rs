//! Spec 355 R4: the number of test binaries and test functions in the workspace, checked against a committed
//! baseline. `--check` fails when either count drops below it, so a large deletion cannot pass the gate unnoticed.
//! A test function is a `#[test]` or `#[tokio::test]` attribute line in `crates/*` or `xtask`, `src` and `tests`
//! alike; a test binary is a top-level `.rs` file in one of those `tests` directories.

use std::path::{Path, PathBuf};

/// The committed floor, one `key=value` per line. Raise it when tests are added; lowering it is a reviewed decision.
pub const BASELINE: &str = "xtask/test-inventory.baseline";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub binaries: usize,
    pub functions: usize,
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Counts the workspace's test binaries and test functions under `root`.
pub fn count(root: &Path) -> std::io::Result<Inventory> {
    let mut packages = vec![root.join("xtask")];
    for entry in std::fs::read_dir(root.join("crates"))? {
        let path = entry?.path();
        if path.is_dir() {
            packages.push(path);
        }
    }
    let mut inv = Inventory::default();
    for package in packages {
        for sub in ["src", "tests"] {
            let dir = package.join(sub);
            if !dir.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            collect(&dir, &mut files)?;
            for file in files {
                if sub == "tests" && file.parent() == Some(dir.as_path()) {
                    inv.binaries += 1;
                }
                let text = std::fs::read_to_string(&file)?;
                inv.functions += text
                    .lines()
                    .map(str::trim_start)
                    .filter(|l| l.starts_with("#[test") || l.starts_with("#[tokio::test"))
                    .count();
            }
        }
    }
    Ok(inv)
}

/// Reads the committed floor. A missing or malformed baseline is a violation, not a silent pass.
fn read_baseline(root: &Path) -> Result<Inventory, Vec<String>> {
    let path = root.join(BASELINE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        vec![format!(
            "no test-inventory baseline at {}: {e}",
            path.display()
        )]
    })?;
    let mut base = Inventory::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("binaries", n)) => {
                base.binaries = n.trim().parse().map_err(|_| vec![bad(line)])?;
            }
            Some(("functions", n)) => {
                base.functions = n.trim().parse().map_err(|_| vec![bad(line)])?;
            }
            _ => return Err(vec![bad(line)]),
        }
    }
    Ok(base)
}

fn bad(line: &str) -> String {
    format!("malformed test-inventory baseline line: {line:?}")
}

/// Prints the counts; with `check`, fails when either count is below the committed baseline.
pub fn run(root: &Path, check: bool) -> Result<(), Vec<String>> {
    let now = count(root).map_err(|e| vec![format!("could not count the tests: {e}")])?;
    println!("test binaries: {}", now.binaries);
    println!("test functions: {}", now.functions);
    if !check {
        return Ok(());
    }
    let base = read_baseline(root)?;
    let mut violations = Vec::new();
    if now.binaries < base.binaries {
        violations.push(format!(
            "test binaries dropped below the baseline: {} < {}",
            now.binaries, base.binaries
        ));
    }
    if now.functions < base.functions {
        violations.push(format!(
            "test functions dropped below the baseline: {} < {}",
            now.functions, base.functions
        ));
    }
    if violations.is_empty() {
        println!(
            "PASS: test-inventory ({} binaries >= {}, {} functions >= {})",
            now.binaries, base.binaries, now.functions, base.functions
        );
        Ok(())
    } else {
        Err(violations)
    }
}
