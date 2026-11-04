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
/// Event: a replay found an effect started and never confirmed; it refuses (spec 186).
pub const EFFECT_UNKNOWN: &str = "effect.unknown";
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
/// Event: a work item was added to the queue from outside (spec 352 R6).
pub const QUEUE_ITEM_ADDED: &str = "queue.item.added";
/// Event: a work item was claimed from the queue (`id`, `by`; spec 356).
pub const QUEUE_ITEM_CLAIMED: &str = "queue.item.claimed";
/// Event: a bash command matched the deny list.
pub const TOOLS_DENY_LIST_HIT: &str = "tools.deny_list.hit";
/// Event: the provider and model a request resolved to.
pub const MODEL_PROVIDER_IN_FORCE: &str = "model.provider.in_force";
/// Event: the model tier and model spec an agent node resolved to (spec 114).
pub const MODEL_TIER_RESOLVED: &str = "model.tier.resolved";
/// Event: a configured model tier's provider could not be built; runs naming it refuse (spec 114).
pub const MODEL_TIER_UNAVAILABLE: &str = "model.tier.unavailable";
/// Event: a retired `ollama:` model spec was refused (spec 129).
pub const MODEL_OLLAMA_REFUSED: &str = "model.ollama.refused";
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
/// Event: a coding-agent worker refused to start (`agent`, `model`, `reason`); nothing was spawned (spec 185).
pub const WORKER_START_REFUSED: &str = "worker.start.refused";
/// Event: a worker's respawn after death failed; the worker is left failed (`worker`, `error`; spec 356).
pub const WORKER_RESPAWN_FAILED: &str = "worker.respawn.failed";
/// Event: a coder node spawned its agent (`agent`, `model`, `pid`, `argv_digest`, `session`); no raw argv (spec 217).
pub const CODER_SPAWNED: &str = "coder.spawned";
/// Event: a coder node's image is absent or has no digest (`image`, `reason`); nothing was started (spec 209).
pub const CODER_IMAGE_MISSING: &str = "coder.image.missing";
/// Event: a coder node's criterion is not in the named table (`criterion`); nothing was spawned (spec 215).
pub const CRITERIA_UNKNOWN: &str = "criteria.unknown";
/// Event: a coder node's container runtime could not be reached or refused a call (`socket`, `status`, `reason`).
///
/// Nothing was spawned (spec 230). Never `coder.image.missing`: a runtime failure is not an absent image.
pub const CODER_RUNTIME_UNAVAILABLE: &str = "coder.runtime.unavailable";
/// Event: a coder node ran on the host because it names no `container_image` (`agent`); deprecated (spec 209).
pub const CODER_HOST_SPAWN_DEPRECATED: &str = "coder.host_spawn.deprecated";
/// Event: a container's logs could not be read after it exited (`container`); its exit code stands (spec 241).
pub const CODER_LOGS_UNREADABLE: &str = "coder.logs_unreadable";
/// Event: a coder call ended (`pid`, `rc`, `dur_ms`, `stop_reason`, criteria counts, `first_failure`) (spec 217).
pub const CODER_EXITED: &str = "coder.exited";
/// Event: the environment names a coder call was given (`names`, comma-joined; never a value) (spec 217 R3).
pub const CODER_ENV_NAMES: &str = "coder.env.names";
/// Event: a coder call's environment was refused because a name would journal a value (spec 217 R3).
pub const CODER_ENV_REFUSED: &str = "coder.env.refused";
/// Event: boot recovery reaped an agent a dead daemon left (`run`, `kind`, `id`, `result`: `killed`|`already_gone`|`failed`) (spec 242).
pub const CODER_ORPHAN_REAPED: &str = "coder.orphan_reaped";
/// Event: a pinned skill's stored text no longer hashes to its pin (`name`, `pinned`, `found`); nothing spawned (spec 225).
pub const SKILL_DRIFT: &str = "skill.drift";
/// Event: a memory search completed.
pub const MEMORY_SEARCH: &str = "memory.search";
/// Event: a memory consolidation completed.
pub const MEMORY_CONSOLIDATE: &str = "memory.consolidate";
/// Event: an entity was created or merged (`id`, `kind`, `outcome`: `created`, `merged` or `unchanged`; `provenance`).
pub const MEMORY_ENTITY_WRITTEN: &str = "memory.entity_written";
/// Event: a relationship was created or merged (`id`, `kind`, `from`, `to`, `outcome`, `provenance`).
pub const MEMORY_RELATIONSHIP_WRITTEN: &str = "memory.relationship_written";
/// Event: an entity or relationship write was refused (`what`: `entity` or `relationship`; `reason`: `no_provenance`, `bad_provenance`, `invalid`, `too_large`, `entity_cap`, `relationship_cap`, `provenance_cap` or `unknown_entity`).
pub const MEMORY_WRITE_REFUSED: &str = "memory.write_refused";
/// Event: an entity query was refused (`op`: `get`, `by_kind`, `by_name` or `neighbours`; `reason`: `bad_query`, `bad_depth`, `depth_cap`, `result_cap` or `unknown_entity`).
pub const MEMORY_QUERY_REFUSED: &str = "memory.query_refused";
/// Event: a record's tier changed (`id`, `from`, `to`: `hot`, `warm` or `cold`) (spec 127).
pub const MEMORY_TIER_CHANGED: &str = "memory.tier_changed";
/// Event: records were archived and removed after a verified dump (`path`, `records`, `bytes`) (spec 127).
pub const MEMORY_ARCHIVED: &str = "memory.archived";
/// Event: an archive or restore was refused and nothing was removed (`op`: `archive` or `restore`; `reason`; `error`) (spec 127).
pub const MEMORY_ARCHIVE_REFUSED: &str = "memory.archive_refused";
/// Event: an archive was restored (`file`, `restored`, `unchanged`, `refused`) (spec 127).
pub const MEMORY_RESTORED: &str = "memory.restored";
/// Event: an operator-facing CLI failure (startup, daemon connect, graph error).
pub const CLI_FAILURE: &str = "cli.failure";
/// Event: a lint registry's store could not be opened, so its kinds fail lint as unknown (`store`, `reason`) (spec 227 R7).
pub const LINT_STORE_UNAVAILABLE: &str = "lint.store.unavailable";
/// Event: a cron entry could not be started (`cron`, `graph`, `error`; spec 177).
pub const CRON_START_REFUSED: &str = "cron.start.refused";
/// Event: a start found a suspended run already armed for a cron entry and kept it (`cron`, `graph`, `run`; spec 187).
pub const CRON_REARMED: &str = "cron.rearmed";
/// Event: a cron entry started a run (`cron`, `graph`, `run`; spec 193).
pub const CRON_FIRED: &str = "cron.fired";
/// Event: a channel message was received but its run could not be started (`channel`, `graph`, `error`; spec 177).
pub const CHANNEL_RUN_REFUSED: &str = "channel.run.refused";
/// Event: a channel poll failed.
pub const CHANNEL_POLL_FAILED: &str = "channel.poll.failed";
/// Event: a channel poll panicked; the poll counts as failed and the channel degrades (`channel`; spec 356).
pub const CHANNEL_POLL_PANICKED: &str = "channel.poll.panicked";
/// Event: a channel's cursor could not advance past a delivered message (`channel`, `run`, `error`; spec 356).
pub const CURSOR_ADVANCE_FAILED: &str = "cursor.advance.failed";
/// Event: a channel began failing (spec 347): one line per outage, not per attempt.
pub const CHANNEL_DEGRADED: &str = "channel.degraded";
/// Event: a failing channel answered again (spec 347).
pub const CHANNEL_RECOVERED: &str = "channel.recovered";
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
/// Event: a `post` node delivered a message into a channel (`channel`, `destination`, `bytes`; spec 128).
pub const CHANNEL_POSTED: &str = "channel.posted";
/// Event: a `post` node was refused or failed (`channel`, `reason`; spec 128).
pub const CHANNEL_POST_REFUSED: &str = "channel.post_refused";
/// Event: the startup post naming the deployed revision was delivered (`channel`, `text`; spec 157).
pub const CHANNEL_VERIFICATION_POSTED: &str = "channel.verification_posted";

