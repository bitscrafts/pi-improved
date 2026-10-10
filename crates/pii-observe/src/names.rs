//! Name registry: the vocabulary of event kinds, measure names and subsystems.
//!
//! Call sites and tests use these symbols, not literals. Every constant must
//! also be listed in [`ALL`]; a unit test checks the list has no duplicates
//! or empty names.

/// Metric: nodes newly marked dead.
pub const GRAPH_NODE_DEAD: &str = "graph.node.dead";
/// Metric: a join that can never be satisfied.
pub const GRAPH_JOIN_UNSATISFIABLE: &str = "graph.join.unsatisfiable";
/// Metric: width of an executed batch.
pub const GRAPH_BATCH_WIDTH: &str = "graph.batch.width";
/// Event: a durable-store write failed.
pub const STORE_PERSIST_ERROR: &str = "store.persist.error";
/// Event: a capability was denied.
pub const CORE_CAP_DENIED: &str = "core.cap.denied";
/// Event: a subsystem's health changed.
pub const HEALTH_STATE_CHANGED: &str = "health.state.changed";
/// Measure: observer events dropped.
pub const OBSERVE_EVENTS_DROPPED: &str = "observe.events.dropped";
/// Event: an egress request was denied by the net policy.
pub const NET_EGRESS_DENIED: &str = "net.egress.denied";
/// Event: an outbound dial failed.
pub const NET_DIAL_FAILED: &str = "net.dial.failed";
/// Event: accepting an inbound connection failed.
pub const NET_ACCEPT_FAILED: &str = "net.accept.failed";
/// Event: a tool was invoked (tool name and outcome).
pub const TOOLS_CALL: &str = "tools.call";
/// Event: the sandbox profile in force at daemon start (profile, grant, enforced).
pub const DAEMON_SANDBOX_IN_FORCE: &str = "daemon.sandbox.in_force";
/// Event: a bash command matched the deny list.
pub const TOOLS_DENY_LIST_HIT: &str = "tools.deny_list.hit";
/// Event: the provider and model a request resolved to.
pub const MODEL_PROVIDER_IN_FORCE: &str = "model.provider.in_force";
/// Event: the model tier and model spec an agent node resolved to (spec 114).
pub const MODEL_TIER_RESOLVED: &str = "model.tier.resolved";
/// Event: a configured model tier's provider could not be built; runs naming it refuse (spec 114).
pub const MODEL_TIER_UNAVAILABLE: &str = "model.tier.unavailable";
/// Measure: model request latency.
pub const MODEL_REQUEST_LATENCY: &str = "model.request.latency";
/// Event: a model provider returned an error.
pub const MODEL_PROVIDER_ERROR: &str = "model.provider.error";
/// Event: a hosted provider rejected the credential (401/403); names the host, never the key.
pub const MODEL_PROVIDER_AUTH_FAILED: &str = "model.provider.auth_failed";
/// Event: a hosted provider rate-limited the request (429).
pub const MODEL_PROVIDER_RATE_LIMITED: &str = "model.provider.rate_limited";
/// Event: a hosted provider answered 2xx with a body that does not parse.
pub const MODEL_PROVIDER_BAD_BODY: &str = "model.provider.bad_body";
/// Event: the egress allowlist refused a provider's host.
pub const MODEL_PROVIDER_EGRESS_DENIED: &str = "model.provider.egress_denied";
/// Event: one partial-token delta from a streaming provider.
pub const MODEL_PROVIDER_STREAM_DELTA: &str = "model.provider.stream_delta";
/// Event: a supervised worker changed state.
pub const WORKER_STATE_CHANGED: &str = "worker.state.changed";
/// Event: a request to a worker was refused or cancelled around a drain/reload, with a named error
/// to its caller (`worker`, `op`, `reason`: `queue_full` | `queue_timeout` | `reload_deadline`).
pub const WORKER_REQUEST_REFUSED: &str = "worker.request.refused";
/// Event: a worker reload finished (`worker`, `old_pid`, `new_pid`, `in_flight_at_start`,
/// `queued_flushed`, `aborted`); `0`/`0`/`0` means nothing was in flight.
pub const WORKER_RELOAD_COMPLETED: &str = "worker.reload.completed";
/// Event: a memory search completed.
pub const MEMORY_SEARCH: &str = "memory.search";
/// Event: a memory consolidation completed.
pub const MEMORY_CONSOLIDATE: &str = "memory.consolidate";
/// Event: an operator-facing CLI failure (startup, daemon connect, graph error).
pub const CLI_FAILURE: &str = "cli.failure";
/// Event: a channel poll failed.
pub const CHANNEL_POLL_FAILED: &str = "channel.poll.failed";
/// Event: a channel send failed.
pub const CHANNEL_SEND_FAILED: &str = "channel.send.failed";
/// Event: a channel rejected a credential (an inbound request without the
/// shared secret, or an upstream API answering 401/403).
pub const CHANNEL_AUTH_REJECTED: &str = "channel.auth.rejected";
/// Event: a channel's outbound call was refused by the egress policy.
pub const CHANNEL_EGRESS_DENIED: &str = "channel.egress.denied";
/// Event: a channel was built misconfigured (e.g. a missing token); it polls
/// nothing and reports `Degraded`.
pub const CHANNEL_MISCONFIGURED: &str = "channel.misconfigured";
/// Measure: messages polled but not yet acknowledged past the cursor.
pub const CHANNEL_CURSOR_LAG: &str = "channel.cursor.lag";
/// Event: an MQTT channel connected to its broker (`channel`, `broker`).
pub const MQTT_CONNECTED: &str = "mqtt.connected";
/// Event: an MQTT channel lost or failed to reach its broker (`channel`, `broker`, `error`, `attempt`, `retry_in_ms`).
pub const MQTT_CONNECTION_LOST: &str = "mqtt.connection.lost";
/// Event: an MQTT message was accepted into the inbound queue (`channel`, `topic`, `bytes`).
pub const MQTT_MESSAGE_RECEIVED: &str = "mqtt.message.received";
/// Event: an MQTT message was dropped and counted (`channel`, `topic`, `reason`).
pub const MQTT_MESSAGE_DROPPED: &str = "mqtt.message.dropped";
/// Event: an MQTT message was written to the broker connection (`channel`, `topic`, `qos`, `retain`, `bytes`).
pub const MQTT_PUBLISHED: &str = "mqtt.published";
/// Event: credentials are being sent over plaintext to a non-loopback broker because the config opted in (`channel`, `broker`).
pub const MQTT_PLAINTEXT_CREDENTIALS: &str = "mqtt.plaintext_credentials";

