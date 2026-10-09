//! What this process IS (spec 99): the identity fields of `sys.status`.
//!
//! Resolved store, mounts, build, config and counts, so a divergence between
//! two daemons is visible at a glance. Everything here is read from the live process —
//! resolved paths and persisted values, never a profile name or a constant.

use std::collections::BTreeMap;
use std::path::Path;

use pii_observe::{Health, SubsystemHealth};
use pii_proto::sys::{
    BuildIdentity, ConfigIdentity, Counts, MountStatus, StatusResult, StoreStatus, SubsystemStatus,
};
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

use super::status::{ChannelRuntime, DaemonContext};
use crate::Engine;

/// The subsystem groups `sys.status` aggregates, in report order. A group
/// with no live members (no daemon, no workers configured) is still
/// registered; `registry_is_pinned_by_name` fails if one is dropped.
const GROUPS: [&str; 5] = ["store", "memory", "net", "worker", "channels"];

/// Every health-reporting subsystem the daemon aggregates, as
/// `&dyn Health` under its group name.
struct Registry<'a>(Vec<(&'static str, Vec<&'a dyn Health>)>);

impl<'a> Registry<'a> {
    fn build(
        engine: &'a Engine,
        net: Option<&'a pii_net::NetService>,
        workers: Option<&'a BTreeMap<String, pii_worker::Supervisor>>,
        channels: Option<&'a BTreeMap<String, ChannelRuntime>>,
    ) -> Self {
        Self(vec![
            (GROUPS[0], vec![&*engine.store as &dyn Health]),
            (GROUPS[1], vec![&*engine.memory as &dyn Health]),
            (
                GROUPS[2],
                net.map(|n| n as &dyn Health).into_iter().collect(),
            ),
            (
                GROUPS[3],
                workers
                    .into_iter()
                    .flat_map(|m| m.values().map(|w| w as &dyn Health))
                    .collect(),
            ),
            (
                GROUPS[4],
                channels
                    .into_iter()
                    .flat_map(|m| m.values().map(|c| c as &dyn Health))
                    .collect(),
            ),
        ])
    }

    #[cfg(test)]
    fn group_names(&self) -> Vec<&'static str> {
        self.0.iter().map(|(g, _)| *g).collect()
    }

    fn reports(&self) -> Vec<SubsystemHealth> {
        self.0
            .iter()
            .flat_map(|(_, members)| members.iter().map(|h| h.health()))
            .collect()
    }
}

/// Fills the identity fields of `out` from `engine` (and `ctx` when a daemon
/// is attached). The pre-existing fields are left untouched.
pub fn attach(engine: &Engine, ctx: Option<&DaemonContext>, out: &mut StatusResult) {
    let info = engine.store.info().ok();
    let mut counts = Counts {
        memory_docs: out.memory.docs,
        approvals_pending: engine
            .approvals
            .pending_approvals()
            .map_or(0, |a| a.len() as u64),
        ..Counts::default()
    };
    if let Some(i) = info {
        counts.runs = i.tables.get("run").copied().unwrap_or(0);
        counts.journal_events = i.tables.get("run_journal").copied().unwrap_or(0);
        out.stores.push(StoreStatus {
            name: "main".into(),
            path: shown(&i.path),
            bytes: i.bytes,
            format_version: Some(i.format_version),
            last_write_ms: i.last_write_ms,
            entries: i.tables,
        });
    }
    out.stores.push(memory_store(engine));
    let dir = engine.store.path().parent().map(Path::to_path_buf);
    if let Some(dir) = dir {
        let aux = |name: &str, file: &str, v: Option<u64>| aux_store(name, &dir.join(file), v);
        out.stores.push(aux(
            "approval",
            "approval.redb",
            engine.approvals.format_version().ok(),
        ));
        out.stores
            .push(aux("heal", "heal.redb", engine.heals.format_version().ok()));
        if let Some(ctx) = ctx {
            out.stores.push(aux_store(
                "channel",
                &ctx.runtime_dir.join("channel.redb"),
                ctx.channel_store.format_version().ok(),
            ));
            out.stores.push(aux_store(
                "timer",
                &ctx.runtime_dir.join("timer.redb"),
                ctx.timer_store.format_version().ok(),
            ));
        }
    }
    out.mounts.push(workspace_mount(engine));
    out.build = BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        revision: env!("PII_GIT_REV").to_owned(),
        target: env!("PII_TARGET").to_owned(),
    };
    let workers = ctx.map(|c| c.workers.lock());
    let channels = ctx.map(|c| c.channels.lock());
    let registry = Registry::build(
        engine,
        ctx.map(|c| &*c.net),
        workers.as_deref(),
        channels.as_deref(),
    );
    if let Some(ctx) = ctx {
        counts.cron_entries = ctx.cron.lock().len() as u64;
        out.mounts.extend(daemon_mounts(ctx));
        out.config = ConfigIdentity {
            path: ctx.config_path.display().to_string(),
            sha256: std::fs::read(&ctx.config_path)
                .ok()
                .map(|b| hex(&Sha256::digest(b))),
        };
    }
    let reports = registry.reports();
    drop(registry);
    drop((workers, channels));
    out.counts = counts;
    out.subsystems = reports.into_iter().map(wire).collect();
}

