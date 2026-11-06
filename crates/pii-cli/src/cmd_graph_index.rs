//! `pii graph index` and `pii graph find` (spec 388). The catalogue is derived from the graph files and kept in the
//! memory store: the daemon writes and answers when it runs, because it holds the store, and the store file is opened
//! directly when none does.

// reason: bin crate: pub(crate) is the reachable form, and the module is private.
#![allow(clippy::redundant_pub_crate)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Clock as _;
use pii_memory::catalogue::{Entry, Synced, find, sync};
use pii_memory::embed::TrigramEmbedder;
use pii_memory::search::MemorySearcher;
use pii_memory::skill::sha256_hex;
use pii_proto::method::Method;
use pii_store::{MemStore, Store};
use serde_json::{Value, json};

use crate::cli_util::fail;
use crate::cmd_ctl::{CallError, next_str, run_call_typed};

const USAGE: &str = "usage: pii graph index [--dir D]\n       pii graph find <question>\n\
--dir is the system graph directory and defaults to ./graphs; the user graph directory is always walked too.";

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn error(msg: impl std::fmt::Display) -> ExitCode {
    fail("graph", msg);
    ExitCode::from(2)
}

/// `pii graph index [--dir D]`.
pub(crate) fn cmd_index(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut system = PathBuf::from("graphs");
    while let Some(arg) = next_str(args) {
        match (arg.as_str(), next_str(args)) {
            ("--dir", Some(dir)) => system = PathBuf::from(dir),
            _ => return usage(),
        }
    }
    let user = pii_core::state_dir().join("graphs");
    // The user directory first: it is the first one the daemon searches, so its graph is the one a name loads.
    let (entries, refused) = derive(&[user, system]);
    for (file, why) in &refused {
        eprintln!("not indexed: {}: {why}", file.display());
    }
    let duplicates = duplicate_flows(&entries);
    for line in &duplicates {
        println!("{line}");
    }
    let (synced, unlooked) = match write_entries(&entries) {
        Ok(done) => done,
        Err(code) => return code,
    };
    println!(
        "indexed {} graphs ({} written, {} unchanged, {} removed, {} refused)",
        entries.len(),
        synced.written,
        synced.unchanged,
        synced.removed,
        refused.len()
    );
    println!(
        "graphs_written_without_a_lookup={unlooked} duplicate_flows_found={}",
        duplicates.len()
    );
    ExitCode::SUCCESS
}

/// `pii graph find <question>`.
pub(crate) fn cmd_find(args: &mut std::env::ArgsOs) -> ExitCode {
    let words: Vec<String> = std::iter::from_fn(|| next_str(args)).collect();
    if words.is_empty() {
        return usage();
    }
    let question = words.join(" ");
    let text = match run_call_typed(Method::GraphFind, json!({ "question": question })) {
        Ok(reply) => reply
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Err(CallError::Failed(msg)) => return error(msg),
        Err(CallError::Down(_)) => match find_in_store(&question) {
            Ok(text) => text,
            Err(e) => return error(e),
        },
    };
    print!("{text}");
    ExitCode::SUCCESS
}

fn find_in_store(question: &str) -> Result<String, PiiError> {
    let path = pii_core::state_dir().join("memory.redb");
    if !path.exists() {
        return Ok(String::from(
            "no candidate: the catalogue is empty; run pii graph index\n",
        ));
    }
    let store = Arc::new(MemStore::open(&path)?);
    let searcher = MemorySearcher::new(Arc::clone(&store), Some(Arc::new(TrigramEmbedder)));
    let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    Ok(find(&searcher, &store, &ctx, question)?.text)
}

/// Writes through the daemon when one runs, else into the store files. Also answers the R5 counter, read from the run
/// journals: an index run itself is never counted as a lookup.
fn write_entries(entries: &[Entry]) -> Result<(Synced, u64), ExitCode> {
    match run_call_typed(Method::GraphIndex, json!({ "entries": entries })) {
        Ok(reply) => {
            let synced =
                serde_json::from_value(reply.get("synced").cloned().unwrap_or(Value::Null))
                    .map_err(|e| error(format!("the daemon's index reply does not decode: {e}")))?;
            let unlooked = reply
                .get("graphs_written_without_a_lookup")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Ok((synced, unlooked))
        }
        Err(CallError::Failed(msg)) => Err(error(msg)),
        Err(CallError::Down(_)) => write_to_store(entries).map_err(error),
    }
}

