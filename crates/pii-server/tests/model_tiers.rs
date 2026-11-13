//! Spec-114 model-tier tests: the `[[model_tier]]` table, the one precedence
//! function, loud refusal (unknown / unavailable tier, no fallback), the
//! capability-layer tool gate, and the observable record.

#![allow(
    clippy::print_stderr, // reason: the exit criteria paste these traces from --nocapture
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use parking_lot::Mutex;

use pii_abi::{NodeInput, Outcome, PiiError};
use pii_core::{Budget, CapKind, CapSet, Ctx, ObsEvent, PathScope, RecordingObserver, TraceCtx};
use pii_graph::node::NodeFactory as _;
use pii_model::{ChatRequest, ChatResponse, ModelProvider};
use pii_server::daemon::config::{Config, ModelTierSection};
use pii_server::node_kinds::agent::AgentFactory;
use pii_server::tiers::{Level, TierTable, precedence};

fn scratch() -> std::path::PathBuf {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/tiers"));
    std::fs::create_dir_all(dir).unwrap();
    dunce::canonicalize(dir).unwrap()
}

/// Writes a one-turn scripted fixture replying `text`; returns its spec.
fn fixture(name: &str, text: &str) -> String {
    let p = scratch().join(format!("{name}.json"));
    std::fs::write(
        &p,
        serde_json::json!({"turns": [{"text": text, "tool_calls": []}]}).to_string(),
    )
    .unwrap();
    format!("scripted:{}", p.display())
}

fn section(name: &str, model: &str, tools: Option<&[&str]>) -> ModelTierSection {
    ModelTierSection {
        name: name.into(),
        model: model.into(),
        tools: tools.map(|t| t.iter().map(|s| (*s).to_owned()).collect()),
        max_tokens: None,
    }
}

fn table(sections: &[ModelTierSection]) -> (Arc<TierTable>, Arc<Mutex<Vec<String>>>) {
    let unavailable = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&unavailable);
    let t = TierTable::build(
        sections,
        pii_model::provider_from_spec,
        move |tier, model, _why| sink.lock().push(format!("{tier}={model}")),
    );
    (Arc::new(t), unavailable)
}

/// The default (run-level) provider: replies "DEFAULT" and counts calls.
struct Default_(Arc<std::sync::atomic::AtomicUsize>);

