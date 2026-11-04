# Spec: One place names the models

**Project**: /projects/pi-improved · **Phase**: 0.2.1 · **Source**: user directive 2026-11-03 — *"the model is
deepseek-v4.1-flash; we should have all models configured in a single place"*; workhorse implements, planner writes.

## Verified premises (measured 2026-11-03)

- Seven graphs hardcode a model, and **three of them name an id that no longer exists**:
  `graphs/coder-demo.toml:17`, `graphs/coder-podman.toml:17`, `graphs/intervention-report.toml:34` all say
  `deepseek-v4-flash`. The first real run of `graphs/queue.toml` died in 1.6 s with
  `{"code":"internal","exit_code":1,"message":"Model deepseek-v4-flash not found"}`.
- `~/.pii/pii.toml` has no `[models]` table (`grep` → 0 hits).
- The provider's own model list (25 ids) contains `deepseek-v4.1-flash`, `deepseek-v4.1-flash:1017`,
  `glm-5.3` and `glm-5.3-flash`; a one-turn probe against `glm-5.3` answered with
  `"provider": "ollama-cloud", "model": "glm-5.3"`, rc 0.
- Two graphs already take the model from the run input (`graphs/docs.toml:15`, `graphs/spec.toml:23`), which is the
  reason a stale id can hide for months: nothing validates it until an agent is spawned.

## Requirements

**R1.** The config gains one table per role, and no graph needs to know a model id again:

```toml
[models.workhorse]        # implements a spec
model = "deepseek-v4.1-flash"
provider = "ollama-cloud"

[models.planner]          # writes a spec
model = "glm-5.3"
provider = "ollama-cloud"
```

**R2.** A node may name a role instead of a model: `model = "@workhorse"`. The daemon resolves it to the configured
`model` and `provider` when the node is built. An explicit model id keeps working exactly as today (a graph that
already names one must not change behaviour).

**R3.** An unknown role is a **refusal that names the role and the configured roles**, before any agent is spawned —
never a silent fallback to a default. Measured cost of the silent version: a 1.6 s spawn failure per row, and eight
rows would have parked for one dead id.

