//! `pii lane plan <N> [--resume <sid>] [<extra-brief>] [--dry-run]` (spec 317, part 1): the setup half of
//! `scripts/run-batchC-pii.sh`. `plan()` is pure and takes the time and the devbox as inputs; `cmd_lane`
//! is the thin shell that reads the repository and writes the files and streams it returns.
#![allow(
    clippy::redundant_pub_crate,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::too_many_lines,
    clippy::useless_let_if_seq,
    clippy::single_match_else
)] // bin crate: pub(crate) is the reachable form

mod args;
mod brief;
mod checks;
mod clock;
mod devbox;
mod env;
mod facts;
mod git;
mod graph;
mod guard;
mod input;
mod label;
mod preflight;
mod run;
mod shell;
mod transport;
mod wait;

use args::{Args, Resume};
use env::LaneEnv;
use git::RepoSnapshot;
use graph::Agent;
use std::fmt::Write as _;
use std::path::PathBuf;

pub(crate) use shell::cmd_lane;

const USAGE: &str =
    "usage: run-batchC-pii.sh <spec-number> [--resume <session-id>] [extra-brief-file]";
const NODE_TIMEOUT_DEFAULT: &str = "3000000";
const WAIT_MARGIN_DEFAULT: &str = "600";
/// The coder graph's wall limit outlives its node by this much (the script's `+ 300000`).
const COMPOSED_MARGIN: u64 = 300_000;

/// The devbox probes (spec 317 R8). Part 2 ships the ssh implementation.
pub(crate) trait Devbox {
    /// A claude transcript `<sid>.jsonl` exists in the devbox.
    fn resume_exists(&self, sid: &str) -> bool;
    /// `uv` is on the devbox PATH. The error is the probe's output.
    fn has_uv(&self) -> Result<(), String>;
}

/// The dry implementation: answers "found" and "has uv" without touching the devbox.
pub(crate) struct DryDevbox;

impl Devbox for DryDevbox {
    fn resume_exists(&self, _sid: &str) -> bool {
        true
    }
    fn has_uv(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Everything the shell writes: streams, files in write order, and the part-2 fields.
#[derive(Debug, Default)]
pub(crate) struct LanePlan {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) files: Vec<(PathBuf, Vec<u8>)>,
    pub(crate) exit: i32,
    pub(crate) deadline_s: Option<u64>,
    pub(crate) log: PathBuf,
    pub(crate) graph_name: String,
    pub(crate) input: Option<PathBuf>,
    pub(crate) tag: String,
    pub(crate) sid: Option<String>,
}

impl LanePlan {
    fn say(&mut self, line: impl AsRef<str>) {
        self.stdout.extend_from_slice(line.as_ref().as_bytes());
        self.stdout.push(b'\n');
    }

    fn fail(mut self, line: &str, exit: i32) -> Self {
        self.say(line);
        self.exit = exit;
        self
    }
}

/// The spec file the shell's `specs/<N>-*.md` glob picks: the first in bytewise order, a regular file.
fn find_spec<'a>(specs: &'a [(String, bool)], n: &str) -> Option<&'a str> {
    let prefix = format!("{n}-");
    let mut hits: Vec<&(String, bool)> = specs
        .iter()
        .filter(|(name, _)| {
            name.starts_with(&prefix) && name.ends_with(".md") && name.len() >= prefix.len() + 3
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0));
    let (name, is_file) = hits.first()?;
    is_file.then_some(name.as_str())
}

