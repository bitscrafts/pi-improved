//! Spec 242: boot recovery of the agents a dead daemon left running.
//!
//! A daemon that dies mid-agent leaves the agent running with no owner. Before the runs it left `Running` are closed,
//! each agent it started is killed: a container by its `io.pii` and `pii.run` labels, a host process by the pid it
//! journalled while `/proc` still shows that pid running the same program. Every reap is journalled on its run as
//! `coder.orphan_reaped`, and the run's failure text says so when an agent was killed.

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{NodeId, PiiError, RunEvent, RunId, RunStatus};
use pii_core::{Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_observe::names;
use pii_store::Store;
use serde_json::{Value, json};

use crate::Engine;
use crate::node_kinds::nodes::coder_runtime::{ContainerRuntime, Listed};
use crate::node_kinds::nodes::coder_runtime_api::PodmanApi;
use crate::node_kinds::nodes::coder_runtime_cli::PodmanCli;

/// The process facts boot recovery reads and signals (spec 242 R1, R2). A seam, so tests inject a table.
pub trait ProcProbe {
    /// The command line of process `pid`, its arguments joined by spaces; `None` when no such process is running.
    fn cmdline(&self, pid: u32) -> Option<String>;

    /// Kills the process group whose leader is `pid`.
    ///
    /// # Errors
    /// [`PiiError`] when the kill fails.
    fn kill_group(&self, pid: u32) -> Result<(), PiiError>;
}

/// The real process table: `/proc/<pid>/cmdline` and `killpg`.
pub struct ProcFs;

impl ProcProbe for ProcFs {
    fn cmdline(&self, pid: u32) -> Option<String> {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let args: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect();
        (!args.is_empty()).then(|| args.join(" "))
    }

    fn kill_group(&self, pid: u32) -> Result<(), PiiError> {
        pii_core::kill_group(pid)
    }
}

/// One reap, as its run journals it (spec 242 R3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaped {
    /// The run the agent belonged to.
    pub run: String,
    /// The node that spawned the agent; the reap is journalled on it.
    pub node: NodeId,
    /// `container` or `process`.
    pub kind: &'static str,
    /// The container's id, or the pid. Empty when the container listing itself failed.
    pub id: String,
    /// `killed`, `already_gone` or `failed`.
    pub result: &'static str,
}

impl Reaped {
    /// The `coder.orphan_reaped` event this reap journals on its run.
    #[must_use]
    pub fn event(&self) -> RunEvent {
        RunEvent::NodeObserved {
            node: self.node.clone(),
            kind: names::CODER_ORPHAN_REAPED.to_owned(),
            fields: json!({
                "run": self.run,
                "kind": self.kind,
                "id": self.id,
                "result": self.result,
            }),
        }
    }
}

/// Finds and kills the agents the dead daemon started, for every run in `store` that is `Running` (spec 242 R1, R2,
/// R6).
///
/// A run is searched only when it journalled more `coder.spawned` than `coder.exited` events. Containers are listed at
/// most once, and only when a run needs it. A failed reap is returned as `failed` and boot goes on (R3).
pub fn reap_orphans(
    store: &Store,
    ctx: &Ctx,
    runtime: &dyn ContainerRuntime,
    proc: &dyn ProcProbe,
) -> Vec<Reaped> {
    let Ok(running) = store.runs_by_status(RunStatus::Running) else {
        return Vec::new();
    };
    let mut reaped = Vec::new();
    let mut listing: Option<Result<Vec<Listed>, PiiError>> = None;
    for run in running {
        let Ok(journal) = store.read_journal(&run) else {
            continue;
        };
        let coder: Vec<(NodeId, String, Value)> = journal
            .into_iter()
            .filter_map(|(_, event)| match event {
                RunEvent::NodeObserved { node, kind, fields } if kind.starts_with("coder.") => {
                    Some((node, kind, fields))
                }
                _ => None,
            })
            .collect();
        let spawns = coder.iter().filter(|(_, kind, _)| kind == "coder.spawned");
        let Some((node, _, _)) = spawns.clone().next() else {
            continue;
        };
        let exits: Vec<&Value> = coder
            .iter()
            .filter(|(_, kind, _)| kind == "coder.exited")
            .map(|(_, _, fields)| fields)
            .collect();
        if spawns.count() <= exits.len() {
            continue;
        }
        let run_id = run.as_str().to_owned();
        let listed = listing.get_or_insert_with(|| runtime.list_all(ctx));
        reap_containers(&mut reaped, &run_id, node, listed, ctx, runtime);
        reap_processes(&mut reaped, &run_id, &coder, &exits, proc);
    }
    reaped
}

