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
pii memory link <from-id> <to-id> {--kind|--rel} K [--source S]... [--attr k=v]... [--dir D]\n       \
pii memory neighbours <id> [--depth N] [--limit N] [--dir D]\n       pii memory compact [--dir D]\n\
sources are run:<id>, file:<path> or actor:<name>; --dir defaults to the runtime dir; \
caps: --max-entities --max-relationships --max-provenance --max-depth --max-results --max-bytes;\n\
tiers (spec 127): --hot-secs --warm-secs --hot-count; list takes --rank; \
`pii memory show <id>` states the tier and why; `pii memory tiers` reports counts;\n\
`pii memory archive --before <epoch-secs|Ns|Nm|Nh|Nd ago>` writes <dir>/memory/archive-<ts>.jsonl, \
verifies it, then removes those records; `pii memory restore <file>` reads one back; \
--now-secs N overrides the clock (diagnostics and tests).";

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
    rank: bool,
    before: Option<String>,
    now_secs: Option<u64>,
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts {
        dir: Some(pii_core::state_dir()),
        cfg: GraphConfig::default(),
        kind: None,
        name: None,
        sources: Vec::new(),
        attrs: Vec::new(),
        positional: Vec::new(),
        depth: None,
        limit: None,
        rank: false,
        before: None,
        now_secs: None,
    };
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--kind" | "--rel" => o.kind = Some(next_str(args)?),
            "--name" => o.name = Some(next_str(args)?),
            "--source" => o.sources.push(next_str(args)?),
            "--attr" => {
                let kv = next_str(args)?;
                let (k, v) = kv.split_once('=')?;
                o.attrs.push((k.to_owned(), v.to_owned()));
            }
            "--depth" => o.depth = Some(next_str(args)?.parse().ok()?),
            "--limit" => o.limit = Some(next_str(args)?.parse().ok()?),
            "--rank" => o.rank = true,
            "--before" => o.before = Some(next_str(args)?),
            "--now-secs" => o.now_secs = Some(next_str(args)?.parse().ok()?),
            "--hot-secs" => o.cfg.hot_secs = next_str(args)?.parse().ok()?,
            "--warm-secs" => o.cfg.warm_secs = next_str(args)?.parse().ok()?,
            "--hot-count" => o.cfg.hot_count = next_str(args)?.parse().ok()?,
            "--max-bytes" => o.cfg.max_bytes = next_str(args)?.parse().ok()?,
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
    let store = with_clock(store, o.now_secs);
    let pos = |i: usize| o.positional.get(i).map(String::as_str);
    if is_tier_cmd(&o, sub, verb.as_deref()) {
        return tier_cmd(&store, &o, sub, verb.as_deref());
    }
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
            let mut sources = o.sources.clone();
            if sources.is_empty() {
                sources.push(String::from("actor:cli"));
            }
            let input = RelationshipInput {
                from: from.to_owned(),
                to: to.to_owned(),
                kind,
                attributes: o.attrs.iter().cloned().collect(),
                provenance: sources,
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

/// `--now-secs N` pins the store's clock (diagnostics and tests); otherwise the system clock.
fn with_clock(store: EntityStore, now_secs: Option<u64>) -> EntityStore {
    let Some(n) = now_secs else { return store };
    let clock = std::sync::Arc::new(pii_core::FixedClock::new());
    clock.advance(n.saturating_mul(1000));
    store.with_clock(clock)
}

/// The tiering surface (spec 127): `show`, `tiers`, `archive`, `restore` and `entity list --rank`.
fn tier_cmd(store: &EntityStore, o: &Opts, sub: &str, verb: Option<&str>) -> ExitCode {
    let pos = |i: usize| o.positional.get(i).map(String::as_str);
    match (sub, verb) {
        ("entity", Some("show")) | ("show", None) => show(store, pos(0)),
        ("tiers", None) => {
            if let Err(e) = store.sweep() {
                return refused(&e);
            }
            store.counts().map_or_else(
                |e| refused(&e),
                |c| {
                    print(&serde_json::json!({
                        "tiers": c.tiers, "tier_changes": c.tier_changes,
                        "archive_ops": c.archive_ops, "entities": c.entities,
                        "relationships": c.relationships, "bytes": c.bytes,
                    }))
                },
            )
        }
        ("archive", None) => archive(store, o),
        ("restore", None) => {
            let Some(file) = pos(0) else { return usage() };
            match store.restore(std::path::Path::new(file)) {
                Ok(r) => {
                    let _ = writeln!(
                        std::io::stdout(),
                        "restored={} unchanged={} refused={}",
                        r.restored,
                        r.unchanged,
                        r.refused.len()
                    );
                    for (line, reason, why) in &r.refused {
                        let _ = writeln!(std::io::stdout(), "refused line {line}: {reason}: {why}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => refused(&e),
            }
        }
        ("entity", Some("list")) if o.rank => {
            let found = match (&o.kind, &o.name) {
                (Some(k), None) => store.by_kind_ranked(k, o.limit),
                (None, Some(n)) => store.by_name_ranked(n, o.limit),
                _ => return usage(),
            };
            found.map_or_else(
                |e| refused(&e),
                |list| {
                    print(&serde_json::json!(
                        list.iter()
                            .map(|r| serde_json::json!({"entity": r.entity, "tier": r.info}))
                            .collect::<Vec<_>>()
                    ))
                },
            )
        }
        _ => usage(),
    }
}

/// True for the subcommands [`tier_cmd`] serves.
fn is_tier_cmd(o: &Opts, sub: &str, verb: Option<&str>) -> bool {
    matches!(
        (sub, verb),
        ("show" | "tiers" | "archive" | "restore", None) | ("entity", Some("show"))
    ) || (sub == "entity" && verb == Some("list") && o.rank)
}

/// `show`: the record and, from just before this read, the tier it was in and why.
fn show(store: &EntityStore, id: Option<&str>) -> ExitCode {
    let Some(id) = id else { return usage() };
    let info = store.explain(id);
    match (store.get(id), info) {
        (Ok(Some(e)), Ok(info)) => {
            let mut v = serde_json::json!(e);
            if let Some(m) = v.as_object_mut() {
                m.insert(String::from("tier"), serde_json::json!(info));
            }
            print(&v)
        }
        (Ok(None), _) => fail(&format!("no entity {id}")),
        (Err(e), _) | (_, Err(e)) => refused(&e),
    }
}

/// `--before`: epoch seconds, or an age (`90s`, `15m`, `12h`, `30d`) counted back from now.
fn cutoff(store: &EntityStore, text: &str) -> Option<u64> {
    let (num, unit) = text.split_at(text.len().checked_sub(1)?);
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return text.parse().ok(),
    };
    Some(
        store
            .now_secs()
            .saturating_sub(num.parse::<u64>().ok()?.checked_mul(mult)?),
    )
}

/// The audited removal: dump, verify, then remove.
fn archive(store: &EntityStore, o: &Opts) -> ExitCode {
    let Some(before) = o.before.as_deref().and_then(|t| cutoff(store, t)) else {
        return usage();
    };
    match store.archive_before(before) {
        Ok(r) => {
            let file = r
                .path
                .as_ref()
                .map_or_else(|| String::from("-"), |p| p.display().to_string());
            let _ = writeln!(
                std::io::stdout(),
                "archived records={} bytes={} entities={} relationships={} file={file}",
                r.records,
                r.bytes,
                r.entities,
                r.relationships
            );
            ExitCode::SUCCESS
        }
        Err(e) => refused(&e),
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