/// Event: a TUI render or attach failure.
pub const TUI_FAILURE: &str = "tui.failure";
/// Event: a reattach resumed after a gap in the stream.
pub const TUI_REATTACH_GAP: &str = "tui.reattach.gap";
/// Event: a web request failed.
pub const WEB_REQUEST_FAILED: &str = "web.request.failed";
/// Event: the daemon (upstream) returned an error to the web proxy.
pub const WEB_UPSTREAM_ERROR: &str = "web.upstream.error";
/// Event: an MCP tool call failed.
pub const MCP_TOOL_CALL_FAILED: &str = "mcp.tool_call.failed";
/// Event: an evolve cycle started.
pub const EVOLVE_CYCLE_STARTED: &str = "evolve.cycle.started";
/// Event: an evolve cycle finished.
pub const EVOLVE_CYCLE_FINISHED: &str = "evolve.cycle.finished";
/// Event: launching the boss graph failed.
pub const EVOLVE_LAUNCH_FAILED: &str = "evolve.launch.failed";
/// Event: an `http_fetch` call completed (status or error).
pub const FETCH_OUTCOME: &str = "fetch.outcome";
/// Event: an `http_fetch` target was refused by the allowlist.
pub const FETCH_ALLOWLIST_DENIED: &str = "fetch.allowlist.denied";
/// Event: one journal event of a run (trajectory record; `seq` + `event`).
pub const RUN_JOURNAL: &str = "run.journal";
/// Event: a run's terminal outcome, carried verbatim.
pub const RUN_OUTCOME: &str = "run.outcome";
/// Event: a run's trajectory hit its size cap and further events were dropped.
pub const TRAJECTORY_TRUNCATED: &str = "trajectory.truncated";
/// Event: writing a trajectory failed (the run is not aborted).
pub const TRAJECTORY_WRITE_FAILED: &str = "trajectory.write.failed";
/// Event: the retention policy pruned old trajectories (`files`, `bytes`, `remaining_files`, `remaining_bytes`).
pub const TRAJECTORY_PRUNED: &str = "trajectory.pruned";
/// Event: the retention policy cannot be met because only in-flight runs remain (`files`, `bytes`).
pub const TRAJECTORY_OVER_BUDGET: &str = "trajectory.over_budget";
/// Event: a run's start was refused after its id was handed out; the run is failed, not lost (`run`, `error`).
pub const RUN_START_REFUSED: &str = "run.start.refused";
/// Event: the daemon finished starting and is serving.
pub const DAEMON_STARTED: &str = "daemon.started";
/// Event: the daemon is shutting down (`reason`: signal or forced).
pub const DAEMON_STOPPED: &str = "daemon.stopped";
/// Event: a connecting peer was refused (uid mismatch or unreadable credential).
pub const DAEMON_PEER_REFUSED: &str = "daemon.peer.refused";
/// Event: the daemon runs with its observer fan-out detached (`--observer off`); measurement only.
pub const DAEMON_OBSERVER_DETACHED: &str = "daemon.observer.detached";
/// Event: the daemon runs with its audit feed disabled (`--audit off`); measurement only.
pub const DAEMON_AUDIT_DISABLED: &str = "daemon.audit.disabled";
/// Event: writing the daemon log file failed (the daemon keeps running).
pub const LOG_WRITE_FAILED: &str = "log.write.failed";