/// Removes the containers labelled for `run_id` (R1a, R2), or journals one `failed` reap when the listing failed.
fn reap_containers(
    reaped: &mut Vec<Reaped>,
    run_id: &str,
    node: &NodeId,
    listed: &Result<Vec<Listed>, PiiError>,
    ctx: &Ctx,
    runtime: &dyn ContainerRuntime,
) {
    let Ok(containers) = listed else {
        reaped.push(Reaped {
            run: run_id.to_owned(),
            node: node.clone(),
            kind: "container",
            id: String::new(),
            result: "failed",
        });
        return;
    };
    for container in containers.iter().filter(|c| is_ours(c, run_id)) {
        let result = match runtime.force_remove(ctx, &container.id) {
            Ok(true) => "killed",
            Ok(false) => "already_gone",
            Err(_) => "failed",
        };
        reaped.push(Reaped {
            run: run_id.to_owned(),
            node: node.clone(),
            kind: "container",
            id: container.id.clone(),
            result,
        });
    }
}

/// Kills the host agents `run_id` spawned whose pid no `coder.exited` accounts for (R1b, R2). A pid whose command line
/// no longer names the spawned program is journalled `already_gone` and not signalled.
fn reap_processes(
    reaped: &mut Vec<Reaped>,
    run_id: &str,
    coder: &[(NodeId, String, Value)],
    exits: &[&Value],
    proc: &dyn ProcProbe,
) {
    let exited_pids: Vec<i64> = exits
        .iter()
        .filter_map(|fields| fields.get("pid").and_then(Value::as_i64))
        .collect();
    for (node, _, fields) in coder.iter().filter(|(_, kind, _)| kind == "coder.spawned") {
        if fields.get("runtime").and_then(Value::as_str) != Some("host") {
            continue;
        }
        let Some(pid) = fields
            .get("pid")
            .and_then(Value::as_i64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
        else {
            continue;
        };
        if exited_pids.contains(&i64::from(pid)) {
            continue;
        }
        let argv0 = fields
            .get("argv0")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match proc.cmdline(pid) {
            Some(cmdline) if runs_program(&cmdline, argv0) => match proc.kill_group(pid) {
                Ok(()) => "killed",
                Err(_) => "failed",
            },
            _ => "already_gone",
        };
        reaped.push(Reaped {
            run: run_id.to_owned(),
            node: node.clone(),
            kind: "process",
            id: pid.to_string(),
            result,
        });
    }
}

/// The boot recovery the daemon runs before it closes the runs its predecessor left `Running` (spec 242).
///
/// The socket runtime when its socket resolves, else the podman CLI. Both reach the same engine, so either lists every
/// container a run started.
pub fn recover_at_boot(engine: &Engine) {
    let ctx = Ctx::with_observer(
        CapSet {
            kinds: CapKind::PROC,
            ..CapSet::default()
        },
        Budget::unlimited(),
        Arc::new(NullObserver),
        TraceCtx {
            run: RunId::from("boot"),
            span: 0,
        },
    );
    if let Ok(socket) = pii_tools::sandbox::runtime_socket() {
        engine.recover_boot(&ctx, &PodmanApi { socket }, &ProcFs);
    } else {
        let cli = PodmanCli {
            podman: PathBuf::from("podman"),
            env: ["PATH", "HOME"]
                .iter()
                .filter_map(|k| pii_core::env_var(k).map(|v| ((*k).to_owned(), v)))
                .collect(),
            runs_dir: None,
        };
        engine.recover_boot(&ctx, &cli, &ProcFs);
    }
}

/// True when the container carries `io.pii=1` and `pii.run=<run>`. Both labels: a container of ours from another run,
/// or one that merely names this run, is not ours to remove.
fn is_ours(container: &Listed, run: &str) -> bool {
    let label = |key: &str| {
        container
            .labels
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    label("io.pii") == Some("1") && label("pii.run") == Some(run)
}

/// True when `cmdline` runs the program the agent was spawned as. A recycled pid runs something else and is left alone.
fn runs_program(cmdline: &str, argv0: &str) -> bool {
    !argv0.is_empty()
        && cmdline
            .strip_prefix(argv0)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}
