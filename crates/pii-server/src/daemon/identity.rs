//! What this process IS (spec 99): the identity fields of `sys.status`.
//!
//! Resolved store, mounts, build, config and counts, so a divergence between
//! two daemons is visible at a glance. Everything here is read from the live process —
//! resolved paths and persisted values, never a profile name or a constant.

use std::path::Path;

use pii_observe::{Health, SubsystemHealth};
use pii_proto::sys::{
    BuildIdentity, ConfigIdentity, Counts, MountStatus, StatusResult, StoreStatus, SubsystemStatus,
};
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

use super::status::DaemonContext;
use crate::Engine;

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
    out.mounts.push(workspace_mount(engine));
    out.build = BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        revision: env!("PII_GIT_REV").to_owned(),
        target: env!("PII_TARGET").to_owned(),
    };
    let mut reports: Vec<SubsystemHealth> = vec![engine.store.health(), engine.memory.health()];
    if let Some(ctx) = ctx {
        counts.cron_entries = ctx.cron.lock().len() as u64;
        out.mounts.extend(daemon_mounts(ctx));
        out.config = ConfigIdentity {
            path: ctx.config_path.display().to_string(),
            sha256: std::fs::read(&ctx.config_path)
                .ok()
                .map(|b| hex(&Sha256::digest(b))),
        };
        reports.push(ctx.net.health());
        reports.extend(ctx.workers.lock().values().map(Health::health));
        reports.extend(ctx.channels.lock().values().map(Health::health));
    }
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
        // The memory index file persists no format version (only the main
        // store does), so none is reported rather than an invented one.
        format_version: None,
        last_write_ms,
        entries: std::iter::once(("docs".to_owned(), engine.memory.n_docs().unwrap_or(0)))
            .collect(),
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
