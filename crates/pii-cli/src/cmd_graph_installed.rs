//! `pii graph {list|show <name>|explain|edit}` (spec 213): the audit and change surface for the
//! graphs the daemon loads from its installed directory (`<dir>/<name>.toml`). The repo copy is
//! `<repo>/<name>.toml`. `list`, `show` and `explain` only read files; the armed-run count is asked
//! of the daemon, or read from the store when no daemon holds it. `edit` lints first, moves the
//! previous file aside, and writes atomically: no verb deletes a graph.

// reason: bin crate: pub(crate) is the reachable form, and the module is private.
#![allow(clippy::redundant_pub_crate)]
mod edit;
mod explain;
mod list;

use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::RunStatus;
use pii_graph::fragment::Inlined;
use pii_graph::{GraphSpec, NodeRegistry};
use pii_proto::method::Method;
use pii_store::Store;
use serde_json::{Value, json};

use crate::cli_util::fail;
use crate::cmd_ctl::{next_str, run_call};

const USAGE: &str = "usage: pii graph list [--all] [--json] [--dir D] [--repo R]\n       \
pii graph show <name> [--dir D]\n       pii graph explain <name> [--dir D] [--repo R]\n       \
pii graph edit <name> --file <path> [--force] [--dir D] [--repo R] [--store S]\n\
--dir defaults to the runtime graphs dir; --repo defaults to ./graphs; --store defaults to the\n\
store beside the installed dir.";

/// The positional and flag values every verb here reads.
#[derive(Default)]
struct Opts {
    dir: Option<PathBuf>,
    repo: Option<PathBuf>,
    store: Option<PathBuf>,
    file: Option<PathBuf>,
    all: bool,
    json: bool,
    force: bool,
    positional: Vec<String>,
}

/// Where a graph's installed copy and its repo copy stand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drift {
    Same,
    Changed,
    InstalledOnly,
    RepoOnly,
}

impl Drift {
    fn label(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Changed => "DRIFT",
            Self::InstalledOnly => "installed-only",
            Self::RepoOnly => "repo-only",
        }
    }
}

/// A file that passed the checks `pii graph lint` makes: its spec, and what was woven into it.
struct Checked {
    spec: GraphSpec,
    woven: Vec<Inlined>,
}

/// A suspended run and the graph it belongs to. A run header carries the graph's body hash; the
/// name is read from the stored body. A key that stays a hash could not be named.
struct Suspended {
    run: String,
    graph: String,
}

/// One graph as `list` shows it.
struct Row {
    name: String,
    sha: String,
    checked: Result<Checked, String>,
    armed: Option<usize>,
    drift: Drift,
}

/// `pii graph list [--all] [--json]`.
pub(crate) fn cmd_list(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    list::run(&o)
}

/// `pii graph explain <name>`.
pub(crate) fn cmd_explain(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    explain::run(&o)
}

/// `pii graph edit <name> --file <path> [--force]`.
pub(crate) fn cmd_edit(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    edit::run(&o)
}

/// Whether `show`'s argument names an installed graph rather than a file: a bare name that is not
/// an existing path. `show graphs/x.toml` keeps its topology rendering.
pub(crate) fn is_installed_name(arg: &str) -> bool {
    let as_file = Path::new(arg);
    !arg.contains(['/', '\\'])
        && !as_file.exists()
        && !as_file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

/// `pii graph show <name>`: the installed TOML, verbatim.
pub(crate) fn cmd_show(name: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let name = match graph_name(name) {
        Ok(n) => n,
        Err(e) => return error(e),
    };
    let dirs = match installed_dirs(&o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let Some(path) = pii_server::find_graph(&dirs, &name) else {
        return error(pii_server::graph_not_found(&name, &dirs));
    };
    match std::fs::read_to_string(&path) {
        Ok(src) => {
            print!("{src}");
            ExitCode::SUCCESS
        }
        Err(e) => error(format!("{name} is not installed: {e}")),
    }
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts::default();
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--repo" => o.repo = Some(PathBuf::from(next_str(args)?)),
            "--store" => o.store = Some(PathBuf::from(next_str(args)?)),
            "--file" => o.file = Some(PathBuf::from(next_str(args)?)),
            "--all" => o.all = true,
            "--json" => o.json = true,
            "--force" => o.force = true,
            s if !s.starts_with("--") => o.positional.push(s.to_owned()),
            _ => return None,
        }
    }
    Some(o)
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// Reports an operator-facing error on stderr and returns the usage/IO exit code (2).
fn error(msg: impl Display) -> ExitCode {
    fail("graph", msg);
    ExitCode::from(2)
}

/// A graph name is a file stem: one path component, optionally written with `.toml`.
fn graph_name(raw: &str) -> Result<String, String> {
    let name = raw.strip_suffix(".toml").unwrap_or(raw);
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(format!("not a graph name: {raw}"));
    }
    Ok(name.to_owned())
}

/// The one name a verb takes, validated.
fn one_name(o: &Opts) -> Result<String, ExitCode> {
    match o.positional.as_slice() {
        [raw] => graph_name(raw).map_err(error),
        _ => Err(usage()),
    }
}