impl ModelProvider for Default_ {
    fn name(&self) -> &'static str {
        "default"
    }
    fn chat(&self, _c: &Ctx, _r: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ChatResponse {
            text: String::from("DEFAULT"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

fn factory(tiers: Option<Arc<TierTable>>) -> (AgentFactory, Arc<std::sync::atomic::AtomicUsize>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let factory = AgentFactory {
        provider: Arc::new(Default_(Arc::clone(&calls))),
        workspace: scratch(),
        live: None,
        memory: None,
        route_policy: std::collections::BTreeMap::new(),
        tiers,
        models: Arc::default(),
    };
    (factory, calls)
}

fn ctx(obs: Arc<RecordingObserver>) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::FS_READ | CapKind::PROC | CapKind::NET;
    caps.fs_read = PathScope::new(vec![scratch()]);
    Ctx::with_observer(
        caps,
        Budget::unlimited(),
        obs,
        TraceCtx {
            run: pii_abi::RunId::from("r"),
            span: 0,
        },
    )
}

fn input() -> NodeInput {
    NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({"prompt": "hi"}),
        nodes: serde_json::json!({}),
    }
}

fn text_of(o: &Outcome) -> String {
    match o {
        Outcome::Next { value, .. } => value.to_string(),
        other => panic!("unexpected outcome {other:?}"),
    }
}

// ---- R1: the table parses --------------------------------------------------

#[test]
fn tier_table_parses_and_rejects_typos_and_duplicates() {
    let cfg = Config::parse(
        r#"
[[model_tier]]
name = "fast"
model = "scripted:/f.json"
tools = ["fs_read", "grep"]
max_tokens = 512

[[model_tier]]
name = "big"
model = "llamacpp-server:big-70b"
"#,
    )
    .unwrap();
    assert_eq!(cfg.model_tier.len(), 2);
    assert_eq!(cfg.model_tier[0].tools.as_deref().unwrap().len(), 2);
    assert_eq!(cfg.model_tier[0].max_tokens, Some(512));
    assert_eq!(cfg.model_tier[1].tools, None);
    let typo = Config::parse("[[model_tier]]\nname=\"a\"\nmodel=\"m\"\ntoolz=[]\n");
    assert!(typo.is_err(), "a typo'd tier key must be a load error");
    let dup = Config::parse(
        "[[model_tier]]\nname=\"a\"\nmodel=\"m\"\n[[model_tier]]\nname=\"a\"\nmodel=\"n\"\n",
    )
    .unwrap_err();
    assert!(dup.to_string().contains("duplicate"), "{dup}");
}

// ---- R6: a node with a tier runs on that tier's fixture; the record names it

#[test]
fn node_with_tier_runs_on_that_tiers_fixture_and_records_it() {
    let fast = fixture("fast", "FAST-REPLY");
    let big = fixture("big", "BIG-REPLY");
    let (t, _) = table(&[section("fast", &fast, None), section("big", &big, None)]);
    let (f, default_calls) = factory(Some(t));
    for (tier, spec, want) in [("fast", &fast, "FAST-REPLY"), ("big", &big, "BIG-REPLY")] {
        let node = f
            .build(&serde_json::json!({"system": "s", "tier": tier}))
            .unwrap();
        let obs = Arc::new(RecordingObserver::new());
        let out = node.run(&ctx(obs.clone()), &input()).unwrap();
        assert!(text_of(&out).contains(want), "{tier}: {out:?}");
        let rec: Vec<_> = obs
            .events()
            .into_iter()
            .filter(|e| matches!(e, ObsEvent::TierResolved { .. }))
            .collect();
        assert_eq!(
            rec,
            [ObsEvent::TierResolved {
                tier: tier.into(),
                model: spec.clone()
            }],
            "the run records the resolved tier + model"
        );
        eprintln!("TRACE tier={tier} model={spec} reply={}", text_of(&out));
    }
    assert_eq!(
        default_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the default provider must not serve a tiered node"
    );
}

// ---- R2: the precedence, one case per level (and the wrong-level control) --

#[test]
fn precedence_each_level_wins_over_the_ones_below() {
    let s = |x: &str| Some(x.to_owned());
    let all = precedence(s("node"), s("tier"), s("run"), s("env"), s("cfg")).unwrap();
    assert_eq!(all, (Level::NodeModel, "node".into()));
    let no_node = precedence(None, s("tier"), s("run"), s("env"), s("cfg")).unwrap();
    assert_eq!(no_node, (Level::NodeTier, "tier".into()));
    let no_tier = precedence(None, None, s("run"), s("env"), s("cfg")).unwrap();
    assert_eq!(no_tier, (Level::RunModel, "run".into()));
    let env = precedence(None, None, None, s("env"), s("cfg")).unwrap();
    assert_eq!(env, (Level::Env, "env".into()));
    let cfg = precedence(None, None, None, None, s("cfg")).unwrap();
    assert_eq!(cfg, (Level::Config, "cfg".into()));
    assert!(precedence(None, None, None, None, None).is_none());
    // Control: a lower level must NEVER win while a higher one is present.
    assert_ne!(all.0, Level::NodeTier);
    assert_ne!(no_node.0, Level::RunModel);
}

#[test]
fn node_explicit_model_beats_its_tier_and_tier_beats_the_default_provider() {
    let fast = fixture("fast2", "FAST-REPLY");
    let (t, _) = table(&[section("fast", &fast, None)]);
    let (f, default_calls) = factory(Some(t));
    // explicit model + tier: the explicit model wins; the default provider serves.
    let node = f
        .build(&serde_json::json!({"system": "s", "tier": "fast", "model": "explicit-m"}))
        .unwrap();
    let out = node
        .run(&ctx(Arc::new(RecordingObserver::new())), &input())
        .unwrap();
    assert!(text_of(&out).contains("DEFAULT"), "{out:?}");
    assert_eq!(default_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // tier only: the tier's fixture serves, not the default.
    let node = f
        .build(&serde_json::json!({"system": "s", "tier": "fast"}))
        .unwrap();
    let out = node
        .run(&ctx(Arc::new(RecordingObserver::new())), &input())
        .unwrap();
    assert!(text_of(&out).contains("FAST-REPLY"), "{out:?}");
    // neither: the run's provider.
    let node = f.build(&serde_json::json!({"system": "s"})).unwrap();
    let out = node
        .run(&ctx(Arc::new(RecordingObserver::new())), &input())
        .unwrap();
    assert!(text_of(&out).contains("DEFAULT"), "{out:?}");
}

#[test]
fn config_resolve_model_goes_through_the_one_precedence_function() {
    let cfg = Config::parse("[core]\nmodel = \"from-config\"\n").unwrap();
    assert_eq!(
        cfg.resolve_model(Some("run".into())).as_deref(),
        Some("run")
    );
    // The ONE place: the chain is written only in tiers.rs.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for entry in walk(&src) {
        let text = std::fs::read_to_string(&entry).unwrap();
        if text.contains("fn precedence(") {
            hits.push(entry.file_name().unwrap().to_string_lossy().into_owned());
        }
        assert!(
            !text.contains(".or_else(|| pii_core::env_var(\"PII_MODEL\"))"),
            "{} re-implements the model chain",
            entry.display()
        );
    }
    assert_eq!(
        hits,
        ["tiers.rs"],
        "precedence is defined in exactly one file"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}

// ---- R3: loud refusal, no silent downgrade ---------------------------------

#[test]
fn unknown_tier_refuses_naming_it_and_the_available_ones() {
    let (t, _) = table(&[
        section("fast", &fixture("fast3", "x"), None),
        section("big", &fixture("big3", "y"), None),
    ]);
    let (f, _) = factory(Some(t));
    let err = f
        .build(&serde_json::json!({"system": "s", "tier": "ghost"}))
        .err()
        .expect("unknown tier must refuse");
    assert!(matches!(err, PiiError::NotFound(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("ghost") && msg.contains("big, fast"), "{msg}");
    eprintln!("UNKNOWN-TIER: {msg}");
}

#[test]
fn unavailable_provider_does_not_fall_back_and_other_tier_still_works() {
    let dead_spec = format!("scripted:{}/nope-missing.json", scratch().display());
    let ok_spec = fixture("fast4", "FAST-REPLY");
    let (t, unavailable) = table(&[
        section("dead", &dead_spec, None),
        section("fast", &ok_spec, None),
    ]);
    assert_eq!(*unavailable.lock(), [format!("dead={dead_spec}")]);
    let (f, default_calls) = factory(Some(Arc::clone(&t)));
    let err = f
        .build(&serde_json::json!({"system": "s", "tier": "dead"}))
        .err()
        .expect("an unavailable tier must refuse, not fall back");
    assert!(matches!(err, PiiError::Unavailable(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("'dead'") && msg.contains(&dead_spec), "{msg}");
    eprintln!("UNAVAILABLE: {msg}");
    // Even with an explicit model on the node, the named dead tier still refuses.
    assert!(
        f.build(&serde_json::json!({"system": "s", "tier": "dead", "model": "m"}))
            .is_err()
    );
    assert_eq!(default_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Negative control: the other tier still serves.
    let node = f
        .build(&serde_json::json!({"system": "s", "tier": "fast"}))
        .unwrap();
    let out = node
        .run(&ctx(Arc::new(RecordingObserver::new())), &input())
        .unwrap();
    assert!(text_of(&out).contains("FAST-REPLY"), "{out:?}");
    // Health shows the degraded tier and the healthy one.
    let reports: Vec<_> = t
        .health_members()
        .map(pii_observe::Health::health)
        .collect();
    let dead = reports
        .iter()
        .find(|h| h.name == "model_tier:dead")
        .unwrap();
    assert_eq!(dead.state, pii_observe::State::Degraded);
    assert!(dead.detail.as_deref().unwrap().contains(&dead_spec));
    let fast = reports
        .iter()
        .find(|h| h.name == "model_tier:fast")
        .unwrap();
    assert_eq!(fast.state, pii_observe::State::Ok);
}

// ---- R4: the tool gate lives in the capability layer -----------------------

/// Turn 1: call `tool` with `args`; turn 2: reply. Captures what the model saw.
struct ToolCaller {
    tool: &'static str,
    args: serde_json::Value,
    n: std::sync::atomic::AtomicUsize,
    advertised: Mutex<Vec<String>>,
    tool_result: Mutex<String>,
}

impl ModelProvider for ToolCaller {
    fn name(&self) -> &'static str {
        "tool-caller"
    }
    fn chat(&self, _c: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            *self.advertised.lock() = req.tools.iter().map(|t| t.name.clone()).collect();
            return Ok(ChatResponse {
                text: String::new(),
                tool_calls: vec![pii_model::ToolCall {
                    id: String::from("c1"),
                    name: self.tool.to_owned(),
                    arguments: self.args.clone(),
                }],
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            });
        }
        let last = req
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        *self.tool_result.lock() = last;
        Ok(ChatResponse {
            text: String::from("done"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

struct Shared(Arc<ToolCaller>);
impl ModelProvider for Shared {
    fn name(&self) -> &'static str {
        "tool-caller"
    }
    fn chat(&self, c: &Ctx, r: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.0.chat(c, r)
    }
}

/// Runs one agent node on a tier whose tools are `["fs_read"]`, whose provider
/// calls `tool` once. Returns (provider capture, observer).
fn tier_run(
    tool: &'static str,
    args: serde_json::Value,
) -> (Arc<ToolCaller>, Arc<RecordingObserver>) {
    let caller = Arc::new(ToolCaller {
        tool,
        args,
        n: std::sync::atomic::AtomicUsize::new(0),
        advertised: Mutex::new(Vec::new()),
        tool_result: Mutex::new(String::new()),
    });
    let c2 = Arc::clone(&caller);
    let t = TierTable::build(
        &[section("ro", "stub:ro", Some(&["fs_read"]))],
        move |_| Ok(Arc::new(Shared(Arc::clone(&c2))) as Arc<dyn ModelProvider>),
        |_, _, _| {},
    );
    let (f, _) = factory(Some(Arc::new(t)));
    let node = f
        .build(&serde_json::json!({"system": "s", "tier": "ro"}))
        .unwrap();
    let obs = Arc::new(RecordingObserver::new());
    let out = node.run(&ctx(obs.clone()), &input()).unwrap();
    assert!(matches!(out, Outcome::Next { .. }), "{out:?}");
    (caller, obs)
}

fn denials(obs: &RecordingObserver) -> Vec<String> {
    obs.events()
        .into_iter()
        .filter_map(|e| match e {
            ObsEvent::Denial { cap, .. } => Some(cap),
            _ => None,
        })
        .collect()
}

#[test]
fn tool_outside_the_tier_is_denied_by_the_capability_layer_with_an_event() {
    // `bash` is a real built-in with PROC granted on this ctx: only the tier
    // gate stands between the model's request and the command.
    let marker = scratch().join("bash-ran.marker");
    let _ = std::fs::remove_file(&marker);
    let (caller, obs) = tier_run(
        "bash",
        serde_json::json!({"command": format!("touch {}", marker.display())}),
    );
    let d = denials(&obs);
    assert_eq!(d.first().map(String::as_str), Some("tool:bash"), "{d:?}");
    assert!(d.iter().all(|c| c.contains("tool:bash")), "{d:?}");
    let seen = caller.tool_result.lock().clone();
    assert!(
        seen.contains("tool:bash"),
        "the model sees the denial: {seen}"
    );
    assert!(!marker.exists(), "the denied tool must not have run");
    let adv = caller.advertised.lock().clone();
    assert_eq!(adv, ["fs_read"], "only the tier's tools are advertised");
    eprintln!("DENIED: events={d:?} model-saw={seen}");
}

#[test]
fn tool_inside_the_tier_is_allowed() {
    let file = scratch().join("readable.txt");
    std::fs::write(&file, "tier-readable").unwrap();
    let (caller, obs) = tier_run(
        "fs_read",
        serde_json::json!({"path": file.display().to_string()}),
    );
    assert!(denials(&obs).is_empty(), "no denial for an allowed tool");
    let seen = caller.tool_result.lock().clone();
    assert!(seen.contains("tier-readable"), "the tool ran: {seen}");
    eprintln!("ALLOWED: model-saw={seen}");
}

// ---- R5: the new names are registered --------------------------------------

#[test]
fn tier_event_names_are_registered() {
    for n in [
        pii_observe::names::MODEL_TIER_RESOLVED,
        pii_observe::names::MODEL_TIER_UNAVAILABLE,
    ] {
        assert!(pii_observe::names::ALL.contains(&n), "{n} missing from ALL");
    }
}
