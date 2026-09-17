//! Wire round-trip tests (spec 16): every `Method` variant, `run.*` params,
//! `session.*` params, and `sys.*` results survive
//! `serde_json::from_str(serde_json::to_string(&v)?)` AND the serialized
//! string matches the documented wire shape.
//!
//! Unwrapping is fine here: a failing round-trip is exactly what a panic in
//! a test should report.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use pii_proto::method::Method;
use pii_proto::run::{RunListParams, RunStartParams, WireRunHeader, WireRunStatus};
use pii_proto::session::{ContentBlock, SessionCreateParams};
use pii_proto::sys::{
    ChannelStatus, HelloResult, Limits, ListenerStatus, MemoryStatus, PeerInfo, StatusResult,
    WorkerStatus,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------- method --

/// All (variant, wire string) pairs, mirroring the `#[serde(rename)]`s in
/// `method.rs` and the `Display`/`FromStr` tables.
const METHODS: &[(Method, &str)] = &[
    (Method::SysHello, "sys.hello"),
    (Method::SysStatus, "sys.status"),
    (Method::SysVersion, "sys.version"),
    (Method::SysShutdown, "sys.shutdown"),
    (Method::SessionCreate, "session.create"),
    (Method::SessionPrompt, "session.prompt"),
    (Method::SessionInterrupt, "session.interrupt"),
    (Method::SessionState, "session.state"),
    (Method::SessionList, "session.list"),
    (Method::SessionClose, "session.close"),
    (Method::RunStart, "run.start"),
    (Method::RunGet, "run.get"),
    (Method::RunList, "run.list"),
    (Method::RunCancel, "run.cancel"),
    (Method::RunResume, "run.resume"),
    (Method::RunTrace, "run.trace"),
    (Method::GraphApply, "graph.apply"),
    (Method::GraphGet, "graph.get"),
    (Method::GraphList, "graph.list"),
    (Method::GraphLint, "graph.lint"),
    (Method::EventSubscribe, "event.subscribe"),
    (Method::EventUnsubscribe, "event.unsubscribe"),
    (Method::WorkerList, "worker.list"),
    (Method::WorkerReload, "worker.reload"),
    (Method::ApproveList, "approve.list"),
    (Method::ApproveResolve, "approve.resolve"),
    (Method::HealList, "heal.list"),
    (Method::HealGet, "heal.get"),
    (Method::CronList, "cron.list"),
    (Method::ChannelList, "channel.list"),
    (Method::MemorySearch, "memory.search"),
    (Method::MemoryPut, "memory.put"),
    (Method::MemoryConsolidate, "memory.consolidate"),
    (Method::SysReload, "sys.reload"),
];

#[test]
fn method_roundtrips_and_matches_wire_string() {
    for (method, wire) in METHODS {
        let s = serde_json::to_string(method).expect("serialize method");
        assert_eq!(s, format!("\"{wire}\""), "wire string for {wire:?}");
        let back: Method = serde_json::from_str(&s).expect("deserialize method");
        assert_eq!(&back, method, "round-trip for {wire:?}");
        // Cross-check against the hand-written Display/FromStr tables too.
        assert_eq!(method.to_string(), *wire);
        assert_eq!(wire.parse::<Method>().ok().as_ref(), Some(method));
    }
}

#[test]
fn method_rejects_unknown_wire_string() {
    assert!("nope.nope".parse::<Method>().is_err());
    assert!(serde_json::from_str::<Method>("\"nope.nope\"").is_err());
}

// -------------------------------------------------------------------- run --

#[test]
fn run_start_params_roundtrips() {
    let params = RunStartParams {
        graph: "verify-fix".to_string(),
        input: json!({"test_command": "cargo test", "nested": {"k": [1, 2, 3]}}),
        caps: Some(vec!["net".to_string(), "fs".to_string()]),
    };
    let s = serde_json::to_string(&params).expect("serialize");
    let back: RunStartParams = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back, params);

    // Wire shape: flat object with the three fields.
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["graph"], "verify-fix");
    assert_eq!(v["caps"], json!(["net", "fs"]));
    assert_eq!(v["input"]["test_command"], "cargo test");
}

#[test]
fn run_start_params_caps_none_roundtrips() {
    let params = RunStartParams {
        graph: "g".to_string(),
        input: Value::Null,
        caps: None,
    };
    let s = serde_json::to_string(&params).unwrap();
    let back: RunStartParams = serde_json::from_str(&s).unwrap();
    assert_eq!(back, params);
    // `None` serializes as an explicit JSON null (no skip attribute).
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v.get("caps"), Some(&Value::Null));
}

#[test]
fn run_list_params_roundtrips_with_none() {
    let params = RunListParams {
        status: None,
        since: None,
    };
    let s = serde_json::to_string(&params).unwrap();
    let back: RunListParams = serde_json::from_str(&s).unwrap();
    assert_eq!(back, params);

    // Actual JSON shape: both Option fields are present as explicit nulls
    // (no `skip_serializing_if` on these), so a wire peer sees:
    // {"status":null,"since":null}
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(
        v["status"],
        Value::Null,
        "status None must be null, not omitted"
    );
    assert_eq!(
        v["since"],
        Value::Null,
        "since None must be null, not omitted"
    );
    assert!(v.as_object().unwrap().contains_key("status"));
    assert!(v.as_object().unwrap().contains_key("since"));

    // Filled variant also round-trips.
    let full = RunListParams {
        status: Some(WireRunStatus::Suspended),
        since: Some(42),
    };
    let fs = serde_json::to_string(&full).unwrap();
    let fback: RunListParams = serde_json::from_str(&fs).unwrap();
    assert_eq!(fback, full);
    let fv: Value = serde_json::from_str(&fs).unwrap();
    assert_eq!(fv["status"], "suspended"); // snake_case enum on the wire
    assert_eq!(fv["since"], 42);
}