/// A decimal integer as the shell's arithmetic reads one (spec 317 divergence 2).
fn decimal(raw: &str) -> Option<u64> {
    (!raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
        .then(|| raw.parse().ok())
        .flatten()
}

/// The lane's setup, as `scripts/run-batchC-pii.sh` lines 29-218 compute it. Pure: no IO.
pub(crate) fn plan(
    env: &LaneEnv,
    args: &Args,
    snap: &RepoSnapshot,
    dev: &dyn Devbox,
    ts: u64,
) -> LanePlan {
    let mut p = LanePlan::default();
    let Some(n) = args.spec.as_deref() else {
        return p.fail(USAGE, 2);
    };
    let Some(file) = find_spec(&snap.specs, n) else {
        return p.fail(&format!("no spec {n}"), 2);
    };
    let tag = file.strip_suffix(".md").unwrap_or(file).to_owned();
    let spec_rel = format!("specs/{file}");
    if args.resume == Resume::MissingValue {
        p.stderr.extend_from_slice(b"--resume needs a session id\n");
        p.exit = 1;
        return p;
    }
    if !snap.status.is_empty() && !env.allow_dirty {
        p.say("STOPPED: dirty tree (set ALLOW_DIRTY=1 only to resume this spec's own WIP):");
        p.stdout.extend_from_slice(&snap.status);
        p.exit = 8;
        return p;
    }
    let Some(agent) = Agent::from_env(env) else {
        return p.fail("PII_AGENT must be claude or pi", 2);
    };

    let lane_spec = env.spec_lane;
    let lane = if lane_spec { "spec" } else { "coder" };
    let graph_name = if lane_spec {
        "spec".to_owned()
    } else {
        format!("q{n}-{ts}")
    };
    let agent_logd = snap.agent_root.join("_tmp/claude-batchC");
    let logd = snap.root.join("_tmp/claude-batchC");
    let graphs = snap.root.join("_tmp/devbox-test/graphs");
    let log = logd.join(format!("{tag}.{ts}.log"));
    let brief_path = logd.join(format!("brief-{tag}.md"));
    p.say(format!("LANE-GRAPH: {lane}"));

    // PII_NODE_TIMEOUT_MS (coder) or PII_WAIT_MARGIN_S (spec) must be a decimal integer, before anything is written.
    let (name, raw, default) = if lane_spec {
        (
            "PII_WAIT_MARGIN_S",
            env.wait_margin_s.as_deref(),
            WAIT_MARGIN_DEFAULT,
        )
    } else {
        (
            "PII_NODE_TIMEOUT_MS",
            env.node_timeout_ms.as_deref(),
            NODE_TIMEOUT_DEFAULT,
        )
    };
    let raw = raw.unwrap_or(default);
    let Some(num) = decimal(raw) else {
        return p.fail(
            &format!("ABORT: {name} is not a decimal integer: nothing started"),
            2,
        );
    };

    // The brief: written before the `'''` check, so an abort leaves it on disk.
    let mut brief = brief::text(
        &tag,
        n,
        &agent_logd.to_string_lossy(),
        brief::scratch(lane_spec),
    )
    .into_bytes();
    if let Some(extra) = &snap.extra {
        brief.extend_from_slice(b"\n\n");
        match extra {
            Ok(bytes) => brief.extend_from_slice(bytes),
            Err(msg) => p.stderr.extend_from_slice(format!("{msg}\n").as_bytes()),
        }
    }
    p.files.push((brief_path.clone(), brief.clone()));
    if brief.windows(3).any(|w| w == b"'''") {
        return p.fail(
            "ABORT: the brief contains ''' and cannot be written as a TOML literal",
            2,
        );
    }
    let brief_path_text = brief_path.display().to_string();

    // The spec lane's deadline: the graph's wall in seconds plus the margin (read before anything starts).
    let spec_bytes = snap.spec.bytes.as_deref().unwrap_or_default();
    let mut deadline = None;
    let mut wall_text = String::new();
    if lane_spec {
        let Some(wall) = checks::max_wall_ms(spec_bytes).and_then(|w| {
            let v = w.parse::<u64>().ok()?;
            wall_text = w;
            Some(v)
        }) else {
            return p.fail(
                "ABORT: graphs/spec.toml has no numeric max_wall_ms: nothing started",
                2,
            );
        };
        deadline = Some(wall / 1000 + num);
    }

    // The resume handle: claude only; a found session goes into the graph or the input.
    let mut sid: Option<String> = None;
    let mut resume_note: Option<String> = None;
    if let (Resume::Sid(s), "claude") = (&args.resume, agent.name.as_str()) {
        if args.dry {
            p.say(format!(
                "== dry run: session {s} is not checked in the devbox"
            ));
            sid = Some(s.clone());
        } else if dev.resume_exists(s) {
            p.say(format!(
                "== RESUMING claude session {s} (found in the devbox)"
            ));
            sid = Some(s.clone());
        } else {
            let note = format!("RESUME: session {s} not found in the devbox, starting fresh");
            p.say(&note);
            resume_note = Some(note);
        }
    }

    let mut graph_path = None;
    let mut input_path = None;
    if lane_spec {
        if env.graph_edit_ok && args.dry {
            p.say("NOTE: graph edits allowed (PII_GRAPH_EDIT_OK)");
        } else {
            let graph_states = [
                ("spec", &snap.spec),
                ("docs", &snap.docs),
                ("preflight", &snap.preflight),
            ];
            for (name, g) in graph_states {
                if g.differs || g.untracked {
                    return p.fail(
                        &format!("ABORT: graphs/{name}.toml differs from HEAD (the pin protects the reviewed graph): nothing started"),
                        2,
                    );
                }
            }
        }
        let pin = checks::pin(spec_bytes);
        let digest = snap
            .docs
            .bytes
            .as_deref()
            .map(checks::sha256_hex)
            .unwrap_or_default();
        if digest.is_empty() || pin != digest {
            return p.fail(
                &format!("ABORT: graphs/docs.toml digest '{digest}' is not the pin in graphs/spec.toml ('{pin}'): nothing started"),
                2,
            );
        }
        if snap.preflight.bytes.is_none() {
            return p.fail(
                "ABORT: graphs/preflight.toml is missing (the preflight judge graph): nothing started",
                2,
            );
        }
        // The input. The shell truncates the file before the brief is read, so a failure after that leaves it empty;
        // a missing max_micro_usd fails before the truncation and leaves the file alone.
        let input = logd.join(format!("input-{tag}.json"));
        let Some(limit) = checks::limit_micro_usd(spec_bytes) else {
            return p.fail("ABORT: could not build the input", 2);
        };
        let cap = env.max_budget.as_deref().unwrap_or("1.00");
        let docs = brief::docs(n);
        let workdir = snap.agent_root.to_string_lossy().into_owned();
        let provider = if agent.name == "claude" {
            "anthropic"
        } else {
            agent.provider.as_str()
        };
        let json = input::budget(cap, limit)
            .zip(input::universal_newlines(&brief))
            .map(|(budget, text)| {
                input::render(&input::Input {
                    agent: &agent.name,
                    model: &agent.model,
                    provider,
                    spec: &spec_rel,
                    base: snap.head.as_deref().unwrap_or_default(),
                    workdir: &workdir,
                    brief: &text,
                    docs_brief: &docs,
                    budget,
                    sid: sid.as_deref(),
                })
            });
        match json {
            Some(text) => p.files.push((input, text.into_bytes())),
            None => {
                p.files.push((input, Vec::new()));
                return p.fail("ABORT: could not build the input", 2);
            }
        }
        input_path = Some(logd.join(format!("input-{tag}.json")));
    } else {
        let graph_bytes = graph::Coder {
            name: &graph_name,
            wall_ms: num.saturating_add(COMPOSED_MARGIN),
            agent: &agent,
            sid: sid.as_deref(),
            workdir: &snap.agent_root.to_string_lossy(),
            spec: &spec_rel,
            brief: brief::strip_trailing_newlines(&brief),
            timeout_ms: raw,
        }
        .render();
        let path = graphs.join(format!("{graph_name}.toml"));
        p.files.push((path.clone(), graph_bytes));
        graph_path = Some(path);
    }

    // The printed block, then the preflight, then the log (written only once every check has passed).
    p.say(format!(
        "AGENT    : {}   (run by pii, not by the agent CLI directly)",
        agent.name
    ));
    p.say(format!("MODEL    : {}", agent.model));
    p.say(format!("PROVIDER : {}", agent.provider));
    p.say(format!("SPEC     : {spec_rel}"));
    p.say(format!(
        "BRIEF    : {brief_path_text} ({} bytes)",
        brief.len()
    ));
    match (&input_path, &graph_path) {
        (Some(i), _) => p.say(format!("INPUT    : {}", i.display())),
        (None, Some(g)) => p.say(format!("GRAPH    : {}", g.display())),
        (None, None) => {}
    }
    p.say(format!("LOG      : {}", log.display()));
    if let Some(d) = deadline {
        let margin = env.wait_margin_s.as_deref().unwrap_or(WAIT_MARGIN_DEFAULT);
        p.say(format!(
            "WAIT     : up to {d}s (graph max_wall_ms {wall_text} + {margin})"
        ));
    }
    let preflight = (lane_spec && !args.dry).then(|| dev.has_uv());
    if let Some(Err(out)) = &preflight {
        let out = if out.is_empty() {
            "no output"
        } else {
            out.as_str()
        };
        return p.fail(&format!("ABORT: container preflight failed: {out}"), 2);
    }
    let mut log_text = format!(
        "== {tag} via pii: agent={} model={} provider={}\n== graph {graph_name}\n",
        agent.name, agent.model, agent.provider
    );
    if let Some(note) = &resume_note {
        log_text.push_str(note);
        log_text.push('\n');
    }
    writeln!(log_text, "LANE-GRAPH: {lane}").ok();
    p.files.push((log.clone(), log_text.into_bytes()));
    if lane_spec {
        if args.dry {
            p.say("PREFLIGHT: skipped (dry run)");
        } else {
            p.say("PREFLIGHT: uv ok");
        }
    }
    if args.dry {
        p.say("DRY RUN: nothing started, nothing spent");
    }
    p.deadline_s = deadline;
    p.log = log;
    p.graph_name = graph_name;
    p.input = input_path;
    p.tag = tag;
    p.sid = sid;
    p
}

/// Wall-clock seconds for the lane's `TS` (the shell's `date +%s`).
// reason: the lane's TS is the wall clock; the pure core takes it as a parameter.
#[allow(clippy::disallowed_methods)]
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