/// The channel is ready to be talked to, and nothing was written to it.
///
/// The startup verification post is opt-in via `PII_VERIFY_POST=1`: a forum turns every message into a thread,
/// so a restart must not create one.
pub const CHANNEL_READY: &str = "channel.ready";
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
/// Event: an operator cancelled a not-running run (`run`, `reason`).
pub const RUN_CANCELLED: &str = "run.cancelled";
/// Event: a cancel was refused because a node is executing (`run`, `node`).
pub const RUN_CANCEL_REFUSED: &str = "run.cancel_refused";
/// Event: an operator asked to kill a running node; the run is being stopped (`run`, `node`, `reason`).
pub const RUN_CANCEL_KILLING: &str = "run.cancel_killing";
/// Event: a killed run did not leave flight within its wait; it is left as it is (`run`, `node`).
pub const RUN_CANCEL_KILL_TIMEOUT: &str = "run.cancel_kill_timeout";
/// Event: a resume of a finished run was refused, e.g. a due timer outliving a cancel (`run`, `status`).
pub const RUN_RESUME_REFUSED: &str = "run.resume.refused";
/// Event: the scheduler's resume of a due timer was refused (`run`, `key`, `error`).
pub const TIMER_RESUME_REFUSED: &str = "timer.resume.refused";
/// Event: the timer wheel could not be read or written; reported once per outage, then the scheduler backs off (`error`).
pub const TIMER_WHEEL_ERROR: &str = "timer.error";
/// Event: the scheduler took a due timer off the wheel and resumed its run (`run`, `key`; spec 356).
pub const TIMER_FIRED: &str = "timer.fired";
/// Event: a resume of a suspended run was refused; its timer is offered again while the run waits (`run`, `key`, `error`).
pub const RUN_RESUME_FAILED: &str = "run.resume.failed";
/// Event: boot reconciliation closed a run a crash left `Running` (`run`, `reason`).
pub const RUN_RECONCILED: &str = "run.reconciled";
/// Event: a node panicked; the run is failed, the daemon survives (`run`, `node`, `message`).
pub const NODE_PANICKED: &str = "node.panicked";
/// Event: the daemon is starting — written before the store opens, the engine is built or the socket
/// is bound, so a start that dies early still leaves a line (spec 322).
pub const DAEMON_STARTING: &str = "daemon.starting";
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