#[test]
fn wire_run_header_roundtrips() {
    let header = WireRunHeader {
        run_id: "run-7".to_string(),
        graph: "verify-fix".to_string(),
        status: WireRunStatus::Running,
        seq: 123,
        dur_ms: 4_500,
    };
    let s = serde_json::to_string(&header).unwrap();
    let back: WireRunHeader = serde_json::from_str(&s).unwrap();
    assert_eq!(back, header);

    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["run_id"], "run-7");
    assert_eq!(v["status"], "running");

    // Every status variant round-trips in its snake_case wire form.
    for (status, wire) in [
        (WireRunStatus::Running, "running"),
        (WireRunStatus::Suspended, "suspended"),
        (WireRunStatus::Succeeded, "succeeded"),
        (WireRunStatus::Failed, "failed"),
    ] {
        let h = WireRunHeader {
            status,
            ..header.clone()
        };
        let hs = serde_json::to_string(&h).unwrap();
        let hv: Value = serde_json::from_str(&hs).unwrap();
        assert_eq!(hv["status"], wire);
        let hback: WireRunHeader = serde_json::from_str(&hs).unwrap();
        assert_eq!(hback, h);
    }
}

// ---------------------------------------------------------------- session --

#[test]
fn session_create_params_roundtrips() {
    let params = SessionCreateParams {
        cwd: "/projects/pi-improved".to_string(),
        model: Some("ollama-cloud/glm-5.3-flash:cloud".to_string()),
        tools: vec!["read".to_string(), "bash".to_string()],
        graph: None,
    };
    let s = serde_json::to_string(&params).unwrap();
    let back: SessionCreateParams = serde_json::from_str(&s).unwrap();
    assert_eq!(back, params);

    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["model"], "ollama-cloud/glm-5.3-flash:cloud");
    // graph: None serializes as explicit null (no skip attribute).
    assert_eq!(v["graph"], Value::Null);
}

#[test]
fn session_create_params_all_none_roundtrips() {
    let params = SessionCreateParams {
        cwd: "/tmp".to_string(),
        model: None,
        tools: vec![],
        graph: None,
    };
    let s = serde_json::to_string(&params).unwrap();
    let back: SessionCreateParams = serde_json::from_str(&s).unwrap();
    assert_eq!(back, params);
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["model"], Value::Null);
    assert_eq!(v["graph"], Value::Null);
}

#[test]
fn content_block_wire_shape_is_tagged() {
    // Hand-written wire JSON (internally tagged with "type") deserializes
    // into the enum.
    let block: ContentBlock = serde_json::from_str(r#"{"type": "text", "text": "hello"}"#)
        .expect("hand-written wire shape must deserialize");
    assert_eq!(
        block,
        ContentBlock::Text {
            text: "hello".to_string()
        }
    );

    // And the serialization is exactly that tagged shape.
    let block = ContentBlock::Text {
        text: "wörld\n".to_string(),
    };
    let s = serde_json::to_string(&block).unwrap();
    assert_eq!(s, r#"{"type":"text","text":"wörld\n"}"#);
    let back: ContentBlock = serde_json::from_str(&s).unwrap();
    assert_eq!(back, block);
}

// --------------------------------------------------------------------- sys --

#[test]
fn hello_result_roundtrips() {
    let result = HelloResult {
        protocol: "1".to_string(),
        server: PeerInfo {
            name: "pii".to_string(),
            version: "0.1.0".to_string(),
            build: "debug".to_string(),
        },
        encoding: "json".to_string(),
        capabilities: vec!["graph".to_string(), "events".to_string()],
        limits: Limits {
            max_frame_bytes: 1_048_576,
            max_subscriptions: 64,
        },
    };
    let s = serde_json::to_string(&result).unwrap();
    let back: HelloResult = serde_json::from_str(&s).unwrap();
    assert_eq!(back, result);

    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["protocol"], "1");
    assert_eq!(v["server"]["name"], "pii");
    assert_eq!(v["limits"]["max_frame_bytes"], 1_048_576);
}

#[test]
fn status_result_roundtrips() {
    let result = StatusResult {
        uptime_ms: 60_000,
        runs_active: 2,
        sessions: 3,
        sandbox_profile: "default".to_string(),
        protocol: "1".to_string(),
        rss_kb: 12_345,
        workers: vec![WorkerStatus {
            name: "w1".to_string(),
            state: "ready".to_string(),
            pid: 4242,
            restarts: 1,
        }],
        listeners: vec![ListenerStatus {
            id: 7,
            protocol: "pii-proto".to_string(),
            addr: "127.0.0.1:4601".to_string(),
        }],
        channels: vec![ChannelStatus {
            name: "discord-main".to_string(),
            kind: "discord".to_string(),
            errors: 0,
            cursor: 99,
        }],
        memory: MemoryStatus { docs: 12 },
    };
    let s = serde_json::to_string(&result).unwrap();
    let back: StatusResult = serde_json::from_str(&s).unwrap();
    assert_eq!(back, result);

    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["workers"][0]["name"], "w1");
    assert_eq!(v["listeners"][0]["addr"], "127.0.0.1:4601");
    assert_eq!(v["channels"][0]["kind"], "discord");
    assert_eq!(v["memory"]["docs"], 12);
    // Channel wire shape never carries a token (F6.8).
    assert!(!v["channels"][0].as_object().unwrap().contains_key("token"));
}