/// Event: the peer did not answer the hello within the handshake bound (`endpoint`, `bound_ms`).
pub const PROTO_HANDSHAKE_TIMEOUT: &str = "proto.handshake.timeout";
/// Event: the peer hung up before answering the hello (`endpoint`).
pub const PROTO_HANDSHAKE_CLOSED: &str = "proto.handshake.closed";
/// Event: the peer speaks another protocol version (`endpoint`, `got`, `want`).
pub const PROTO_HANDSHAKE_SKEW: &str = "proto.handshake.skew";
/// Event: the handshake failed for another reason (`endpoint`, `error`).
pub const PROTO_HANDSHAKE_FAILED: &str = "proto.handshake.failed";
/// Event: the client could not connect to a daemon socket (`endpoint`, `error`).
pub const UDS_CONNECT_FAILED: &str = "uds.connect.failed";

/// Event: an SOP step started (`sop`, `step`, `kind`).
pub const SOP_STEP_STARTED: &str = "sop.step.started";
/// Event: an SOP step finished (`sop`, `step`, `kind`).
pub const SOP_STEP_FINISHED: &str = "sop.step.finished";
/// Event: an SOP step failed and ended the run (`sop`, `step`, `kind`, `reason`).
pub const SOP_STEP_FAILED: &str = "sop.step.failed";
/// Event: an SOP approval step is waiting on a human (`sop`, `step`, `kind`, `approval`).
pub const SOP_STEP_AWAITING_APPROVAL: &str = "sop.step.awaiting_approval";