/// Event: an extraction scan produced candidates (`repeated_failure`, `stuck_loop`, `limit_hit`, `cost_outlier` counts).
pub const LEARN_CANDIDATES_EXTRACTED: &str = "learn.candidates_extracted";
/// Event: a candidate was refused instead of emitted (`reason`: `no_evidence`, `no_counts` or `no_claim`; `count`).
pub const LEARN_CANDIDATE_REFUSED: &str = "learn.candidate_refused";
/// Event: an extraction scan finished (`files_scanned`, `files_seen`, `bytes_read`, `corrupt_lines`, `duration_ms`).
pub const LEARN_SCAN_COMPLETED: &str = "learn.scan_completed";

/// Event: a shadow-run verification of one candidate began (`candidate`, `kind`).
pub const LEARN_VERIFICATION_STARTED: &str = "learn.verification_started";
/// Event: a candidate's claim reproduced in an isolated shadow run (`candidate`, `shadow`, `shadow_runs`, `shadow_failed`).
pub const LEARN_CANDIDATE_CONFIRMED: &str = "learn.candidate_confirmed";
/// Event: a candidate's claim did NOT reproduce - a successful verification (`candidate`, `shadow`, `shadow_runs`, `shadow_failed`).
pub const LEARN_CANDIDATE_REFUTED: &str = "learn.candidate_refuted";
/// Event: a candidate could not be decided (`candidate`, `reason`, `bound`, `elapsed_ms`).
pub const LEARN_CANDIDATE_INCONCLUSIVE: &str = "learn.candidate_inconclusive";

/// Event: one unattended extraction + verification pass ended (`candidates`, `new_candidates`, `verified`, `total_proposed`). Emitted even when all are 0.
pub const LEARN_UNATTENDED_RUN: &str = "learn.unattended_run";

/// Event: a promotion took effect (`promotion`, `kind`, `graph`, `metric`, `window_secs`, `ttl_secs`, `ledger`).
pub const LEARN_PROMOTION_APPLIED: &str = "learn.promotion_applied";
/// Event: a promotion was taken back, by a person or automatically (`promotion`, `kind`, `graph`, `metric`, `state`, `ledger`, and `before`/`after` when measured).
pub const LEARN_PROMOTION_REVERTED: &str = "learn.promotion_reverted";
/// Event: a promotion lapsed at its TTL (`promotion`, `kind`, `graph`, `metric`, `state`, `ledger`).
pub const LEARN_PROMOTION_EXPIRED: &str = "learn.promotion_expired";
/// Event: a promotion was refused, nothing changed (`candidate`, `kind`, `graph`, `reason`, `actor`).
pub const LEARN_PROMOTION_REFUSED: &str = "learn.promotion_refused";

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

/// Metric (counter): messages the `post` node delivered into channels since the daemon started (spec 128).
pub const METRIC_CHANNEL_POSTS_TOTAL: &str = "pii_channel_posts_total";
/// Metric (counter): `post` node calls refused or failed since the daemon started (spec 128).
pub const METRIC_CHANNEL_POST_REFUSALS_TOTAL: &str = "pii_channel_post_refusals_total";

