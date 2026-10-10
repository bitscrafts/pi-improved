# 054 — Spec 108 (P3): coverage measured, ratchet enforced, adequacy reviewed

## R1 coverage (first ever measurement)
Tool: `cargo-llvm-cov 0.9.1` (installed with `cargo install cargo-llvm-cov --locked`; needs `rustup component add llvm-tools`).
Command: `cargo llvm-cov --workspace --lib --bins --tests --json --summary-only --output-path <f>`.
Scopes counted: unit + integration tests (incl. the spawned `pii` binary of `CARGO_BIN_EXE_pii` tests, which cargo-llvm-cov instruments). NOT counted: doctests (need nightly); `pii-worker`'s supervisor tests spawn the plain `target/debug/pii` (uninstrumented). `tests/` files themselves are excluded by the tool's default.

| crate | lines covered / total | % |
|---|---|---|
| pii-abi | 472/480 | 98.3 |
| pii-channels | 266/305 | 87.2 |
| pii-cli | 1234/2486 | 49.6 |
| pii-core | 1220/1375 | 88.7 |
| pii-evolve | 160/202 | 79.2 |
| pii-graph | 2681/3078 | 87.1 |
| pii-http-fetch | 203/220 | 92.3 |
| pii-mcp | 196/297 | 66.0 |
| pii-memory | 616/690 | 89.3 |
| pii-model | 878/960 | 91.5 |
| pii-net | 1270/1516 | 83.8 |
| pii-observe | 960/1046 | 91.8 |
| pii-proto | 392/453 | 86.5 |
| pii-server | 4219/5489 | 76.9 |
| pii-store | 2414/2758 | 87.5 |
| pii-tools | 1310/1476 | 88.8 |
| pii-tui | 220/433 | 50.8 |
| pii-uds | 38/66 | 57.6 |
| pii-web | 259/400 | 64.8 |
| pii-worker | 467/564 | 82.8 |
| xtask | 2266/3935 | 57.6 |
| **workspace** | **21741/28229** | **77.02** |

Weakest: pii-cli (49.6), pii-tui (50.8), xtask (57.6), pii-uds (57.6), pii-web (64.8), pii-mcp (66.0). Coverage is not correctness: a line executed by a weak assertion counts as covered.

## R2 ratchet
`coverage-floor.txt` = 74.0 (3 points under the measured 77.02), enforced by a `quality-gate.sh` step (`PASS: coverage-floor` / `FAIL: coverage-floor`) via `cargo xtask coverage-floor`. Missing `cargo-llvm-cov` is a FAIL, not a skip. Mutation: floor 90 (or `--floor 78`) → `error: coverage total 77.02% is below the floor ...`, exit 1.

## R3 adequacy table (16 non-exempt crates; the spec said 15)
"Wired" = reaches the daemon log in the real start-up path, not only in the crate's unit test. Spec 100/101 left most seams unattached in production; that was the main gap.

