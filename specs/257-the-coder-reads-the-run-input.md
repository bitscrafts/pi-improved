# Spec 257: the coder node reads spec, agent, model and brief from the run input — one tracked graph implements any spec

## Overview
**Project**: /projects/pi-improved
**Phase**: production — generic spec graph, slice 1 (operator, 2026-10-24: "graphs should be reused"; Opus design review `/projects/_tmp/pii-graph-design/pii-graph-design-plan-20261024T155725Z.md`, Verdict 7 and Q10)

Today `scripts/run-batchC-pii.sh` WRITES a new one-node graph per dispatch (`_tmp/devbox-test/graphs/q<N>-<ts>.toml`, gitignored) because the coder node cannot take its variable
parts from the run. The node reads `agent`, `model`, `spec`, `workdir`, `brief` from its own config, or `spec` from `nodes.select.item.spec`. `pii run start <graph> --input <json>` already
carries input and the expression language already resolves `$.input.<path>` (`crates/pii-graph/src/expr.rs`), but the coder never evaluates its config against it.

This spec makes the coder resolve designated config keys from the run input, ships ONE tracked graph (`graphs/spec.toml`) that uses it, and makes a malformed `--input` an error. Nothing
is generated per run after this. (The judge, red/green, classification, the fix loop and the `call` documentation sub-graph are separate specs; this is the smallest slice that lets one real
spec run through one tracked graph.)

## Verified Premises
- **CONFIRMED** `crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs:27-30`: `run` starts with `let cfg = with_selected_spec(&self.cfg, input)` and reads every key from that value; `CoderFactory::build` (`coder.rs:389`) stores `cfg` unchanged and validates nothing, so a `"$.input.agent"` string survives to `run`.
- **CONFIRMED** `pii_graph::expr::eval_or_literal(ctx, value)` passes a non-`$.` value through and evaluates a `$.` string against `{"input","payload","nodes"}`; an unresolvable path is `Null`, not an error (`expr.rs:39,66`). `hal.rs:100-112` is the in-repo example of a node that evaluates its config this way.
- **CONFIRMED** a string `timeout_ms` fails V10: `validate.rs:105` reads it with `as_u64()`, which is `None` for a string, so `timeout_ms` cannot be an expression and stays a literal in the graph. This is wanted: the time bound must be visible in the tracked file.
- **CONFIRMED** `crates/pii-cli/src/cmd_sys.rs:264-269`: `run_start` does `serde_json::from_str(&raw).unwrap_or(input)`, so malformed `--input` silently becomes `{}`. For a graph whose only variability is its input that is a silent wrong run.
- **CONFIRMED** `WorkerSpec::from_cfg` (`agent_worker.rs:272`) reads `agent, model, provider(opt), spec, workdir, sid, session(opt), run, max_budget_usd`.
- **MEASURED 2026-10-24** (`/projects/_tmp/measure-257-premise.sh`): no code outside the coder node reads `agent`, `model`, `provider`, `spec`, `workdir` or `brief` from a node's raw config. The graph-wide readers of a coder node are `validate.rs:100` (V10, `timeout_ms` only), `validate.rs:219` (V11, the `err` edge) and `agent_calls.rs:177` (`expect_spawn`); none touches the keys R1 resolves. Inside the coder, every reader (`WorkerSpec::from_cfg`, `graph_node.rs:33-35`, `coder.rs:226` `resolve_criteria` reading `workdir`) takes the `cfg` that `run()` passes, so resolving once at the top of `run()` reaches all of them. If a reader of the RAW `self.cfg` is found that this measurement missed, make it read the effective cfg; do not resolve in `build`.

## Requirements
### Functional
- **R1. Which keys.** In `CoderNode::run`, after `with_selected_spec`, resolve these top-level config keys when (and only when) the value is a string starting with `$.`: `agent`, `model`, `provider`, `spec`, `workdir`, `brief`, `session`, `sid`, `max_budget_usd`. Resolve with `pii_graph::expr::eval_or_literal` against `{"input": input.input, "payload": input.payload, "nodes": input.nodes}`. Every other key (`env_pass`, `criteria`, `timeout_ms`, `container_image`, `container_runtime`, and anything else) is NEVER evaluated: the run input chooses what to work on, not which environment names pass, which image runs, or how long it may take. A literal value anywhere is untouched.
- **R2. Required keys.** If `agent`, `model`, `spec`, `workdir` or `brief` resolves to `Null`, a non-string, or an empty string, the node refuses BEFORE anything spawns, through the existing `self.refuse(...)` path, with the reason `input key '<key>' ($.input.<path>) resolved to nothing` naming the key and its expression. The agent is not launched.
- **R3. Optional keys.** If `provider`, `session`, `sid` or `max_budget_usd` resolves to `Null`, the key is removed from the effective config (as if absent). A non-null value is used as if literal.
- **R4. No input is the old behaviour.** A config with no `$.` strings behaves exactly as before: all existing coder tests pass unedited.
- **R5. Malformed `--input`.** `pii run start <graph> --input <not json>` prints `error: --input must be valid JSON` to stderr and exits 2, without contacting the daemon. `--input` with no value exits 2 (`error: --input needs a value`). Valid JSON, and no `--input` at all (still `{}`), are unchanged.
- **R6. The tracked graph.** New file `graphs/spec.toml`: `name = "spec"`, `entry = "code"`, `caps = ["spawn", "proc"]`; a `code` node of kind `coder` with `agent = "$.input.agent"`, `model = "$.input.model"`, `provider = "$.input.provider"`, `spec = "$.input.spec"`, `workdir = "$.input.workdir"`, `brief = "$.input.brief"`, `max_budget_usd = "$.input.max_budget_usd"`, `env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR"]`, `timeout_ms = 3000000`, and NO `criteria`; a `done` transform; a `refused` node of kind `refuse` (`hook = "coder"`, `reason = "the agent failed"`); edges `code --ok--> done` and `code --err--> refused`. The file contains no absolute path (no `/` at the start of any string value) and no credential value.