/// Metric (counter): runs refused because a promotion quarantines their graph (spec 131).
pub const METRIC_QUARANTINE_REFUSALS_TOTAL: &str = "pii_quarantine_refusals_total";
/// Metric (counter): quarantine refusals whose deny decision reached the ledger; a gap to the line above is a finding (spec 131).
pub const METRIC_QUARANTINE_REFUSALS_RECORDED_TOTAL: &str =
    "pii_quarantine_refusals_recorded_total";

/// Metric (counter, label `kind`): HAL operations that succeeded since the daemon started (spec 121).
pub const METRIC_HAL_OPS_TOTAL: &str = "pii_hal_ops_total";
/// Metric (counter): HAL operations refused by the claim or scope rules since the daemon started (spec 121).
pub const METRIC_HAL_REFUSALS_TOTAL: &str = "pii_hal_refusals_total";

/// Metric (counter, label `outcome`): wasm calls by outcome since the daemon started (spec 122).
pub const METRIC_WASM_CALLS_TOTAL: &str = "pii_wasm_calls_total";
/// Metric (counter): wasm modules or calls refused before running (spec 122).
pub const METRIC_WASM_REFUSALS_TOTAL: &str = "pii_wasm_refusals_total";

/// Metric (gauge, label `kind`): candidate lessons from the last extraction scan (spec 123).
pub const METRIC_LEARN_CANDIDATES: &str = "pii_learn_candidates";
/// Metric (counter): candidates refused for missing evidence, across every scan (spec 123).
pub const METRIC_LEARN_REFUSALS_TOTAL: &str = "pii_learn_refusals_total";

/// Metric (counter): distinct candidates the unattended loop has proposed since it was enabled (spec 139).
pub const METRIC_LEARN_CANDIDATES_PROPOSED_TOTAL: &str = "pii_learn_candidates_proposed_total";
/// Metric (counter): unattended extraction + verification passes completed (spec 139).
pub const METRIC_LEARN_UNATTENDED_RUNS_TOTAL: &str = "pii_learn_unattended_runs_total";

/// Metric (gauge, label `verdict`): verdicts of the last verification pass (spec 124).
pub const METRIC_LEARN_VERDICTS: &str = "pii_learn_verdicts";

/// Metric (gauge, label `state`): promotions by state, plus `refused` (spec 125).
pub const METRIC_LEARN_PROMOTIONS: &str = "pii_learn_promotions";

/// Metric (gauge): entities in the memory graph (spec 126).
pub const METRIC_MEMORY_ENTITIES: &str = "pii_memory_entities";
/// Metric (gauge): relationships in the memory graph (spec 126).
pub const METRIC_MEMORY_RELATIONSHIPS: &str = "pii_memory_relationships";
/// Metric (counter, label `kind`): entity and relationship writes that changed the graph (spec 126).
pub const METRIC_MEMORY_WRITES_TOTAL: &str = "pii_memory_writes_total";
/// Metric (counter, label `reason`): memory graph writes refused (spec 126).
pub const METRIC_MEMORY_WRITE_REFUSALS_TOTAL: &str = "pii_memory_write_refusals_total";
/// Metric (counter, label `reason`): memory graph queries refused (spec 126).
pub const METRIC_MEMORY_QUERY_REFUSALS_TOTAL: &str = "pii_memory_query_refusals_total";

/// Metric (gauge, label `tier`): memory records in each tier (spec 127).
pub const METRIC_MEMORY_TIER: &str = "pii_memory_tier";
/// Metric (counter, label `change`): tier transitions seen, `<from>_to_<to>` (spec 127).
pub const METRIC_MEMORY_TIER_CHANGES_TOTAL: &str = "pii_memory_tier_changes_total";
/// Metric (counter, label `op`): archive operations by outcome, `archived`, `restored` or `refused` (spec 127).
pub const METRIC_MEMORY_ARCHIVE_OPS_TOTAL: &str = "pii_memory_archive_ops_total";