| crate | failure mode | revealed by | emitted? | evidence |
|---|---|---|---|---|
| core | capability denied; persist failure | `core.cap.denied` (+audit.log), `store.persist.error` | yes (spec 107) | pii-server `tests/session.rs`, 107 canary |
| graph | node dead / unsatisfiable join / batch width | run journal + `run.outcome` (trajectory), `pii graph trace`; the three metrics fire only through `Ctx`, and the per-run observer chain carries no `ContractObserver` | journal yes; metrics NOT in daemon log — OPEN, deliberate: per-tick Debug metrics would flood the log; needs a level decision | pii-graph `tests/observe_contract.rs` |
| store | durable write fails | `store.persist.error` | **gap closed**: `Store` observer was never attached; now via `SwitchObserver` | `log_wiring::a_store_write_failure_reaches_the_log` |
| net | egress denied / dial / accept fails | `net.egress.denied`, `net.dial.failed`, `net.accept.failed` | **gap closed**: daemon + ollama `NetService` had no observer | `log_wiring::a_denied_egress_reaches_the_log` |
| tools | tool error / bash deny-list hit | `tools.call`, `tools.deny_list.hit` | **gap closed**: no production `ToolSet` had an observer; now `ObsEvent::ToolCall` through the run ctx + `ToolCallObserver` in the run chain | `engine::tool` tests, `log_wiring::a_deny_list_hit_in_a_run_reaches_the_log` |
| model | provider error / latency / which model | `model.provider.error`, `.in_force`, `.request.latency` | **gap closed**: `ObservedProvider` was never constructed; `Engine::new` wraps the provider | `log_wiring::a_model_request_reaches_the_log` |
| worker | worker start/crash/restart | `worker.state.changed` | **gap closed**: daemon supervisors had no observer | `log_wiring::a_worker_state_change_reaches_the_log`, `pii-cli/tests/daemon_log_wiring.rs` (real daemon process) |
| channels | poll/send fails, cursor lag | `channel.poll.failed`, `.send.failed`, `.cursor.lag` | **gap closed**: `ObservedChannel` was never used; `build_channel` wraps | `log_wiring::a_failing_channel_poll_reaches_the_log` |
| memory | search / consolidate fails | `memory.search`, `memory.consolidate` | partly: `memory.search` RPC and the `memory` consolidate node **wired now**; the agent-turn memory injection (`agent/turn.rs`) still builds an un-observed searcher and swallows its error (`if let Ok`) — OPEN (needs `AgentFactory` to carry the log; 9 literal sites) | `log_wiring::a_memory_search_rpc_reaches_the_log`, `nodes::memory::tests` |
| cli | startup/daemon-connect failure; daemon lifecycle, peer refused, sandbox in force | `cli.failure`, `daemon.*` | yes (spec 105/107) | existing |
| server | engine failures | via store/model/net/worker/channel/tools above; run failures in the journal | yes (above) | `tests/log_wiring.rs` |
| tui | attach/render failure, reattach gap | `tui.failure`, `tui.reattach.gap` | **gap closed**: `cmd_tui` used the null observer; events are now held and written to stderr after the terminal is restored | `pii-cli/tests/tui_failure_logged.rs` |
| web | request / upstream failure | `web.request.failed`, `web.upstream.error` | yes (`StderrObserver` in `main`) | existing unit tests |
| mcp | tool call fails | `mcp.tool_call.failed` | yes (`StderrObserver` in `main`) | `pii-mcp` unit test |
| evolve | cycle start/finish/launch fail | `evolve.cycle.*`, `evolve.launch.failed` | yes (`run_cycle_observed(&StderrObserver)`) | `tests/observe_contract.rs` |
| http-fetch | fetch denied/failed | `fetch.outcome`, `fetch.allowlist.denied` | seam proven, but **no crate depends on `pii-http-fetch`**: the tool is not registered in any shipped `ToolSet`, so nothing can fire. Nothing to wire until the tool ships — FINDING | `tests/observe_contract.rs` |

Other findings (not fixed): `names::HEALTH_STATE_CHANGED` and `names::OBSERVE_EVENTS_DROPPED` are registered but emitted nowhere; `pii_proto::client::Client::connect` hangs forever when the peer closes the socket without answering the hello (the reader task ends without failing pending calls) — found while writing the TUI test; `pii tui` loopback still defaults to sandbox `off`.

## R4 residuals
1. `pii -p` default sandbox is now `workspace` (was `off`). Breaks: tools that write outside the workspace (and trajectory dirs not granted) are denied; on a kernel without Landlock the run now exits 4 instead of running unconfined; `--sandbox off` restores the old behaviour. `PII_TRAJECTORY_DIR` is granted read-write. Connecting to a daemon socket outside the workspace was verified to still work confined. Tests: `pii-cli/tests/oneshot_default_sandbox.rs` (mutations: default back to `Off` → RED; dropping the trajectory grant → RED).
2. Bash deny-list hit: `observe_call` now also reports `ObsEvent::ToolCall` (with `deny_entry`) on the run's observer; `ContractObserver` maps it to `tools.call` + `tools.deny_list.hit`. Before, only `core.cap.denied` appeared.

## R5 mutations (each RED then restored)
- `ctx` ToolCall emit removed → 3 `engine::tool` tests RED; `ToolCallObserver` dropped from run chain → `a_deny_list_hit_in_a_run…` RED.
- `net.set_observer`, `sup.set_observer`, channel wrap, `store.set_observer`, provider wrap each removed → exactly the matching `log_wiring` test RED; `attach_log` removed from `cmd_daemon` → `daemon_log_wiring` RED; `.with_observer` removed in `memory_search` → RED; consolidate unwrapped → `consolidate_node_reports_to_the_log` RED.
- `run_tui_observed` swapped back to `run_tui` → `tui_failure_logged` RED.
- Sandbox default `Off` → `default_run_is_refused…` RED.
- `SwitchObserver`: no mutation needed beyond the unit test (drops-before-attach asserted).
- Floor: raise above measured → FAIL.

New `names.rs` consts: none.
