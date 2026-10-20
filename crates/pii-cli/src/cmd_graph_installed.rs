//! `pii graph {list|show <name>|explain|edit}` (spec 213): the audit and change surface for the
//! graphs the daemon loads from its installed directory (`<dir>/<name>.toml`). The repo copy is
//! `<repo>/<name>.toml`. `list`, `show` and `explain` only read files; the armed-run count is asked
//! of the daemon, or read from the store when no daemon holds it. `edit` lints first, moves the
//! previous file aside, and writes atomically: no verb deletes a graph.

// reason: bin crate: pub(crate) is the reachable form, and the module is private.
#![allow(clippy::redundant_pub_crate)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::{PortName, RunStatus};
use pii_core::{CapKind, Clock as _};
use pii_graph::GraphSpec;
use pii_graph::fragment::{Inlined, sha256_hex};
use pii_graph::{NodeRegistry, NodeSpec};
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
    list(&o)
}

/// `pii graph explain <name>`.
pub(crate) fn cmd_explain(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    explain(&o)
}

/// `pii graph edit <name> --file <path> [--force]`.
pub(crate) fn cmd_edit(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    edit(&o)
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
    let dir = match installed_dir(&o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    match std::fs::read_to_string(dir.join(format!("{name}.toml"))) {
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

/// The default installed dir: the runtime graphs dir the daemon reads, else the installer's state
/// dir. The runtime entry is a symlink into the state dir and exists only while the daemon runs.
fn default_installed_dir() -> Option<PathBuf> {
    let runtime = pii_uds::runtime_dir("default").map(|r| r.join("graphs"));
    if runtime.as_ref().is_some_and(|p| p.is_dir()) {
        return runtime;
    }
    pii_core::env_var("HOME")
        .map(|h| PathBuf::from(h).join(".local/state/pii/graphs"))
        .filter(|p| p.is_dir())
        .or(runtime)
}

/// The installed directory, canonical so a runtime symlink resolves to the state dir it names.
fn installed_dir(o: &Opts) -> Result<PathBuf, ExitCode> {
    let dir = o.dir.clone().or_else(default_installed_dir);
    let Some(dir) = dir else {
        return Err(error("no installed graphs dir (use --dir)"));
    };
    std::fs::canonicalize(&dir)
        .map_err(|e| error(format!("no installed graphs at {}: {e}", dir.display())))
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
fn check(src: &str, fragments: &Path, reg: &NodeRegistry) -> Result<Checked, String> {
    let (spec, woven) = if pii_graph::fragment::declares_fragment(src) {
        let fragment = pii_graph::fragment::check_fragment(src, reg).map_err(|e| e.to_string())?;
        (fragment.spec, Vec::new())
    } else {
        let host = pii_graph::fragment::weave(src, fragments, reg).map_err(|e| e.to_string())?;
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

/// `pii graph list`: one row per installed graph (every repo graph under `--all`).
fn list(o: &Opts) -> ExitCode {
    let installed = match installed_dir(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let reg = registry();
    let mut names = match graph_files(&installed) {
        Ok(n) => n,
        Err(e) => return error(e),
    };
    if o.all {
        names.extend(graph_files(&repo).unwrap_or_default());
    }
    let armed = suspended_runs(&store_path(o, &installed));
    if let Err(e) = &armed {
        eprintln!("armed runs unknown: {e}");
    }
    // A run whose graph could not be named may belong to any row, so every count says so.
    let unnamed = armed
        .as_ref()
        .is_ok_and(|runs| runs.iter().any(|r| is_hash(&r.graph)));
    let rows: Vec<Row> = names
        .iter()
        .filter_map(|name| row_of(name, &installed, &repo, &reg, &armed))
        .collect();
    if o.json {
        print_json(&rows);
    } else {
        print_text(&rows, unnamed);
    }
    ExitCode::SUCCESS
}

/// The row for one name, from whichever of its two copies exists; `None` when neither does.
fn row_of(
    name: &str,
    installed: &Path,
    repo: &Path,
    reg: &NodeRegistry,
    armed: &Result<Vec<Suspended>, String>,
) -> Option<Row> {
    let inst = std::fs::read_to_string(installed.join(format!("{name}.toml"))).ok();
    let rep = std::fs::read_to_string(repo.join(format!("{name}.toml"))).ok();
    let (shown, drift) = match (&inst, &rep) {
        (Some(i), Some(r)) if sha256_hex(i.as_bytes()) == sha256_hex(r.as_bytes()) => {
            (i, Drift::Same)
        }
        (Some(i), Some(_)) => (i, Drift::Changed),
        (Some(i), None) => (i, Drift::InstalledOnly),
        (None, Some(r)) => (r, Drift::RepoOnly),
        (None, None) => return None,
    };
    // A repo-only graph is not running anywhere, so it has no armed runs to count.
    let armed = match (armed, drift) {
        (Ok(runs), Drift::Same | Drift::Changed | Drift::InstalledOnly) => {
            Some(runs.iter().filter(|r| r.graph == name).count())
        }
        _ => None,
    };
    Some(Row {
        name: name.to_owned(),
        sha: sha256_hex(shown.as_bytes()),
        checked: check(shown, &fragments_dir(installed, repo), reg),
        armed,
        drift,
    })
}

fn print_text(rows: &[Row], unnamed: bool) {
    for r in rows {
        let (nodes, trig) = r.checked.as_ref().map_or_else(
            |_| (String::from("unloadable"), String::from("-")),
            |c| (c.spec.nodes.len().to_string(), trigger(&c.spec)),
        );
        let armed = match (r.armed, r.drift) {
            (_, Drift::RepoOnly) => String::from("-"),
            (Some(n), _) if unnamed => format!("{n}+unnamed"),
            (Some(n), _) => n.to_string(),
            (None, _) => String::from("?"),
        };
        println!(
            "{} · {} · nodes {nodes} · {trig} · armed {armed} · {}",
            r.name,
            r.sha.get(..12).unwrap_or(r.sha.as_str()),
            r.drift.label(),
        );
    }
}

fn print_json(rows: &[Row]) {
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "sha256": r.sha,
                "nodes": r.checked.as_ref().ok().map(|c| c.spec.nodes.len()),
                "trigger": r.checked.as_ref().ok().map(|c| trigger(&c.spec)),
                "error": r.checked.as_ref().err(),
                "armed": r.armed,
                "drift": r.drift.label(),
            })
        })
        .collect();
    println!("{}", Value::Array(out));
}

/// The node ids where two specs disagree: changed, only in the installed copy, only in the repo's.
fn node_diff<'a>(
    installed: &'a GraphSpec,
    repo: &'a GraphSpec,
) -> (Vec<&'a str>, Vec<&'a str>, Vec<&'a str>) {
    let i: BTreeMap<&str, &NodeSpec> = installed.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let r: BTreeMap<&str, &NodeSpec> = repo.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let changed = i
        .iter()
        .filter(|(id, n)| r.get(*id).is_some_and(|rn| rn != *n))
        .map(|(id, _)| *id)
        .collect();
    let only_i = i
        .keys()
        .filter(|id| !r.contains_key(*id))
        .copied()
        .collect();
    let only_r = r
        .keys()
        .filter(|id| !i.contains_key(*id))
        .copied()
        .collect();
    (changed, only_i, only_r)
}

/// `pii graph explain <name>`: rendered from the installed file's own fields, never paraphrased.
fn explain(o: &Opts) -> ExitCode {
    let name = match one_name(o) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let installed = match installed_dir(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let path = installed.join(format!("{name}.toml"));
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return error(format!("{name} is not installed: {e}")),
    };
    let reg = registry();
    let sha = sha256_hex(src.as_bytes());
    let checked = match check(&src, &fragments_dir(&installed, &repo), &reg) {
        Ok(c) => c,
        Err(e) => return error(format!("{name} does not load: {e}")),
    };
    let spec = &checked.spec;

    println!("graph {name}");
    println!("file {}", path.display());
    println!("sha256 {sha}");
    println!("TRIGGER {}", trigger(spec));
    print_nodes(spec, &reg);
    print_effects(spec);
    print_capabilities(spec);
    print_woven(&checked.woven);
    println!("DRIFT {}", drift_verdict(&name, &repo, &sha, spec, &reg));
    ExitCode::SUCCESS
}

/// The nodes in dependency order, each with its kind, its declared ports and its outgoing edges.
fn print_nodes(spec: &GraphSpec, reg: &NodeRegistry) {
    println!("NODES in dependency order");
    for id in pii_graph::render::topo_order(spec) {
        let Some(node) = spec.node(&id) else { continue };
        let ports = reg
            .factory(&node.kind)
            .ok()
            .and_then(|f| f.ports(&node.cfg).ok())
            .map_or_else(
                || String::from("?"),
                |ps| {
                    ps.iter()
                        .map(PortName::as_str)
                        .collect::<Vec<&str>>()
                        .join(",")
                },
            );
        let edges: Vec<String> = spec
            .outbound(&id)
            .into_iter()
            .filter_map(|i| spec.edges.get(i))
            .map(|e| format!("{}->{}", e.port.as_str(), e.to.as_str()))
            .collect();
        println!(
            "  {id}  kind {}  ports {ports}  {}",
            node.kind,
            edges.join(" ")
        );
    }
}

/// What the graph writes or touches, grouped by effect class, with the node ids that do it.
fn print_effects(spec: &GraphSpec) {
    println!("EFFECTS");
    let mut effects: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &spec.nodes {
        if let Some(label) = effect_of(&node.kind) {
            effects.entry(label).or_default().push(node.id.as_str());
        }
    }
    if effects.is_empty() {
        println!("  none");
    }
    for (label, ids) in &effects {
        println!("  {label}  {}", ids.join(", "));
    }
}

/// The graph's capability grant, then the channel, agent and model each node names.
fn print_capabilities(spec: &GraphSpec) {
    println!("CAPABILITIES");
    let graph_caps: Vec<&str> = CapKind::from_bits_truncate(spec.caps)
        .iter_names()
        .map(|(n, _)| n)
        .collect();
    if graph_caps.is_empty() {
        println!("  graph none");
    } else {
        println!("  graph {}", graph_caps.join(" | "));
    }
    for node in &spec.nodes {
        for key in ["channel", "agent", "model"] {
            if let Some(v) = node.cfg.get(key).and_then(Value::as_str) {
                println!("  {key} {v} (node {})", node.id);
            }
        }
    }
}

/// The fragment nodes woven into the graph, with the fragment and node each came from.
fn print_woven(woven: &[Inlined]) {
    println!("WOVEN");
    if woven.is_empty() {
        println!("  none");
    }
    for w in woven {
        println!(
            "  {} {} origin {} hook {} order {}",
            w.surface, w.node, w.origin, w.attach, w.order
        );
    }
}

/// How the installed copy compares with the repo's, naming the node ids that differ.
fn drift_verdict(
    name: &str,
    repo: &Path,
    sha: &str,
    spec: &GraphSpec,
    reg: &NodeRegistry,
) -> String {
    let Ok(rep_src) = std::fs::read_to_string(repo.join(format!("{name}.toml"))) else {
        return String::from("installed-only (no copy in the repo)");
    };
    let rep_sha = sha256_hex(rep_src.as_bytes());
    if rep_sha == sha {
        return String::from("same (the repo copy has the same sha256)");
    }
    let rep = match check(&rep_src, &fragments_dir(repo, repo), reg) {
        Ok(c) => c.spec,
        Err(e) => return format!("repo copy does not load: {e}"),
    };
    let (changed, only_i, only_r) = node_diff(spec, &rep);
    if changed.is_empty() && only_i.is_empty() && only_r.is_empty() {
        return format!("differs from the repo copy outside its nodes (repo sha256 {rep_sha})");
    }
    format!(
        "differs from the repo copy (repo sha256 {rep_sha}); changed nodes: [{}]; \
         only installed: [{}]; only repo: [{}]",
        changed.join(", "),
        only_i.join(", "),
        only_r.join(", ")
    )
}

/// `pii graph edit <name> --file <path> [--force]`: lint, refuse armed runs, move the old file
/// aside, write atomically, re-lint the installed copy, and report the hashes.
fn edit(o: &Opts) -> ExitCode {
    let name = match one_name(o) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let Some(file) = o.file.clone() else {
        return usage();
    };
    let installed = match installed_dir(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let target = installed.join(format!("{name}.toml"));
    let before = match std::fs::read(&target) {
        Ok(b) => b,
        Err(e) => return error(format!("{name} is not installed: {e}")),
    };
    let new = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => return error(format!("read {}: {e}", file.display())),
    };
    let armed = match gate(o, &name, &file, &installed, &new) {
        Ok(armed) => armed,
        Err(code) => return code,
    };

    let before_sha = sha256_hex(&before);
    let after_sha = sha256_hex(new.as_bytes());
    if before_sha == after_sha {
        println!("unchanged {name}: sha256 {before_sha}");
        return ExitCode::SUCCESS;
    }

    // The previous file moves aside under an epoch name. An existing name is refused, never
    // overwritten.
    let epoch = pii_core::SystemClock.now_ms();
    let prev = installed.join(format!("{name}.toml.prev-{epoch}"));
    if prev.exists() {
        return error(format!("refused: {} already exists", prev.display()));
    }
    let tmp = installed.join(format!(".{name}.toml.tmp-{}", std::process::id()));
    if let Err(e) = install(&target, &prev, &tmp, new.as_bytes()) {
        return error(e);
    }
    if !armed.is_empty() {
        println!("forced past armed run(s): {}", armed.join(", "));
    }
    let reread_sha = match relint_installed(&target, &installed, &repo) {
        Ok(sha) => sha,
        Err(e) => {
            return error(format!(
                "installed copy fails lint after the edit: {e}; previous file kept at {}",
                prev.display()
            ));
        }
    };
    report_edit(&name, &before_sha, &reread_sha, &prev, &repo);
    ExitCode::SUCCESS
}

/// Steps (a) and (b) of an edit. The new file must lint as `pii graph lint` would lint it, and no
/// suspended run of this graph (nor one that cannot be named) may be running unless `--force`
/// is given. Returns the run ids that `--force` let through.
fn gate(
    o: &Opts,
    name: &str,
    file: &Path,
    installed: &Path,
    new: &str,
) -> Result<Vec<String>, ExitCode> {
    let lint_fragments = file
        .parent()
        .map_or_else(|| PathBuf::from("fragments"), |p| p.join("fragments"));
    if let Err(e) = check(new, &lint_fragments, &registry()) {
        fail(
            "graph",
            format!("refused: {} fails lint: {e}", file.display()),
        );
        return Err(ExitCode::from(3));
    }
    let blocking: Vec<String> = match suspended_runs(&store_path(o, installed)) {
        Ok(runs) => runs
            .into_iter()
            .filter(|r| r.graph == name || is_hash(&r.graph))
            .map(|r| r.run)
            .collect(),
        Err(e) => {
            return Err(error(format!(
                "refused: cannot tell whether {name} has armed runs: {e}"
            )));
        }
    };
    if blocking.is_empty() || o.force {
        return Ok(blocking);
    }
    fail(
        "graph",
        format!(
            "refused: {name} has armed run(s) {}; pass --force to edit under them",
            blocking.join(", ")
        ),
    );
    Err(ExitCode::from(3))
}

/// Step (d): the new bytes go to `tmp` first, so a failure there leaves the installed copy alone.
/// Then the old file moves to `prev`, and `tmp` is renamed into `target`. A failed rename puts the
/// previous file back, so the installed name is never left empty.
fn install(target: &Path, prev: &Path, tmp: &Path, new: &[u8]) -> Result<(), String> {
    write_new(tmp, new).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(target, prev).map_err(|e| format!("move {} aside: {e}", target.display()))?;
    if let Err(e) = std::fs::rename(tmp, target) {
        let restored = std::fs::rename(prev, target).is_ok();
        return Err(format!(
            "install {}: {e}; previous restored: {restored}; new bytes are in {}",
            target.display(),
            tmp.display()
        ));
    }
    Ok(())
}

/// Re-reads the installed copy and lints it again. Returns its sha256 on success.
fn relint_installed(target: &Path, installed: &Path, repo: &Path) -> Result<String, String> {
    let src = std::fs::read_to_string(target).map_err(|e| format!("re-read: {e}"))?;
    check(&src, &fragments_dir(installed, repo), &registry())?;
    Ok(sha256_hex(src.as_bytes()))
}

/// The edit's report: both hashes, the previous file, and the drift state against the repo.
fn report_edit(name: &str, before_sha: &str, after_sha: &str, prev: &Path, repo: &Path) {
    println!("edited {name}");
    println!("before-sha {before_sha}");
    println!("after-sha {after_sha}");
    println!("moved aside {}", prev.display());
    match std::fs::read(repo.join(format!("{name}.toml"))) {
        Ok(r) if sha256_hex(&r) == after_sha => println!("drift same (repo copy matches)"),
        Ok(_) => println!("drift DRIFT (repo copy differs)"),
        Err(_) => println!("drift installed-only (no copy in the repo)"),
    }
}

/// Creates `tmp` exclusively and writes `bytes` into it, flushed to disk.
fn write_new(tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()
}
