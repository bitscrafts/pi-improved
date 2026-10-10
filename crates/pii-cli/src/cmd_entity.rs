//! `pii memory {entity add|show|list|link|neighbours|compact}` (spec 126): the entity and
//! relationship store, offline. It reads and writes `<dir>/memory/` directly (the same
//! files the daemon's `remember`/`recall` nodes use, serialised by the store's lock), so it
//! works with the daemon stopped or running. Only `compact` insists the daemon be stopped.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_memory::entity::{EntityInput, EntityStore, GraphConfig, GraphError, RelationshipInput};

use crate::cmd_ctl::next_str;

const USAGE: &str = "usage: pii memory entity add --kind K --name N --source S... [--attr k=v]... [--dir D]\n       \
pii memory entity show <id> [--dir D]\n       pii memory entity list {--kind K|--name N} [--limit N] [--dir D]\n       \
pii memory link <from-id> <to-id> --kind K --source S... [--attr k=v]... [--dir D]\n       \
pii memory neighbours <id> [--depth N] [--limit N] [--dir D]\n       pii memory compact [--dir D]\n\
sources are run:<id>, file:<path> or actor:<name>; --dir defaults to the runtime dir; \
caps: --max-entities --max-relationships --max-provenance --max-depth --max-results.";

struct Opts {
    dir: Option<PathBuf>,
    cfg: GraphConfig,
    kind: Option<String>,
    name: Option<String>,
    sources: Vec<String>,
    attrs: Vec<(String, String)>,
    positional: Vec<String>,
    depth: Option<u32>,
    limit: Option<usize>,
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts {
        dir: pii_uds::runtime_dir("default"),
        cfg: GraphConfig::default(),
        kind: None,
        name: None,
        sources: Vec::new(),
        attrs: Vec::new(),
        positional: Vec::new(),
        depth: None,
        limit: None,
    };
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--kind" => o.kind = Some(next_str(args)?),
            "--name" => o.name = Some(next_str(args)?),
            "--source" => o.sources.push(next_str(args)?),
            "--attr" => {
                let kv = next_str(args)?;
                let (k, v) = kv.split_once('=')?;
                o.attrs.push((k.to_owned(), v.to_owned()));
            }
            "--depth" => o.depth = Some(next_str(args)?.parse().ok()?),
            "--limit" => o.limit = Some(next_str(args)?.parse().ok()?),
            "--max-entities" => o.cfg.max_entities = next_str(args)?.parse().ok()?,
            "--max-relationships" => o.cfg.max_relationships = next_str(args)?.parse().ok()?,
            "--max-provenance" => o.cfg.max_provenance = next_str(args)?.parse().ok()?,
            "--max-depth" => o.cfg.max_depth = next_str(args)?.parse().ok()?,
            "--max-results" => o.cfg.max_results = next_str(args)?.parse().ok()?,
            s if !s.starts_with("--") => o.positional.push(s.to_owned()),
            _ => return None,
        }
    }
    Some(o)
}

fn fail(msg: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "error: {msg}");
    ExitCode::from(2)
}

fn usage() -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{USAGE}");
    ExitCode::from(2)
}

fn print(v: &serde_json::Value) -> ExitCode {
    match serde_json::to_string(v) {
        Ok(s) => {
            let _ = writeln!(std::io::stdout(), "{s}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn refused(e: &GraphError) -> ExitCode {
    fail(&e.to_string())
}

/// `pii memory <sub> ...` for the entity surface; `sub` is `entity`, `link`, `neighbours` or `compact`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_entity(sub: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let verb = if sub == "entity" {
        next_str(args)
    } else {
        None
    };
    let Some(o) = parse(args) else {
        return usage();
    };
    let Some(dir) = o.dir.clone() else {
        return fail("no store directory (use --dir)");
    };
    if sub == "compact" {
        return compact(&dir, &o);
    }
    let store = match EntityStore::open(&dir, o.cfg) {
        Ok(s) => s,
        Err(e) => return fail(&e.to_string()),
    };
    let pos = |i: usize| o.positional.get(i).map(String::as_str);
    match (sub, verb.as_deref()) {
        ("entity", Some("add")) => {
            let (Some(kind), Some(name)) = (o.kind.clone(), o.name.clone()) else {
                return usage();
            };
            let input = EntityInput {
                kind,
                name,
                attributes: o.attrs.iter().cloned().collect(),
                provenance: o.sources.clone(),
            };
            store
                .put_entity(input)
                .map_or_else(|e| refused(&e), |p| print(&serde_json::json!(p.record)))
        }
        ("entity", Some("show")) => {
            let Some(id) = pos(0) else { return usage() };
            match store.get(id) {
                Ok(Some(e)) => print(&serde_json::json!(e)),
                Ok(None) => fail(&format!("no entity {id}")),
                Err(e) => refused(&e),
            }
        }
        ("entity", Some("list")) => {
            let found = match (&o.kind, &o.name) {
                (Some(k), None) => store.by_kind(k, o.limit),
                (None, Some(n)) => store.by_name(n, o.limit),
                _ => return usage(),
            };
            found.map_or_else(|e| refused(&e), |list| print(&serde_json::json!(list)))
        }
        ("link", None) => {
            let (Some(from), Some(to), Some(kind)) = (pos(0), pos(1), o.kind.clone()) else {
                return usage();
            };
            let input = RelationshipInput {
                from: from.to_owned(),
                to: to.to_owned(),
                kind,
                attributes: o.attrs.iter().cloned().collect(),
                provenance: o.sources.clone(),
            };
            store
                .put_relationship(input)
                .map_or_else(|e| refused(&e), |p| print(&serde_json::json!(p.record)))
        }
        ("neighbours", None) => {
            let Some(id) = pos(0) else { return usage() };
            match store.neighbours(id, o.depth, o.limit) {
                Ok(n) => print(&serde_json::json!({
                    "root": n.root,
                    "entities": n.entities.iter().map(|(d, e)| serde_json::json!({"depth": d, "entity": e})).collect::<Vec<_>>(),
                    "relationships": n.relationships,
                })),
                Err(e) => refused(&e),
            }
        }
        _ => usage(),
    }
}

/// Compaction refuses while a daemon holds the store (the spec 115 behaviour).
fn compact(dir: &std::path::Path, o: &Opts) -> ExitCode {
    if let Err(e) = pii_store::compact::ensure_stopped(dir) {
        return fail(&e.to_string());
    }
    match EntityStore::open(dir, o.cfg).and_then(|s| s.compact()) {
        Ok(r) => {
            let _ = writeln!(
                std::io::stdout(),
                "compacted memory/graph.json {} -> {} bytes",
                r.bytes_before,
                r.bytes_after
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}
