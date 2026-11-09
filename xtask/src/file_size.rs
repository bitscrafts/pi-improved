//! Spec 402: one counter and one per-file ratchet for the 450-line file limit.
//!
//! `physical_lines` is the only line counter: `wc -l` over every `.rs` file under `crates/`, `src/` and `tests/` alike,
//! `target/` skipped. `lint-files` and `crate-audit` both judge that number. The effective count (non-blank,
//! non-comment) survives as a report column in `lint-files --report` and nowhere else.
//!
//! The baseline records each over-budget file by path: `path<TAB>lines`. A file not listed may not exceed the limit;
//! a listed file may not grow; a listed file that is at or under the limit, or no longer exists, must be removed.
//! `--update-baseline` only lowers or removes entries. `xtask/` is outside this measure: its files are not crates.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The per-file limit, in physical lines.
pub const LIMIT: usize = 450;
/// `lint-files --report` lists files within this many lines of the limit.
pub const HEADROOM: usize = 30;
/// The checked-in per-file baseline.
pub const BASELINE: &str = "_docs/05-operations/crate-audit-baseline.tsv";
const HEADER: &str = "path\tlines";
const OLD_HEADER: &str = "crate\tover_budget";

/// Physical lines of one file, as `wc -l` counts them (a final unterminated line counts too).
///
/// # Errors
/// Returns the I/O error text when the file cannot be read.
pub fn physical_lines(path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text.lines().count())
}

/// Every `.rs` file under `root/crates`, as `(relative path, physical lines)`, sorted by path.
///
/// # Errors
/// Returns an error when a directory cannot be listed or a file cannot be read.
pub fn measure(root: &Path) -> Result<Vec<(String, usize)>, String> {
    let mut paths = Vec::new();
    rs_files(&root.join("crates"), &mut paths)?;
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            Ok((rel, physical_lines(path)?))
        })
        .collect()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .path();
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

/// Reads the path-keyed baseline.
///
/// # Errors
/// Returns an error when the file is missing, has another header, or a row is malformed.
pub fn read_baseline(root: &Path) -> Result<BTreeMap<String, usize>, String> {
    let path = root.join(BASELINE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!("file-size: no baseline at {BASELINE} ({e}); create it with `crate-audit --update-baseline`")
    })?;
    if text.lines().next().map(str::trim) != Some(HEADER) {
        return Err(format!(
            "file-size: {BASELINE} is not in the {HEADER:?} format; run `crate-audit --migrate-from-crate-counts`"
        ));
    }
    let mut map = BTreeMap::new();
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let (path, lines) = line
            .split_once('\t')
            .ok_or_else(|| format!("file-size: malformed baseline row {line:?}"))?;
        let lines = lines
            .trim()
            .parse::<usize>()
            .map_err(|e| format!("file-size: malformed baseline count in {line:?}: {e}"))?;
        map.insert(path.trim().to_owned(), lines);
    }
    Ok(map)
}

/// Files over the limit that are unlisted, or listed with a smaller count than they now have.
fn growth(measured: &[(String, usize)], base: &BTreeMap<String, usize>) -> Vec<String> {
    measured
        .iter()
        .filter(|(_, lines)| *lines > LIMIT)
        .filter_map(|(path, lines)| match base.get(path) {
            None => Some(format!(
                "{path}: {lines} lines, over the {LIMIT}-line limit and not in the baseline; split it into a folder module"
            )),
            Some(recorded) if lines > recorded => Some(format!(
                "{path}: {lines} lines, grew past its recorded {recorded}; the baseline only shrinks"
            )),
            Some(_) => None,
        })
        .collect()
}

/// Every refusal of the per-file ratchet: growth, plus listed files that are at or under the limit or that no longer exist.
pub fn violations(measured: &[(String, usize)], base: &BTreeMap<String, usize>) -> Vec<String> {
    let now: BTreeMap<&str, usize> = measured.iter().map(|(p, l)| (p.as_str(), *l)).collect();
    let mut out = growth(measured, base);
    for (path, recorded) in base {
        match now.get(path.as_str()) {
            None => out.push(format!(
                "{path}: listed in the baseline (recorded {recorded}) but no longer exists; remove the entry"
            )),
            Some(lines) if *lines <= LIMIT => out.push(format!(
                "{path}: {lines} lines, at or under the {LIMIT}-line limit but still listed (recorded {recorded}); remove the entry"
            )),
            Some(_) => {}
        }
    }
    out
}

/// `--ratchet`: fails on every refusal in [`violations`].
///
/// # Errors
/// Returns one message per refusal, each naming the file, its count and the recorded count.
pub fn check_ratchet(root: &Path, measured: &[(String, usize)]) -> Result<(), Vec<String>> {
    let base = read_baseline(root).map_err(|e| vec![e])?;
    let found = violations(measured, &base);
    if found.is_empty() {
        println!(
            "file-size: ratchet holds, every file over {LIMIT} lines is listed at or above its count"
        );
        Ok(())
    } else {
        Err(found)
    }
}