/// Every registered name; keep in sync with the constants above.
pub const ALL: &[&str] = &[
    GRAPH_NODE_DEAD,
    GRAPH_JOIN_UNSATISFIABLE,
    GRAPH_BATCH_WIDTH,
    EFFECT_UNKNOWN,
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
    QUEUE_ITEM_ADDED,
    QUEUE_ITEM_CLAIMED,
    MODEL_PROVIDER_IN_FORCE,
    MODEL_TIER_RESOLVED,
    MODEL_TIER_UNAVAILABLE,
    MODEL_OLLAMA_REFUSED,
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
    WORKER_START_REFUSED,
    WORKER_RESPAWN_FAILED,
    CODER_IMAGE_MISSING,
    CODER_RUNTIME_UNAVAILABLE,
    CODER_HOST_SPAWN_DEPRECATED,
    CODER_LOGS_UNREADABLE,
    CRITERIA_UNKNOWN,
    MEMORY_SEARCH,
    MEMORY_CONSOLIDATE,
    MEMORY_ENTITY_WRITTEN,
    MEMORY_RELATIONSHIP_WRITTEN,
    MEMORY_WRITE_REFUSED,
    MEMORY_QUERY_REFUSED,
    MEMORY_TIER_CHANGED,
    MEMORY_ARCHIVED,
    MEMORY_ARCHIVE_REFUSED,
    MEMORY_RESTORED,
    CLI_FAILURE,
    LINT_STORE_UNAVAILABLE,
    CODER_SPAWNED,
    CODER_EXITED,
    CODER_ENV_NAMES,
    CODER_ENV_REFUSED,
    CODER_ORPHAN_REAPED,
    SKILL_DRIFT,
    CRON_START_REFUSED,
    CRON_REARMED,
    CRON_FIRED,
    CHANNEL_RUN_REFUSED,
    CHANNEL_POLL_FAILED,
    CHANNEL_POLL_PANICKED,
    CURSOR_ADVANCE_FAILED,
    CHANNEL_DEGRADED,
    CHANNEL_RECOVERED,
    CHANNEL_SEND_FAILED,
    CHANNEL_AUTH_REJECTED,
    CHANNEL_EGRESS_DENIED,
    CHANNEL_MISCONFIGURED,
    CHANNEL_CURSOR_LAG,
    CHANNEL_POSTED,
    CHANNEL_POST_REFUSED,
    CHANNEL_READY,
    CHANNEL_VERIFICATION_POSTED,
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
    RUN_CANCELLED,
    RUN_CANCEL_REFUSED,
    RUN_CANCEL_KILLING,
    RUN_CANCEL_KILL_TIMEOUT,
    RUN_RESUME_REFUSED,
    TIMER_RESUME_REFUSED,
    TIMER_WHEEL_ERROR,
    TIMER_FIRED,
    RUN_RESUME_FAILED,
    NODE_PANICKED,
    RUN_RECONCILED,
    DAEMON_STARTING,
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
    LEARN_CANDIDATES_EXTRACTED,
    LEARN_CANDIDATE_REFUSED,
    LEARN_SCAN_COMPLETED,
    LEARN_VERIFICATION_STARTED,
    LEARN_CANDIDATE_CONFIRMED,
    LEARN_CANDIDATE_REFUTED,
    LEARN_CANDIDATE_INCONCLUSIVE,
    LEARN_UNATTENDED_RUN,
    LEARN_PROMOTION_APPLIED,
    LEARN_PROMOTION_REVERTED,
    LEARN_PROMOTION_EXPIRED,
    LEARN_PROMOTION_REFUSED,
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
    METRIC_CHANNEL_POSTS_TOTAL,
    METRIC_CHANNEL_POST_REFUSALS_TOTAL,
    METRIC_QUARANTINE_REFUSALS_TOTAL,
    METRIC_QUARANTINE_REFUSALS_RECORDED_TOTAL,
    METRIC_HAL_OPS_TOTAL,
    METRIC_HAL_REFUSALS_TOTAL,
    METRIC_WASM_CALLS_TOTAL,
    METRIC_WASM_REFUSALS_TOTAL,
    METRIC_LEARN_CANDIDATES,
    METRIC_LEARN_REFUSALS_TOTAL,
    METRIC_LEARN_CANDIDATES_PROPOSED_TOTAL,
    METRIC_LEARN_UNATTENDED_RUNS_TOTAL,
    METRIC_LEARN_VERDICTS,
    METRIC_LEARN_PROMOTIONS,
    METRIC_MEMORY_ENTITIES,
    METRIC_MEMORY_RELATIONSHIPS,
    METRIC_MEMORY_WRITES_TOTAL,
    METRIC_MEMORY_WRITE_REFUSALS_TOTAL,
    METRIC_MEMORY_QUERY_REFUSALS_TOTAL,
    METRIC_MEMORY_TIER,
    METRIC_MEMORY_TIER_CHANGES_TOTAL,
    METRIC_MEMORY_ARCHIVE_OPS_TOTAL,
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