fn wire(h: SubsystemHealth) -> SubsystemStatus {
    SubsystemStatus {
        name: h.name,
        state: format!("{:?}", h.state).to_lowercase(),
        detail: h.detail,
        identity: h.identity,
    }
}

fn memory_store(engine: &Engine) -> StoreStatus {
    let (bytes, last_write_ms) = pii_store::file_stats(engine.memory.path());
    StoreStatus {
        name: "memory".into(),
        path: shown(engine.memory.path()),
        bytes,
        format_version: engine.memory.format_version().ok(),
        last_write_ms,
        entries: std::iter::once(("docs".to_owned(), engine.memory.n_docs().unwrap_or(0)))
            .collect(),
    }
}

/// A store that carries only a path, size and persisted format version.
fn aux_store(name: &str, path: &Path, format_version: Option<u64>) -> StoreStatus {
    let (bytes, last_write_ms) = pii_store::file_stats(path);
    StoreStatus {
        name: name.into(),
        path: shown(path),
        bytes,
        format_version,
        last_write_ms,
        entries: BTreeMap::new(),
    }
}

fn workspace_mount(engine: &Engine) -> MountStatus {
    let policy = match engine.cfg.sandbox {
        SandboxProfileName::Off => "sandbox=off: no filesystem confinement applied",
        SandboxProfileName::Workspace => "sandbox=workspace: read-write granted by landlock",
    };
    MountStatus {
        name: "workspace".into(),
        root: shown(&engine.cfg.workspace),
        mode: "rw".into(),
        policy: policy.into(),
    }
}

fn daemon_mounts(ctx: &DaemonContext) -> [MountStatus; 2] {
    [
        MountStatus {
            name: "channels_scope".into(),
            root: shown(&ctx.channels_scope),
            mode: "rw".into(),
            policy: "channel fs scope: <runtime_dir>/channels, one subdir per channel".into(),
        },
        MountStatus {
            name: "runtime_dir".into(),
            root: shown(&ctx.runtime_dir),
            mode: "rw".into(),
            policy: "daemon runtime dir: control socket, stores, crash reports".into(),
        },
    ]
}

/// The resolved absolute path. A path that cannot be resolved is shown as
/// `<unresolved: path>` — never passed off as a resolved root.
fn shown(p: &Path) -> String {
    dunce::canonicalize(p).map_or_else(
        |e| format!("<unresolved ({e}): {}>", p.display()),
        |c| c.display().to_string(),
    )
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::EngineConfig;

    /// The aggregator's registered set, pinned by NAME (spec 100 R2): dropping
    /// or renaming a registration changes this list and fails the test.
    #[test]
    fn registry_is_pinned_by_name() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "registry-").unwrap();
        let engine = Engine::new(EngineConfig {
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!(
                "scripted:{}/../../fixtures/phase2/hello.json",
                env!("CARGO_MANIFEST_DIR")
            ),
            sandbox: SandboxProfileName::Off,
            route_policy: BTreeMap::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            graph_dir: dir.path().join("graphs"),
        })
        .unwrap();
        let registry = Registry::build(&engine, None, None, None);
        assert_eq!(
            registry.group_names(),
            ["store", "memory", "net", "worker", "channels"]
        );
        let names: Vec<_> = registry.reports().into_iter().map(|h| h.name).collect();
        assert_eq!(
            names,
            ["store", "memory"],
            "bare engine reports store+memory"
        );
    }
}