/// The directories graphs are read from, in the order the daemon searches them: the one `--dir`
/// names, else the same search `run start` makes (`pii_server::graph_search_dirs`) over the runtime
/// graphs dir and the daemon's default store. Canonical, so a runtime symlink resolves to the state
/// dir it names. The runtime entry is a symlink into the state dir and exists only while the daemon runs.
fn installed_dirs(o: &Opts) -> Result<Vec<PathBuf>, ExitCode> {
    if let Some(dir) = &o.dir {
        let dir = std::fs::canonicalize(dir)
            .map_err(|e| error(format!("no installed graphs at {}: {e}", dir.display())))?;
        return Ok(vec![dir]);
    }
    let dirs: Vec<PathBuf> = pii_uds::runtime_dir("default")
        .map(|r| {
            let store = o.store.clone().unwrap_or_else(|| r.join("store.redb"));
            pii_server::graph_search_dirs(&r.join("graphs"), &store)
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .collect();
    if dirs.is_empty() {
        return Err(error("no installed graphs dir (use --dir)"));
    }
    Ok(dirs)
}

/// The directory a write goes to: the first of [`installed_dirs`].
fn installed_dir(o: &Opts) -> Result<PathBuf, ExitCode> {
    installed_dirs(o)?
        .into_iter()
        .next()
        .ok_or_else(|| error("no installed graphs dir (use --dir)"))
}

fn repo_dir(o: &Opts) -> PathBuf {
    o.repo.clone().unwrap_or_else(|| PathBuf::from("graphs"))
}

/// The store the daemon writes runs to: beside the installed dir (the installer's layout).
fn store_path(o: &Opts, installed: &Path) -> PathBuf {
    o.store.clone().unwrap_or_else(|| {
        installed
            .parent()
            .map_or_else(|| PathBuf::from("store.redb"), |p| p.join("store.redb"))
    })
}

/// The fragments a host is woven from: the installed dir's own, else the repo's.
fn fragments_dir(primary: &Path, repo: &Path) -> PathBuf {
    let own = primary.join("fragments");
    if own.is_dir() {
        own
    } else {
        repo.join("fragments")
    }
}

/// The node registry the daemon lints with (the same one `pii graph lint` uses).
fn registry() -> NodeRegistry {
    let runtime = pii_uds::runtime_dir("default").unwrap_or_else(|| PathBuf::from("."));
    pii_server::lint_registry(Path::new("."), &runtime)
}

/// Weaves and checks a graph source the way `pii graph lint` does, then validates it.
fn check(
    src: &str,
    fragments: &Path,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
) -> Result<Checked, String> {
    let (spec, woven) = if pii_graph::fragment::declares_fragment(src) {
        let fragment = pii_graph::fragment::check_fragment(src, reg).map_err(|e| e.to_string())?;
        (fragment.spec, Vec::new())
    } else {
        let host = pii_graph::fragment::load_host_with_graphs(src, fragments, graph_dirs, reg)
            .map_err(|e| e.to_string())?;
        (host.spec, host.sequence)
    };
    pii_graph::validate(&spec, reg).map_err(|errors| errors.join("; "))?;
    Ok(Checked { spec, woven })
}

/// The `.toml` stems of a directory, sorted, without dotfiles.
fn graph_files(dir: &Path) -> Result<BTreeSet<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            if file.starts_with('.') {
                return None;
            }
            file.strip_suffix(".toml").map(str::to_owned)
        })
        .collect())
}

/// The `name` field of a stored graph body (spec JSON).
fn name_in_body(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v.get("name")?.as_str().map(str::to_owned)
}

/// A graph key that is a sha256 hex digest: a body that could not be named.
fn is_hash(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Every suspended run. The daemon answers when it is up; with no daemon the store is read, since
/// nothing else can hold it. Neither is `Err`: the caller must not guess.
fn suspended_runs(store_path: &Path) -> Result<Vec<Suspended>, String> {
    if let Ok(v) = run_call(Method::RunList, json!({ "status": "suspended" })) {
        let mut out = Vec::new();
        for h in v.as_array().into_iter().flatten() {
            let (Some(run), Some(key)) = (
                h.get("run_id").and_then(Value::as_str),
                h.get("graph").and_then(Value::as_str),
            ) else {
                continue;
            };
            let named = run_call(Method::GraphGet, json!({ "hash": key }))
                .ok()
                .and_then(|g| g.get("body").and_then(Value::as_str).map(str::to_owned))
                .and_then(|body| name_in_body(body.as_bytes()));
            out.push(Suspended {
                run: run.to_owned(),
                graph: named.unwrap_or_else(|| key.to_owned()),
            });
        }
        return Ok(out);
    }
    if !store_path.exists() {
        return Ok(Vec::new());
    }
    let store = Store::open(store_path).map_err(|e| {
        format!(
            "{} is held and the daemon did not answer run.list ({e})",
            store_path.display()
        )
    })?;
    let ids = store
        .runs_by_status(RunStatus::Suspended)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(h) = store.get_run(&id).map_err(|e| e.to_string())? {
            let named = store
                .get_graph(&h.graph)
                .ok()
                .flatten()
                .and_then(|body| name_in_body(&body));
            out.push(Suspended {
                run: h.run.to_string(),
                graph: named.unwrap_or(h.graph),
            });
        }
    }
    Ok(out)
}

/// Trigger as the file states it: every `timer` node's cron, else `manual`.
fn trigger(spec: &GraphSpec) -> String {
    let crons: Vec<&str> = spec
        .nodes
        .iter()
        .filter(|n| n.kind == "timer")
        .map(|n| n.cfg.get("cron").and_then(Value::as_str).unwrap_or("?"))
        .collect();
    if crons.is_empty() {
        String::from("manual")
    } else {
        format!("timer {}", crons.join(" ; "))
    }
}

/// The effect class a node kind writes or touches, as `explain` names it.
fn effect_of(kind: &str) -> Option<&'static str> {
    match kind {
        "post" => Some("post"),
        "recall" | "remember" | "memory" => Some("memory"),
        "record" | "heal_record" => Some("store"),
        _ => None,
    }
}
