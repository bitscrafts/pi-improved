//! CLI argument parsers shared by the `xtask` subcommand dispatch in
//! `main.rs` (pure move — bodies are byte-identical to their originals).

#![allow(clippy::missing_errors_doc)] // reason: pure move of private fns; Err(String) is the CLI error message itself.

/// Parses `memory-eval --corpus <f> --queries <f> [--top-k N]` (F4.16).
pub fn parse_mem_eval(rest: &[String]) -> Result<(String, String, usize), String> {
    let mut corpus = String::new();
    let mut queries = String::new();
    let mut top_k = 10usize;
    let mut i = 0usize;
    while i < rest.len() {
        match rest.get(i).map(String::as_str) {
            Some("--corpus") => {
                i += 1;
                corpus = rest.get(i).cloned().ok_or("--corpus needs a value")?;
            }
            Some("--queries") => {
                i += 1;
                queries = rest.get(i).cloned().ok_or("--queries needs a value")?;
            }
            Some("--top-k") => {
                i += 1;
                top_k = rest
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--top-k needs a number")?;
            }
            other => return Err(format!("unknown memory-eval arg '{other:?}'")),
        }
        i += 1;
    }
    if corpus.is_empty() || queries.is_empty() {
        return Err(String::from("memory-eval needs --corpus and --queries"));
    }
    Ok((corpus, queries, top_k))
}

/// Parses `promote-build [--worktree <path>]` (spec 34 R2).
pub fn parse_worktree_flag(rest: &[String]) -> Option<std::path::PathBuf> {
    let mut i = 0usize;
    while i < rest.len() {
        if rest.get(i).map(String::as_str) == Some("--worktree") {
            return rest.get(i + 1).map(std::path::PathBuf::from);
        }
        i += 1;
    }
    None
}

/// Parses `promote-collect <sig> <approval_id> [--approval-db <path>] [--by <name>]`
/// (spec 34 R4).
#[allow(clippy::type_complexity)] // reason: one small positional+flag parse; a
// named struct would not shrink this call site.
pub fn parse_promote_collect(
    rest: &[String],
) -> Result<(String, String, std::path::PathBuf, Option<String>), String> {
    let sig = rest
        .first()
        .cloned()
        .ok_or("promote-collect needs a <sig>")?;
    let approval_id = rest
        .get(1)
        .cloned()
        .ok_or("promote-collect needs an <approval_id>")?;
    let mut approval_db: Option<std::path::PathBuf> = None;
    let mut by: Option<String> = None;
    let mut i = 2usize;
    while i < rest.len() {
        match rest.get(i).map(String::as_str) {
            Some("--approval-db") => {
                i += 1;
                approval_db = rest.get(i).map(std::path::PathBuf::from);
            }
            Some("--by") => {
                i += 1;
                by = rest.get(i).cloned();
            }
            other => return Err(format!("unknown promote-collect arg '{other:?}'")),
        }
        i += 1;
    }
    let approval_db = approval_db.ok_or("promote-collect needs --approval-db <path>")?;
    Ok((sig, approval_id, approval_db, by))
}

/// Parses `rss-gate [--secs N] [--interval N] [--max-mb N] [--runs N]` (F5.14).
pub fn parse_rss_gate(rest: &[String]) -> Result<(u64, u64, u64, u32), String> {
    let mut secs = 300u64;
    let mut interval = 10u64;
    let mut max_mb = 50u64;
    let mut runs = 3000u32;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let next_u64 = |name: &str, it: &mut std::slice::Iter<'_, String>| -> Result<u64, String> {
            it.next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{name} needs a number"))
        };
        match arg.as_str() {
            "--secs" => secs = next_u64("--secs", &mut it)?,
            "--interval" => interval = next_u64("--interval", &mut it)?,
            "--max-mb" => max_mb = next_u64("--max-mb", &mut it)?,
            "--runs" => {
                let v = next_u64("--runs", &mut it)?;
                runs = v.try_into().map_err(|_| "--runs too large")?;
            }
            other => return Err(format!("unknown rss-gate arg '{other}'")),
        }
    }
    Ok((secs, interval, max_mb, runs))
}