/// `--update-baseline`: writes the baseline on first creation; afterwards refuses any addition or raise, and
/// writes only the current over-limit files, so stale entries fall away.
///
/// # Errors
/// Returns one message per refused addition or raise; the baseline file is then left untouched.
pub fn update_baseline(root: &Path, measured: &[(String, usize)]) -> Result<(), Vec<String>> {
    if root.join(BASELINE).is_file() {
        let base = read_baseline(root).map_err(|e| vec![e])?;
        let raised = growth(measured, &base);
        if !raised.is_empty() {
            return Err(raised
                .into_iter()
                .map(|g| format!("refused, the baseline never raises a count: {g}"))
                .collect());
        }
    }
    let over: Vec<(String, usize)> = measured
        .iter()
        .filter(|(_, lines)| *lines > LIMIT)
        .cloned()
        .collect();
    write_rows(root, &over).map_err(|e| vec![e])?;
    println!(
        "file-size: wrote {BASELINE} ({} over-budget file(s))",
        over.len()
    );
    Ok(())
}

/// `--migrate-from-crate-counts`: converts the old `crate<TAB>over_budget` baseline, once. It writes nothing when the
/// new format already exists, or when any crate holds more over-budget files than its old count records.
///
/// # Errors
/// Returns the refusal reasons; nothing is written on refusal.
pub fn migrate(root: &Path, measured: &[(String, usize)]) -> Result<(), Vec<String>> {
    let path = root.join(BASELINE);
    let old_text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(vec![format!("cannot read {}: {e}", path.display())]),
    };
    if old_text.lines().next().map(str::trim) == Some(HEADER) {
        return Err(vec![format!(
            "refused, {BASELINE} is already in the {HEADER:?} format"
        )]);
    }
    let old = read_crate_counts(&old_text).map_err(|e| vec![e])?;
    let over: Vec<(String, usize)> = measured
        .iter()
        .filter(|(_, lines)| *lines > LIMIT)
        .cloned()
        .collect();
    let mut held: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (file, _) in &over {
        let krate = file.split('/').nth(1).unwrap_or_default();
        held.entry(krate.to_owned()).or_default().push(file);
    }
    let refused: Vec<String> = held
        .iter()
        .filter_map(|(krate, files)| {
            let recorded = old.get(krate).copied().unwrap_or(0);
            (files.len() > recorded).then(|| {
                format!(
                    "crate {krate}: {} over-budget file(s) but its old baseline records {recorded}: {}",
                    files.len(),
                    files.join(", ")
                )
            })
        })
        .collect();
    if !refused.is_empty() {
        return Err(refused);
    }
    write_rows(root, &over).map_err(|e| vec![e])?;
    println!(
        "file-size: migrated {BASELINE} to the {HEADER:?} format ({} file(s))",
        over.len()
    );
    Ok(())
}

fn read_crate_counts(text: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut map = BTreeMap::new();
    if text.is_empty() {
        return Ok(map);
    }
    if text.lines().next().map(str::trim) != Some(OLD_HEADER) {
        return Err(format!(
            "file-size: {BASELINE} is neither the old nor the new baseline format"
        ));
    }
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let (krate, count) = line
            .split_once('\t')
            .ok_or_else(|| format!("file-size: malformed baseline row {line:?}"))?;
        let count = count
            .trim()
            .parse::<usize>()
            .map_err(|e| format!("file-size: malformed baseline count in {line:?}: {e}"))?;
        map.insert(krate.trim().to_owned(), count);
    }
    Ok(map)
}

fn write_rows(root: &Path, rows: &[(String, usize)]) -> Result<(), String> {
    let mut body = format!("{HEADER}\n");
    for (path, lines) in rows {
        let _ = writeln!(body, "{path}\t{lines}");
    }
    let path = root.join(BASELINE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Files within [`HEADROOM`] lines of the limit (or over it), largest first, as `(path, lines)`.
#[must_use]
pub fn near_limit(measured: &[(String, usize)]) -> Vec<(String, usize)> {
    let mut near: Vec<(String, usize)> = measured
        .iter()
        .filter(|(_, lines)| *lines + HEADROOM >= LIMIT)
        .cloned()
        .collect();
    near.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    near
}

#[cfg(test)]
mod tests {
    use super::{LIMIT, check_ratchet, growth, near_limit, violations};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn row(path: &str, lines: usize) -> (String, usize) {
        (path.to_owned(), lines)
    }

    #[test]
    fn a_file_at_the_limit_is_not_growth() {
        let measured = vec![row("crates/x/src/a.rs", LIMIT)];
        assert_eq!(growth(&measured, &BTreeMap::new()), Vec::<String>::new());
    }

    #[test]
    fn a_file_over_the_limit_is_growth_when_unlisted() {
        let measured = vec![row("crates/x/src/a.rs", LIMIT + 1)];
        assert_eq!(growth(&measured, &BTreeMap::new()).len(), 1);
    }

    #[test]
    fn violations_include_the_stale_entry_of_a_file_now_under_the_limit() {
        let measured = vec![row("crates/x/src/a.rs", 10)];
        let base = BTreeMap::from([(String::from("crates/x/src/a.rs"), 500)]);
        assert_eq!(violations(&measured, &base).len(), 1);
    }

    #[test]
    fn near_limit_lists_files_within_the_headroom_largest_first() {
        let measured = vec![
            row("a.rs", 100),
            row("b.rs", 430),
            row("c.rs", 420),
            row("d.rs", 450),
        ];
        let near = near_limit(&measured);
        assert_eq!(
            near,
            vec![row("d.rs", 450), row("b.rs", 430), row("c.rs", 420)]
        );
    }

    #[test]
    fn ratchet_reports_a_missing_baseline_as_an_error() {
        let root = Path::new("/nonexistent-file-size-root");
        assert!(check_ratchet(root, &[]).is_err());
    }
}