/// Event: a HAL pin or bus was claimed (`kind`: `gpio`, `i2c` or `spi`; `pin`, or `bus`+`addr`, or `bus`; `dir` for a pin).
pub const HAL_CLAIM: &str = "hal.claim";
/// Event: a HAL pin or bus was released (same target fields as the claim).
pub const HAL_RELEASE: &str = "hal.release";
/// Event: a claimed pin was read (`pin`, `level`).
pub const HAL_READ: &str = "hal.read";
/// Event: a claimed pin was written (`pin`, `level`).
pub const HAL_WRITE: &str = "hal.write";
/// Event: an I2C transaction or SPI transfer completed (`kind`, `bus`, `addr` for I2C, `write_len`, `read_len`).
pub const HAL_TRANSACTION: &str = "hal.transaction";
/// Event: a HAL operation was refused or failed (`op`, the target fields, `reason`: `unclaimed`, `out_of_scope`, `already_claimed`, `wrong_direction`, `no_backend` or `backend`; `error`).
pub const HAL_ERROR: &str = "hal.error";

/// Event: a wasm module was read and compiled (`module`, `entry`, `bytes`).
pub const WASM_LOAD: &str = "wasm.load";
/// Event: a wasm module was instantiated with its limits (`module`, `entry`, `memory_limit`, `timeout_ms`, `fuel`).
pub const WASM_INSTANTIATE: &str = "wasm.instantiate";
/// Event: a wasm call returned a valid JSON value (`module`, `entry`, `elapsed_ms`, `out_bytes`).
pub const WASM_CALL: &str = "wasm.call";
/// Event: a wasm call trapped (`module`, `entry`, `trap`, `backtrace`).
pub const WASM_TRAP: &str = "wasm.trap";
/// Event: a wasm call hit its wall-clock epoch deadline (`module`, `entry`, `elapsed_ms`, `budget_ms`).
pub const WASM_TIMEOUT: &str = "wasm.timeout";
/// Event: a wasm call ran out of fuel (`module`, `entry`, `fuel`).
pub const WASM_FUEL_EXHAUSTED: &str = "wasm.fuel_exhausted";
/// Event: a wasm instance was refused memory beyond its limit (`module`, `entry`, `memory_limit`).
pub const WASM_MEMORY_LIMIT: &str = "wasm.memory_limit";
/// Event: a wasm module or call was refused before running (`module`, `entry`, `reason`: `not_configured`, `out_of_scope`, `unreadable`, `too_large`, `imports`, `bad_module`, `bad_abi` or `invalid_output`; `error`).
pub const WASM_REFUSAL: &str = "wasm.refusal";

/// Metric (gauge): runs in the main store (`pii metrics`, spec 105).
pub const METRIC_RUNS: &str = "pii_runs";
/// Metric (gauge): sessions in the main store.
pub const METRIC_SESSIONS: &str = "pii_sessions";
/// Metric (gauge): approvals awaiting a decision.
pub const METRIC_APPROVALS_PENDING: &str = "pii_approvals_pending";
/// Metric (counter): journal events ever appended to the main store.
pub const METRIC_JOURNAL_EVENTS_TOTAL: &str = "pii_journal_events_total";
/// Metric (gauge): memory documents.
pub const METRIC_MEMORY_DOCS: &str = "pii_memory_docs";
/// Metric (gauge): configured cron entries.
pub const METRIC_CRON_ENTRIES: &str = "pii_cron_entries";
/// Metric (gauge, label `store`): size of a store file in bytes.
pub const METRIC_STORE_BYTES: &str = "pii_store_bytes";
/// Metric (gauge, label `store`): persisted format version of a store.
pub const METRIC_STORE_FORMAT_VERSION: &str = "pii_store_format_version";
/// Metric (gauge): bytes of RSS growth per run over the recent window (spec 116; 0 until measurable).
pub const METRIC_RSS_BYTES_PER_RUN: &str = "pii_rss_bytes_per_run";
/// Metric (gauge): process resident set size in KiB (`sys.status` `rss_kb`).
pub const METRIC_PROCESS_RSS_KB: &str = "pii_process_rss_kb";
/// Metric (gauge): main-store bytes divided by runs (0 with no runs); the growth rate as one number.
pub const METRIC_STORE_BYTES_PER_RUN: &str = "pii_store_bytes_per_run";
/// Metric (gauge, labels `outcome`, `quantile`): run-lifecycle latency in ms from the bounded histogram (upper bucket bound).
pub const METRIC_RUN_LATENCY_MS: &str = "pii_run_latency_ms";
/// Metric (counter, label `outcome`): runs recorded in the latency histogram.
pub const METRIC_RUN_LATENCY_COUNT: &str = "pii_run_latency_count";
/// Metric (gauge): daemon uptime in milliseconds.
pub const METRIC_UPTIME_MS: &str = "pii_uptime_ms";
/// Metric (counter): MQTT messages accepted from brokers since the daemon started.
pub const METRIC_MQTT_RECEIVED_TOTAL: &str = "pii_mqtt_received_total";
/// Metric (counter): MQTT messages dropped (queue full, not UTF-8) since the daemon started.
pub const METRIC_MQTT_DROPPED_TOTAL: &str = "pii_mqtt_dropped_total";
/// Metric (counter): MQTT messages published since the daemon started.
pub const METRIC_MQTT_PUBLISHED_TOTAL: &str = "pii_mqtt_published_total";