### Non-Functional
- No new dependency. Files stay under 450 effective lines: put the resolution in a new `crates/pii-server/src/node_kinds/nodes/coder/input_cfg.rs` and call it from `graph_node.rs`; do not grow `graph_node.rs` by more than 10 lines.
- Observability: the effective values are what the existing `coder.spawned` / `coder.exited` events already journal. Add no event.
- No hardcoded path; no key in any file.

## Architecture
```
pii run start spec --input '{"agent":"claude","model":"claude-haiku-5-5","spec":"specs/N-x.md","workdir":"...","brief":"..."}'
        |                                                    (malformed JSON -> exit 2 here, R5)
        v
graphs/spec.toml : code(coder){ agent="$.input.agent", ... , env_pass/timeout_ms literal } --ok--> done   --err--> refused(refuse)
        |
        v   CoderNode::run:  cfg = with_selected_spec(cfg, input)  ->  cfg = input_cfg::resolve(cfg, input)?   (R1-R3; refuses per R2)
        v   the rest of run() is unchanged and reads the effective cfg
```

## TDD Contract
New file `crates/pii-server/tests/coder_reads_the_run_input.rs`, reusing the stand-in-agent pattern of `crates/pii-server/tests/agent_claude_session_handle.rs` (a script that records its argv and writes a marker file). New file `crates/pii-cli/tests/run_start_refuses_bad_input.rs`. New file `crates/pii-graph/tests/spec_graph_is_tracked_and_valid.rs`.

| Test name | Setup | Expected |
|---|---|---|
| `agent_model_spec_workdir_and_brief_come_from_the_input` | cfg with `$.input.*` for the five, input supplies them, fake claude records argv | the argv carries the input's model, the brief text, and the session file under the input's workdir |
| `a_missing_required_key_refuses_and_names_it` | input lacks `agent` | node refuses; reason contains `agent` and `$.input.agent`; the fake agent's marker file does NOT exist |
| `an_empty_string_required_key_refuses` | `model = ""` in the input | refuses naming `model` |
| `a_null_optional_key_is_absent` | `provider` null, agent pi | the pi argv contains no `--provider` |
| `a_literal_config_is_untouched` | the config of an existing coder test | identical journal facts to the same run before this change (CONTROL: composition of nothing) |
| `env_pass_is_never_resolved` | `env_pass = ["$.input.names"]` with input naming `OLLAMA_API_KEY` | the fake agent does NOT see `OLLAMA_API_KEY` (the literal string is not an env name) |
| `the_input_cannot_pick_the_image` | `container_image` literal, input also supplies `container_image` | the literal image is the one used |
| `a_malformed_input_exits_2_without_a_daemon` | `pii run start g --input '{bad'` with no daemon socket | exit code 2, stderr contains `--input must be valid JSON` |
| `a_missing_input_value_exits_2` | `pii run start g --input` | exit code 2 |
| `valid_input_and_no_input_still_reach_the_daemon_path` | `--input '{}'` and no flag | the usual "no daemon" error (exit 1), NOT exit 2 (CONTROL for the two above) |
| `spec_graph_loads_validates_and_has_no_absolute_path` | load `graphs/spec.toml` with the server registry | validates with no errors; no string value starts with `/` |
| `a_graph_with_an_absolute_path_is_caught_by_the_same_check` | a copy with `workdir = "/x"` | the path check fails (CONTROL: the check can fail) |
| `spec_graph_timeout_is_a_literal` | the graph's `timeout_ms` | is a positive integer, not a string |

**The requirement a plausible implementation satisfies only in appearance:** resolving the expressions in the graph file at load but handing the node the unresolved config at run, or resolving every key including `env_pass`. The first passes a load test and spawns an agent named `"$.input.agent"` (which `Agent::parse` refuses, so it looks safe); the second lets run input widen what the agent can see. `agent_model_spec_workdir_and_brief_come_from_the_input` must RUN the node and read the recorded argv; `env_pass_is_never_resolved` is its control.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_reads_the_run_input 2>&1 | grep -qE "^test result: ok\. ([6-9]|[1-9][0-9]) passed"`
- [ ] `cargo test -q -p pii-cli --test run_start_refuses_bad_input 2>&1 | grep -qE "^test result: ok\. [3-9] passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_is_tracked_and_valid 2>&1 | grep -qE "^test result: ok\. [3-9] passed"`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --test agent_claude_session_handle 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --test coder_spawns_a_process 2>&1 | grep -qE "^test result: ok"`
- [ ] `test -f graphs/spec.toml && test -f crates/pii-server/src/node_kinds/nodes/coder/input_cfg.rs`
- [ ] `cargo clippy -q -p pii-server -p pii-cli -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT evaluate `env_pass`, `criteria`, `timeout_ms`, `container_image` or `container_runtime`, and do NOT make `timeout_ms` accept an expression (V10 requires a literal).
- Do NOT edit any existing test; a failing existing test is a finding to report.
- Do NOT change a verdict, route, exit code or existing `coder.*` event field.
- Do NOT put an absolute path or a credential in `graphs/spec.toml`.
- Do NOT touch `scripts/`. The lane script keeps generating graphs until a later spec moves it onto `graphs/spec.toml`.
- Do NOT commit.

## Follow-ups (not this spec)
`graphs/spec.toml` gains the judge (criteria read from the spec blob at a pinned base sha), the classified failure ports, red/green, the bounded fix loop, `qgate`, and the `call` node for the documentation sub-graph; `run-batchC-pii.sh` becomes `pii run start spec --input`.
