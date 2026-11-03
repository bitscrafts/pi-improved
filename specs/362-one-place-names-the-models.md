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

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test models_from_config 2>&1 | grep -qE 'test result: ok. 3 passed'`
- [ ] `grep -q '@workhorse' graphs/queue.toml`
- [ ] `! grep -rq 'deepseek-v4-flash' graphs/`
- [ ] `grep -q 'workhorse' crates/pii-server/src/daemon/config.rs`
- [ ] `cargo xtask fresh-gate`

## Guardrails

- Do NOT remove the ability to name a model explicitly; several graphs pass one from the run input.
- Do NOT put a credential in the config: the key stays an env reference, as today.
- Do NOT default an unknown role (R3).
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-server/src/daemon/config.rs`
- `crates/pii-server/src/node_kinds/nodes/agent_worker.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-cli/src/cmd_config.rs`
- `graphs/queue.toml`
- `graphs/coder-demo.toml`
- `graphs/coder-podman.toml`
- `graphs/intervention-report.toml`
- `graphs/coder-selffix.toml`
- `crates/pii-server/tests/models_from_config.rs`