fn write_to_store(entries: &[Entry]) -> Result<(Synced, u64), PiiError> {
    let dir = pii_core::state_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| PiiError::Internal(format!("create {}: {e}", dir.display())))?;
    let memory = MemStore::open(&dir.join("memory.redb"))?;
    let synced = sync(&memory, entries, pii_core::SystemClock.now_ms())?;
    let runs = dir.join("store.redb");
    let unlooked = if runs.exists() {
        pii_server::graphs_written_without_a_lookup(&Store::open(&runs)?)?
    } else {
        0
    };
    Ok((synced, unlooked))
}

/// Every graph under `dirs`, and the files that do not load with the reason. The first directory to name a graph wins.
fn derive(dirs: &[PathBuf]) -> (Vec<Entry>, Vec<(PathBuf, String)>) {
    let mut files = Vec::new();
    for dir in dirs {
        toml_files(dir, &mut files);
    }
    let mut seen_files = BTreeSet::new();
    let (mut entries, mut refused) = (Vec::<Entry>::new(), Vec::new());
    for file in files {
        let Ok(real) = dunce::canonicalize(&file) else {
            continue;
        };
        if !seen_files.insert(real.clone()) {
            continue;
        }
        match entry_of(&real) {
            Ok(entry) if entries.iter().any(|e| e.name == entry.name) => {
                eprintln!("shadowed: {} ({})", real.display(), entry.name);
            }
            Ok(entry) => entries.push(entry),
            Err(why) => refused.push((file, why.replace('\n', " "))),
        }
    }
    (entries, refused)
}

fn toml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let dotfile = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_none_or(|n| n.starts_with('.'));
        if dotfile {
            continue;
        }
        if path.is_dir() {
            toml_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
}

/// The catalogue entry a file publishes, after the checks `pii graph lint` makes.
fn entry_of(file: &Path) -> Result<Entry, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let src = String::from_utf8_lossy(&bytes);
    crate::cmd_graph_installed::lint_file(&src, file)?;
    let doc: toml::Table = toml::from_str(&src).map_err(|e| e.to_string())?;
    let text = |key: &str| {
        doc.get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let tables = |key: &str| {
        doc.get(key)
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (nodes, edges) = (tables("node"), tables("edge"));
    let of = |table: &toml::Value, key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let kinds: BTreeSet<String> = nodes.iter().filter_map(|n| of(n, "kind")).collect();
    let calls: BTreeSet<String> = nodes
        .iter()
        .filter(|n| of(n, "kind").as_deref() == Some("call"))
        .filter_map(|n| of(n, "graph"))
        .collect();
    let rules = nodes
        .iter()
        .filter_map(|n| n.get("rules").and_then(toml::Value::as_array))
        .flatten();
    let ports: BTreeSet<String> = edges
        .iter()
        .chain(rules)
        .filter_map(|t| of(t, "port"))
        .collect();
    let caps = doc
        .get("caps")
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let about = header_comment(&src);
    let described = text("description");
    let source = if described.is_empty() {
        about.as_str()
    } else {
        described.as_str()
    };
    Ok(Entry {
        name: text("name"),
        file: file.display().to_string(),
        revision: sha256_hex(&bytes),
        purpose: first_clause(source),
        about: format!("{described} {about}").trim().to_owned(),
        entry: text("entry"),
        exit: text("exit"),
        kinds: kinds.into_iter().collect(),
        calls: calls.into_iter().collect(),
        ports: ports.into_iter().collect(),
        caps,
    })
}

/// The comment a graph file opens with, on one line.
fn header_comment(src: &str) -> String {
    src.lines()
        .map_while(|l| l.strip_prefix('#'))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line of purpose: the first clause, without the `Spec N:` a header opens with.
fn first_clause(text: &str) -> String {
    let body = text
        .strip_prefix("Spec ")
        .and_then(|rest| rest.split_once(": "))
        .map_or(text, |(_, after)| after);
    let end = [", ", ". ", "; "]
        .iter()
        .filter_map(|stop| body.find(stop))
        .min()
        .unwrap_or(body.len());
    let clause = body.get(..end).unwrap_or(body).trim_end_matches('.').trim();
    if clause.is_empty() {
        String::from("(the file states no purpose)")
    } else {
        clause.to_owned()
    }
}

/// R5: the `duplicate-flow` lines. Two graphs with one call shape are one flow written twice, whichever came first.
fn duplicate_flows(entries: &[Entry]) -> Vec<String> {
    let mut lines = Vec::new();
    for (at, one) in entries.iter().enumerate() {
        for other in entries.iter().skip(at + 1) {
            if !one.calls.is_empty() && one.calls == other.calls {
                lines.push(format!(
                    "duplicate-flow: {} and {} have one call shape ({})",
                    one.name,
                    other.name,
                    one.calls.join(", ")
                ));
            }
        }
    }
    lines
}