/// Metric (counter): worker requests refused or cancelled around a drain/reload (spec 120).
pub const METRIC_WORKER_REQUESTS_REFUSED_TOTAL: &str = "pii_worker_requests_refused_total";

/// Metric (counter, label `kind`): HAL operations that succeeded since the daemon started (spec 121).
pub const METRIC_HAL_OPS_TOTAL: &str = "pii_hal_ops_total";
/// Metric (counter): HAL operations refused by the claim or scope rules since the daemon started (spec 121).
pub const METRIC_HAL_REFUSALS_TOTAL: &str = "pii_hal_refusals_total";

/// Metric (counter, label `outcome`): wasm calls by outcome since the daemon started (spec 122).
pub const METRIC_WASM_CALLS_TOTAL: &str = "pii_wasm_calls_total";
/// Metric (counter): wasm modules or calls refused before running (spec 122).
pub const METRIC_WASM_REFUSALS_TOTAL: &str = "pii_wasm_refusals_total";

/// Every registered name; keep in sync with the constants above.
pub const ALL: &[&str] = &[
    GRAPH_NODE_DEAD,
    GRAPH_JOIN_UNSATISFIABLE,
    GRAPH_BATCH_WIDTH,
    STORE_PERSIST_ERROR,
    CORE_CAP_DENIED,
    HEALTH_STATE_CHANGED,
    OBSERVE_EVENTS_DROPPED,
    NET_EGRESS_DENIED,
    NET_DIAL_FAILED,
    NET_ACCEPT_FAILED,
    TOOLS_CALL,
    TOOLS_DENY_LIST_HIT,
    DAEMON_SANDBOX_IN_FORCE,
    MODEL_PROVIDER_IN_FORCE,
    MODEL_TIER_RESOLVED,
    MODEL_TIER_UNAVAILABLE,
    MODEL_REQUEST_LATENCY,
    MODEL_PROVIDER_ERROR,
    MODEL_PROVIDER_AUTH_FAILED,
    MODEL_PROVIDER_RATE_LIMITED,
    MODEL_PROVIDER_BAD_BODY,
    MODEL_PROVIDER_EGRESS_DENIED,
    MODEL_PROVIDER_STREAM_DELTA,
    WORKER_STATE_CHANGED,
    WORKER_REQUEST_REFUSED,
    WORKER_RELOAD_COMPLETED,
    MEMORY_SEARCH,
    MEMORY_CONSOLIDATE,
    CLI_FAILURE,
    CHANNEL_POLL_FAILED,
    CHANNEL_SEND_FAILED,
    CHANNEL_AUTH_REJECTED,
    CHANNEL_EGRESS_DENIED,
    CHANNEL_MISCONFIGURED,
    CHANNEL_CURSOR_LAG,
    MQTT_CONNECTED,
    MQTT_CONNECTION_LOST,
    MQTT_MESSAGE_RECEIVED,
    MQTT_MESSAGE_DROPPED,
    MQTT_PUBLISHED,
    MQTT_PLAINTEXT_CREDENTIALS,
    TUI_FAILURE,
    TUI_REATTACH_GAP,
    WEB_REQUEST_FAILED,
    WEB_UPSTREAM_ERROR,
    MCP_TOOL_CALL_FAILED,
    EVOLVE_CYCLE_STARTED,
    EVOLVE_CYCLE_FINISHED,
    EVOLVE_LAUNCH_FAILED,
    FETCH_OUTCOME,
    FETCH_ALLOWLIST_DENIED,
    RUN_JOURNAL,
    RUN_OUTCOME,
    TRAJECTORY_TRUNCATED,
    TRAJECTORY_WRITE_FAILED,
    TRAJECTORY_PRUNED,
    TRAJECTORY_OVER_BUDGET,
    RUN_START_REFUSED,
    DAEMON_STARTED,
    DAEMON_STOPPED,
    DAEMON_PEER_REFUSED,
    DAEMON_OBSERVER_DETACHED,
    DAEMON_AUDIT_DISABLED,
    LOG_WRITE_FAILED,
    PROTO_HANDSHAKE_TIMEOUT,
    PROTO_HANDSHAKE_CLOSED,
    PROTO_HANDSHAKE_SKEW,
    PROTO_HANDSHAKE_FAILED,
    UDS_CONNECT_FAILED,
    SOP_STEP_STARTED,
    SOP_STEP_FINISHED,
    SOP_STEP_FAILED,
    SOP_STEP_AWAITING_APPROVAL,
    HAL_CLAIM,
    HAL_RELEASE,
    HAL_READ,
    HAL_WRITE,
    HAL_TRANSACTION,
    HAL_ERROR,
    WASM_LOAD,
    WASM_INSTANTIATE,
    WASM_CALL,
    WASM_TRAP,
    WASM_TIMEOUT,
    WASM_FUEL_EXHAUSTED,
    WASM_MEMORY_LIMIT,
    WASM_REFUSAL,
    METRIC_RUNS,
    METRIC_SESSIONS,
    METRIC_APPROVALS_PENDING,
    METRIC_JOURNAL_EVENTS_TOTAL,
    METRIC_MEMORY_DOCS,
    METRIC_CRON_ENTRIES,
    METRIC_STORE_BYTES,
    METRIC_STORE_FORMAT_VERSION,
    METRIC_PROCESS_RSS_KB,
    METRIC_RSS_BYTES_PER_RUN,
    METRIC_STORE_BYTES_PER_RUN,
    METRIC_RUN_LATENCY_MS,
    METRIC_RUN_LATENCY_COUNT,
    METRIC_UPTIME_MS,
    METRIC_MQTT_RECEIVED_TOTAL,
    METRIC_MQTT_DROPPED_TOTAL,
    METRIC_MQTT_PUBLISHED_TOTAL,
    METRIC_WORKER_REQUESTS_REFUSED_TOTAL,
    METRIC_HAL_OPS_TOTAL,
    METRIC_HAL_REFUSALS_TOTAL,
    METRIC_WASM_CALLS_TOTAL,
    METRIC_WASM_REFUSALS_TOTAL,
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn registry_has_no_duplicates_and_no_empties() {
        let mut seen = std::collections::BTreeSet::new();
        for n in ALL {
            assert!(!n.is_empty(), "empty name in registry");
            assert!(seen.insert(*n), "duplicate name {n}");
        }
    }

    #[test]
    fn the_sop_step_events_are_registered() {
        for n in [
            "sop.step.started",
            "sop.step.finished",
            "sop.step.failed",
            "sop.step.awaiting_approval",
        ] {
            assert!(ALL.contains(&n), "{n} missing from ALL");
        }
    }

    #[test]
    fn existing_vocabulary_is_preserved() {
        assert!(ALL.contains(&"graph.node.dead"));
        assert!(ALL.contains(&"graph.join.unsatisfiable"));
        assert!(ALL.contains(&"graph.batch.width"));
    }
}
