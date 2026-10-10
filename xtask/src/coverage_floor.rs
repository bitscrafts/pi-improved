//! Spec 108 R2: the coverage ratchet.
//!
//! Reads the `cargo llvm-cov --json --summary-only` export and fails when the
//! workspace line-coverage total is below the floor pinned in
//! `coverage-floor.txt` (override with `--floor N`, used by the mutation
//! control). The floor is a ratchet: raise it when coverage rises, never
//! lower it to make a run green.

// reason: see lint_files.rs — binary-internal helpers; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::fs;

use xtask::workspace_root;

/// Compares the measured total with the floor.
fn check(percent: f64, floor: f64) -> Result<(), Vec<String>> {
    if percent + f64::EPSILON >= floor {
        println!("coverage-floor: total {percent:.2}% >= floor {floor:.2}%");
        Ok(())
    } else {
        Err(vec![format!(
            "coverage total {percent:.2}% is below the floor {floor:.2}% (coverage-floor.txt)"
        )])
    }
}

/// Extracts the workspace line-coverage percent from a llvm-cov JSON export.
fn total_percent(json: &str) -> Result<f64, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("bad json: {e}"))?;
    v.pointer("/data/0/totals/lines/percent")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| "no data[0].totals.lines.percent in the export".to_owned())
}

/// `coverage-floor <summary.json> [--floor N]`.
pub(crate) fn run(args: &[String]) -> Result<(), Vec<String>> {
    let path = args
        .first()
        .ok_or_else(|| vec!["usage: coverage-floor <summary.json> [--floor N]".to_owned()])?;
    let floor_src = match args.iter().position(|a| a == "--floor") {
        Some(i) => args.get(i + 1).cloned().unwrap_or_default(),
        None => fs::read_to_string(workspace_root().join("coverage-floor.txt"))
            .map_err(|e| vec![format!("coverage-floor.txt: {e}")])?,
    };
    let floor_line = floor_src
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or_default();
    let floor: f64 = floor_line
        .parse()
        .map_err(|e| vec![format!("floor {floor_line:?}: {e}")])?;
    let json = fs::read_to_string(path).map_err(|e| vec![format!("{path}: {e}")])?;
    check(total_percent(&json).map_err(|e| vec![e])?, floor)
}

#[cfg(test)]
mod tests {
    use super::{check, total_percent};

    #[test]
    fn passes_at_or_above_floor() {
        assert!(check(76.4, 73.0).is_ok());
        assert!(check(73.0, 73.0).is_ok());
    }

    #[test]
    fn fails_below_floor() {
        assert!(check(76.4, 80.0).is_err());
    }

    #[test]
    fn parses_total_and_rejects_garbage() {
        let j = r#"{"data":[{"totals":{"lines":{"percent":76.4}}}]}"#;
        assert!((total_percent(j).unwrap_or(0.0) - 76.4).abs() < 1e-9);
        assert!(total_percent("{}").is_err());
        assert!(total_percent("nope").is_err());
    }
}