**R4.** `pii config check` (or the daemon's start-up) refuses a `[models]` entry whose model is not on the provider's
list when the provider is reachable, and says which entry and which id. When the provider cannot be reached, it says
so and continues — an offline start must not depend on the network.

## The requirement a plausible implementation satisfies in appearance only

A worker can add the table, resolve `@role` in one node kind, and leave the other call sites hardcoded — the config
exists, nothing uses it, and the next stale id hides in a graph nobody reads. R4 is the half that makes it real: a dead
id must fail at config time with its name, not at spawn time in a journal.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-server/tests/models_from_config.rs` | `a_role_resolves_to_the_configured_model` | a coder node with `model = "@workhorse"` spawns with `--model deepseek-v4.1-flash --provider ollama-cloud` |
| same | `CONTROL_an_explicit_model_is_untouched` | `model = "some-model"` reaches the argv unchanged |
| same | `an_unknown_role_is_refused_by_name` | `@nope` → refusal naming `nope` and the two configured roles; no process spawned |
| `crates/pii-config/tests/models_are_validated.rs` (or the crate that owns config) | `a_dead_model_is_refused_at_config_time` | an entry naming `deepseek-v4-flash` is refused with that id in the message (fixture: the provider list captured as JSON, no network) |

## Decision (recorded 2026-11-04)

The operator chose **option (a): split the file**, not raise the 450-line budget. The extraction is
`ModelRole`, `resolve_role` and `empty_model_roles` out of `daemon/config.rs` into a new
`daemon/model_roles.rs`, re-exported from `daemon/mod.rs`. This is the same extraction that solved
the size rule twice already (`daemon/sys_status.rs`, `store/sys_status.rs`); raising the budget for
one file would weaken a repo-wide rule to fit a single change.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test models_from_config 2>&1 | grep -qE 'test result: ok. [0-9]+ passed'`
- [ ] `grep -q '@workhorse' graphs/queue.toml`
- [ ] `! grep -rq 'deepseek-v4-flash' graphs/`
- [ ] `grep -q 'model_roles' crates/pii-server/src/daemon/mod.rs`
- [ ] `grep -q 'ModelRole' crates/pii-server/src/daemon/model_roles.rs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | grep -qv 'exceeds 450'`

## Guardrails

- Do NOT remove the ability to name a model explicitly; several graphs pass one from the run input.
- Do NOT put a credential in the config: the key stays an env reference, as today.
- Do NOT default an unknown role (R3).
- The worker does not commit; the orchestrator reviews and commits.

## Files

Adding a required field to Config touches every literal that builds one; that blast
radius is part of the change, not scope creep.

- `crates/pii-cli/src/cmd_daemon.rs`
- `crates/pii-cli/src/cmd_oneshot.rs`
- `crates/pii-cli/src/cmd_tui.rs`
- `crates/pii-server/src/daemon/config.rs`
- `crates/pii-server/src/daemon/identity.rs`
- `crates/pii-server/src/daemon/lag.rs`
- `crates/pii-server/src/daemon/mod.rs`
- `crates/pii-server/src/daemon/model_roles.rs`
- `crates/pii-server/src/daemon/refusal_tests.rs`
- `crates/pii-server/src/daemon/router.rs`
- `crates/pii-server/src/daemon/scheduler.rs`
- `crates/pii-server/src/daemon/suspend.rs`
- `crates/pii-server/src/engine/conn_heartbeat_tests.rs`
- `crates/pii-server/src/engine/inline_measure_tests.rs`
- `crates/pii-server/src/engine/reconcile_tests.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/src/engine/run_scope.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/node_kinds/nodes/coder.rs`
- `crates/pii-server/tests/acked_run_survives_kill.rs`
- `crates/pii-server/tests/approvals.rs`
- `crates/pii-server/tests/boot_recovers_suspended.rs`
- `crates/pii-server/tests/boss_watch.rs`
- `crates/pii-server/tests/cancel_disarms_the_timer.rs`
- `crates/pii-server/tests/cancel_kill.rs`
- `crates/pii-server/tests/cancelled_run_disarms.rs`
- `crates/pii-server/tests/cancelled_run_has_a_duration.rs`
- `crates/pii-server/tests/channel_adapters.rs`
- `crates/pii-server/tests/channel_wiring.rs`
- `crates/pii-server/tests/cron_arming_is_idempotent.rs`
- `crates/pii-server/tests/cron_runtime.rs`
- `crates/pii-server/tests/daemon_graphs.rs`
- `crates/pii-server/tests/daemon_journals_calls.rs`
- `crates/pii-server/tests/durable_store_placement.rs`
- `crates/pii-server/tests/entities_e2e.rs`
- `crates/pii-server/tests/hal_e2e.rs`
- `crates/pii-server/tests/handshake.rs`
- `crates/pii-server/tests/health_identity.rs`
- `crates/pii-server/tests/latency.rs`
- `crates/pii-server/tests/live_registry_refuses_a_drifted_skill.rs`
- `crates/pii-server/tests/live_registry_supplies_the_skill_source.rs`
- `crates/pii-server/tests/log_wiring.rs`
- `crates/pii-server/tests/memory_rpc.rs`
- `crates/pii-server/tests/model_routing.rs`
- `crates/pii-server/tests/models_are_validated.rs`
- `crates/pii-server/tests/models_from_config.rs`
- `crates/pii-server/tests/mqtt_e2e.rs`
- `crates/pii-server/tests/operator_surfaces.rs`
- `crates/pii-server/tests/orphans_are_reaped.rs`
- `crates/pii-server/tests/poll_backoff.rs`
- `crates/pii-server/tests/poll_backoff_outage.rs`
- `crates/pii-server/tests/post_e2e.rs`
- `crates/pii-server/tests/queue_requeue_after_restart.rs`
- `crates/pii-server/tests/reattach.rs`
- `crates/pii-server/tests/run_get_names_the_failure.rs`
- `crates/pii-server/tests/run_get_states.rs`
- `crates/pii-server/tests/scheduler_does_not_spin.rs`
- `crates/pii-server/tests/scheduler_keeps_the_timer.rs`
- `crates/pii-server/tests/session.rs`
- `crates/pii-server/tests/silent_paths_speak.rs`
- `crates/pii-server/tests/skill_edit_through_the_daemons_store.rs`
- `crates/pii-server/tests/skill_put_through_the_daemon.rs`
- `crates/pii-server/tests/slice_f_promotion.rs`
- `crates/pii-server/tests/sop_engine.rs`
- `crates/pii-server/tests/spawn_node.rs`
- `crates/pii-server/tests/spec_graph_daemon/mod.rs`
- `crates/pii-server/tests/start_refuses_missing_needs.rs`
- `crates/pii-server/tests/status_latency.rs`
- `crates/pii-server/tests/tiering_e2e.rs`
- `crates/pii-server/tests/timer_no_refire.rs`
- `crates/pii-server/tests/unconfirmed_effect_refuses.rs`
- `crates/pii-server/tests/unresolved_graph_names_the_directories.rs`
- `crates/pii-server/tests/wasm_e2e.rs`
- `crates/pii-server/tests/woven_graph_can_start.rs`
- `crates/pii-server/tests/woven_pin_drift_refuses.rs`
- `graphs/DOCS/coder-demo.md`
- `graphs/DOCS/coder-podman.md`
- `graphs/DOCS/coder-selffix.md`
- `graphs/DOCS/intervention-report.md`
- `graphs/DOCS/queue.md`
- `graphs/README.md`
- `graphs/coder-demo.toml`
- `graphs/coder-podman.toml`
- `graphs/coder-selffix.toml`
- `graphs/intervention-report.toml`
- `graphs/queue.toml`
- `scripts/autonomy/plan.tsv`
