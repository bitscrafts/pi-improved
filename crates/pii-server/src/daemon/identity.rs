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
use sha2::{Digest, Sha256};

use super::status::{ChannelRuntime, DaemonContext};
use crate::Engine;

/// The subsystem groups `sys.status` aggregates, in report order. A group
/// with no live members (no daemon, no workers configured) is still
/// registered; `registry_is_pinned_by_name` fails if one is dropped.
const GROUPS: [&str; 9] = [
    "store",
    "memory",
    "net",
    "worker",
    "channels",
    "model_tiers",
    "hal",
    "wasm",
    "learn",
];

/// Every health-reporting subsystem the daemon aggregates, as
/// `&dyn Health` under its group name.
struct Registry<'a>(Vec<(&'static str, Vec<&'a dyn Health>)>);

impl<'a> Registry<'a> {
    // reason: one optional member source per group; a struct would only rename the list.
    #[allow(clippy::too_many_arguments)]
    fn build(
        engine: &'a Engine,
        net: Option<&'a pii_net::NetService>,
        workers: Option<&'a BTreeMap<String, pii_worker::Supervisor>>,
        channels: Option<&'a BTreeMap<String, ChannelRuntime>>,
        hal: Option<&'a pii_hal::Hal>,
        wasm: Option<&'a pii_wasm::WasmHost>,
        learn: Option<&'a pii_learn::LearnHealth>,
        entities: Option<&'a pii_memory::entity::EntityHealth>,
    ) -> Self {
        Self(vec![
            (GROUPS[0], vec![&*engine.store as &dyn Health]),
            (
                GROUPS[1],
                std::iter::once(&*engine.memory as &dyn Health)
                    .chain(entities.map(|e| e as &dyn Health))
                    .collect(),
            ),
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
            (GROUPS[5], engine.tiers.health_members().collect()),
            (
                GROUPS[6],
                hal.map(|h| h as &dyn Health).into_iter().collect(),
            ),
            (
                GROUPS[7],
                wasm.map(|w| w as &dyn Health).into_iter().collect(),
            ),
            (
                GROUPS[8],
                learn.map(|l| l as &dyn Health).into_iter().collect(),
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

/// The standing store note (spec 115 R3c). Matches what `pii store compact` does.
pub const STORE_NEVER_SHRINKS: &str = "the main store never shrinks on its own: runs and journal events are never deleted and redb returns only free space at the end of the file; watch pii_store_bytes_per_run, and reclaim interior free pages offline with `pii store compact` (daemon stopped)";

/// Run-lifecycle latency and the standing store note (spec 115 R3c/R4).
fn attach_latency(engine: &Engine, out: &mut StatusResult) {
    out.latency = engine
        .latency
        .snapshot()
        .into_iter()
        .map(|(outcome, p)| pii_proto::sys::LatencyStatus {
            outcome: outcome.to_owned(),
            count: p.count,
            p50_ms: p.p50_ms,
            p95_ms: p.p95_ms,
            p99_ms: p.p99_ms,
            max_ms: p.max_ms,
        })
        .collect();
    out.notes.push(STORE_NEVER_SHRINKS.to_owned());
}

/// Notes `(runs, rss_kb)` on the engine's trend and returns the recent RSS
/// growth in bytes per run (spec 116 R2; 0 until a window of runs exists).
fn rss_gauge(engine: &Engine, runs: u64, rss_kb: u64) -> u64 {
    engine.rss_trend.note(runs, rss_kb);
    engine.rss_trend.bytes_per_run(runs, rss_kb)
}

/// Sums the message counters of every channel that keeps them (spec 117).
fn add_channel_counters(counts: &mut Counts, channels: Option<&BTreeMap<String, ChannelRuntime>>) {
    for c in channels.into_iter().flat_map(BTreeMap::values) {
        if let Some(n) = c.channel.counters() {
            counts.mqtt_received += n.received;
            counts.mqtt_dropped += n.dropped;
            counts.mqtt_published += n.published;
        }
    }
}

/// The HAL's operation counts by kind and its refusals (spec 121). Absent
/// (HAL off) leaves them empty, so `pii metrics` prints no HAL lines.
fn add_hal_counters(counts: &mut Counts, hal: Option<&pii_hal::Hal>) {
    let Some(hal) = hal else {
        return;
    };
    let c = hal.counts();
    counts.hal_ops = [
        ("claim", c.claim),
        ("release", c.release),
        ("read", c.read),
        ("write", c.write),
        ("transaction", c.transaction),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    counts.hal_refusals = c.refusals;
}

/// The HAL's and the plugin host's counters (specs 121, 122).
fn add_device_counters(
    counts: &mut Counts,
    hal: Option<&pii_hal::Hal>,
    wasm: Option<&pii_wasm::WasmHost>,
) {
    add_hal_counters(counts, hal);
    add_wasm_counters(counts, wasm);
}

/// The plugin host's call counts by outcome and its refusals (spec 122).
/// Absent (host off) leaves them empty, so `pii metrics` prints no WASM lines.
fn add_wasm_counters(counts: &mut Counts, wasm: Option<&pii_wasm::WasmHost>) {
    let Some(wasm) = wasm else {
        return;
    };
    let c = wasm.counts();
    counts.wasm_calls = [
        ("ok", c.ok),
        ("trap", c.trap),
        ("timeout", c.timeout),
        ("fuel", c.fuel),
        ("memory", c.memory),
        ("invalid", c.invalid),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    counts.wasm_refusals = c.refusals;
}

/// The last extraction scan's candidates by kind and the refusals (spec 123).
/// Absent (no scan yet) leaves them empty, so `pii metrics` prints no learn lines.
fn add_learn_counters(counts: &mut Counts, learn: Option<&pii_learn::LearnHealth>) {
    let Some(s) = learn.and_then(pii_learn::LearnHealth::status) else {
        return;
    };
    counts.learn_candidates = s.candidates_by_kind;
    counts.learn_refusals = s.refused_total;
}

/// The last verification pass's verdict counts (spec 124); empty before any pass.
fn add_verdict_counters(counts: &mut Counts, learn: Option<&pii_learn::LearnHealth>) {
    if let Some(v) = learn.and_then(pii_learn::LearnHealth::verification) {
        counts.learn_verdicts = v.verdicts;
    }
}

/// The unattended loop's pass and proposal counters (spec 139); 0 until a pass has run.
fn add_unattended_counters(counts: &mut Counts, learn: Option<&pii_learn::LearnHealth>) {
    if let Some(u) = learn.and_then(pii_learn::LearnHealth::unattended) {
        counts.learn_unattended_runs = u.runs;
        counts.learn_candidates_proposed = u.candidates_proposed_total;
    }
}

/// Promotions by state and refusals (spec 125); left empty until any exist, so
/// a daemon that never promoted prints no promotion lines.
fn add_promotion_counters(counts: &mut Counts, learn: Option<&pii_learn::LearnHealth>) {
    if let Some(l) = learn {
        let p = l.promotions();
        if p.values().any(|n| *n > 0) {
            counts.learn_promotions = p;
        }
    }
}

/// The entity store's sizes and counters (spec 126); left empty until the graph has been used.
fn add_entity_counters(counts: &mut Counts, entities: Option<&pii_memory::entity::EntityHealth>) {
    let Some(c) = entities.and_then(pii_memory::entity::EntityHealth::counts) else {
        return;
    };
    counts.memory_entities = c.entities;
    counts.memory_relationships = c.relationships;
    counts.memory_writes = [
        ("entity".to_owned(), c.entities_written),
        ("relationship".to_owned(), c.relationships_written),
    ]
    .into_iter()
    .collect();
    counts.memory_write_refusals = c.write_refusals;
    counts.memory_query_refusals = c.query_refusals;
    counts.memory_tiers = c.tiers;
    counts.memory_tier_changes = c.tier_changes;
    counts.memory_archive_ops = c.archive_ops;
}

/// Worker requests refused or cancelled around a drain/reload, summed (spec 120).
fn refused_total(workers: Option<&BTreeMap<String, pii_worker::Supervisor>>) -> u64 {
    workers
        .into_iter()
        .flat_map(BTreeMap::values)
        .map(pii_worker::Supervisor::refused_count)
        .sum()
}

/// The git revision this binary was built from.
pub const REVISION: &str = env!("PII_GIT_REV");

fn build_identity() -> BuildIdentity {
    BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        revision: REVISION.to_owned(),
        target: env!("PII_TARGET").to_owned(),
    }
}

/// Fills the identity fields of `out` from `engine` (and `ctx` when a daemon
/// is attached). The pre-existing fields are left untouched.
// reason: one block per status section; each added subsystem is a few lines here.
#[allow(clippy::too_many_lines)]
pub fn attach(engine: &Engine, ctx: Option<&DaemonContext>, out: &mut StatusResult) {
    attach_latency(engine, out);
    let info = engine.store.info().ok();
    let mut counts = Counts {
        memory_docs: out.memory.docs,
        approvals_pending: engine
            .approvals
            .pending_approvals()
            .map_or(0, |a| a.len() as u64),
        ..Counts::default()
    };
    // Spec 217: agent-call totals come from the journal, so they survive a restart.
    if let Ok(totals) =
        pii_graph::agent_calls::store_totals(&engine.store, engine.store.process_start_ms())
    {
        counts.dispatches_total = totals.dispatches_total;
        counts.refusals_total = totals.refusals_total;
        counts.criteria_failures_total = totals.criteria_failures_total;
        counts.orphans_total = totals.orphans_total;
        counts.orphans_since_start = totals.orphans_since_start;
        counts.agents_in_flight = totals.agents_in_flight;
    }
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
        out.stores.push(aux(
            "queue",
            "queue.redb",
            engine.queue.format_version().ok(),
        ));
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
    out.build = build_identity();
    let workers = ctx.map(|c| c.workers.lock());
    let channels = ctx.map(|c| c.channels.lock());
    let hal = engine.hal.get();
    let wasm = engine.wasm.get();
    // Spec 123 (+ orchestrator fix, 2026-10-10): read-only, and from the directory the CLI actually writes.
    // The guide and `pii learn --help` both say `--dir` defaults to the DAEMON'S RUNTIME DIR, which is where
    // `pii learn scan` leaves `learn/status.json`. The first implementation read `store.path().parent()`, a
    // different directory in any normal layout — so a guide-conformant scan was invisible to the daemon and
    // `learn_candidates`/`learn_refusals` could only ever read zero (measured: scan into the runtime dir, daemon
    // answers "no scan yet"). The daemon's own runtime dir is the same value the CLI default resolves to.
    let learn = ctx.map(|c| pii_learn::LearnHealth::new(c.runtime_dir.clone()));
    let entities = ctx.map(|c| {
        pii_memory::entity::EntityHealth::new(
            c.runtime_dir.clone(),
            engine
                .entities
                .get()
                .map_or_else(pii_memory::entity::GraphConfig::default, |s| *s.config()),
        )
    });
    let registry = Registry::build(
        engine,
        ctx.map(|c| &*c.net),
        workers.as_deref(),
        channels.as_deref(),
        hal.as_deref(),
        wasm.as_deref(),
        learn.as_ref(),
        entities.as_ref(),
    );
    add_entity_counters(&mut counts, entities.as_ref());
    add_learn_counters(&mut counts, learn.as_ref());
    add_verdict_counters(&mut counts, learn.as_ref());
    add_unattended_counters(&mut counts, learn.as_ref());
    add_promotion_counters(&mut counts, learn.as_ref());
    add_device_counters(&mut counts, hal.as_deref(), wasm.as_deref());
    let posts = engine.channel_hub.counts();
    counts.channel_posts = posts.posted;
    counts.channel_post_refusals = posts.refused;
    counts.quarantine_refusals = engine.refusals.refused();
    counts.quarantine_refusals_recorded = engine.refusals.recorded();
    if let Some(ctx) = ctx {
        counts.cron_entries = ctx.cron.lock().len() as u64;
        add_channel_counters(&mut counts, channels.as_deref());
        counts.worker_requests_refused = refused_total(workers.as_deref());
        out.mounts.extend(daemon_mounts(ctx));
        out.config = ConfigIdentity {
            path: ctx.config_path.display().to_string(),
            sha256: std::fs::read(&ctx.config_path)
                .ok()
                .map(|b| hex(&Sha256::digest(b))),
        };
    }
    let reports = registry.reports();
    watch_health(engine, &reports);
    drop(registry);
    drop((workers, channels));
    out.rss_bytes_per_run = rss_gauge(engine, counts.runs, out.rss_kb);
    out.counts = counts;
    out.since_deploy =
        super::since_deploy::since_deploy(&engine.store, REVISION).unwrap_or_default();
    out.subsystems = reports.into_iter().map(wire).collect();
}

/// Tells the engine's log about every subsystem whose state changed since the
/// previous poll (`health.state.changed`, spec 109 R2).
fn watch_health(engine: &Engine, reports: &[SubsystemHealth]) {
    for h in reports {
        engine.health_watch.note(h, &*engine.log);
    }
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
    let policy = pii_tools::sandbox::grant_policy(engine.cfg.sandbox);
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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::EngineConfig;
    use pii_tools::sandbox::SandboxProfileName;

    /// Spec 107 R1/R2: the workspace mount quotes the profile's real grant —
    /// reads included — and a different profile gives a different text.
    #[test]
    fn workspace_mount_states_the_profile_in_force() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mount-").unwrap();
        let mk = |sandbox| {
            Engine::new(EngineConfig {
                grant_ceiling: None,
                workspace: dir.path().to_path_buf(),
                store_path: dir.path().join(format!("{sandbox:?}.redb")),
                model: format!(
                    "scripted:{}/../../fixtures/phase2/hello.json",
                    env!("CARGO_MANIFEST_DIR")
                ),
                sandbox,
                route_policy: BTreeMap::new(),
                tiers: Vec::new(),
                session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
                llama_host: None,
                graph_dir: dir.path().join("graphs"),
            })
            .unwrap()
        };
        let ws = workspace_mount(&mk(SandboxProfileName::Workspace));
        let off = workspace_mount(&mk(SandboxProfileName::Off));
        assert_eq!(
            ws.policy,
            pii_tools::sandbox::grant_policy(SandboxProfileName::Workspace)
        );
        assert!(
            ws.policy.contains("READS are NOT confined"),
            "{}",
            ws.policy
        );
        assert!(
            off.policy.contains("NO filesystem confinement"),
            "{}",
            off.policy
        );
    }

    /// Spec 116 R2: a leak is visible in the product. Polls that show RSS
    /// growing with runs read as bytes per run; flat RSS reads 0. If
    /// `rss_gauge` stopped feeding the trend, the first assertion fails.
    #[test]
    fn rss_gauge_reports_growth_per_run_and_zero_when_flat() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rssg-").unwrap();
        let mk = |name: &str| {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
            Engine::new(EngineConfig {
                grant_ceiling: None,
                workspace: dir.path().to_path_buf(),
                store_path: dir.path().join(name).join("store.redb"),
                model: format!(
                    "scripted:{}/../../fixtures/phase2/hello.json",
                    env!("CARGO_MANIFEST_DIR")
                ),
                sandbox: SandboxProfileName::Off,
                route_policy: BTreeMap::new(),
                tiers: Vec::new(),
                session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
                llama_host: None,
                graph_dir: dir.path().join("graphs"),
            })
            .unwrap()
        };
        let leaking = mk("a");
        assert_eq!(rss_gauge(&leaking, 0, 50_000), 0, "first poll: no window");
        // 150 B/run: 10 000 runs -> 1 464 KiB.
        let got = rss_gauge(&leaking, 10_000, 50_000 + 1_464);
        assert!((140..=160).contains(&got), "{got}");
        let flat = mk("b");
        rss_gauge(&flat, 0, 50_000);
        assert_eq!(rss_gauge(&flat, 10_000, 50_000), 0);
    }

    /// The aggregator's registered set, pinned by NAME (spec 100 R2): dropping
    /// or renaming a registration changes this list and fails the test.
    #[test]
    fn registry_is_pinned_by_name() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "registry-").unwrap();
        let engine = Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!(
                "scripted:{}/../../fixtures/phase2/hello.json",
                env!("CARGO_MANIFEST_DIR")
            ),
            sandbox: SandboxProfileName::Off,
            route_policy: BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            graph_dir: dir.path().join("graphs"),
        })
        .unwrap();
        let registry = Registry::build(&engine, None, None, None, None, None, None, None);
        assert_eq!(
            registry.group_names(),
            [
                "store",
                "memory",
                "net",
                "worker",
                "channels",
                "model_tiers",
                "hal",
                "wasm",
                "learn"
            ]
        );
        let names: Vec<_> = registry.reports().into_iter().map(|h| h.name).collect();
        assert_eq!(
            names,
            ["store", "memory"],
            "bare engine reports store+memory"
        );
    }

    /// Spec 109 R2: a polled state change reaches the engine's log, once.
    #[test]
    fn a_polled_state_change_reaches_the_engine_log() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "hwatch-").unwrap();
        let engine = Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!(
                "scripted:{}/../../fixtures/phase2/hello.json",
                env!("CARGO_MANIFEST_DIR")
            ),
            sandbox: SandboxProfileName::Off,
            route_policy: BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            graph_dir: dir.path().join("graphs"),
        })
        .unwrap();
        let rec = std::sync::Arc::new(pii_observe::Recorder::new());
        engine.attach_log(rec.clone());
        let mut h = SubsystemHealth::ok("store");
        watch_health(&engine, std::slice::from_ref(&h));
        h.state = pii_observe::State::Down;
        watch_health(&engine, std::slice::from_ref(&h));
        let ev = rec.events_named(pii_observe::names::HEALTH_STATE_CHANGED);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fields.get("to"), Some(&serde_json::json!("down")));
    }
}
