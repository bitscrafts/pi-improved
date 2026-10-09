# Spec 95 R2 — inline RPC handlers: worker occupancy (measured)

Harness: `crates/pii-server/src/engine/inline_measure_tests.rs`. Real `dispatch()` on a real Engine
(real redb store, scripted-model runs, a real `pii worker-exec` worker behind `sys.reload`); nothing stubbed.
Inline path = handler called synchronously from the connection task, so one `dispatch` wall time IS the
time the async worker is held. 15 samples per method (median / max). Bar = 50 ms (spec 80).
The control (`heartbeat_control_light_method_never_late`) stayed green throughout.

| handler | median | max | verdict |
|---|---|---|---|
| session.create | 9.8 ms | 12.9 ms | stays inline (≪50 ms; one redb commit) |
| run.cancel | 0.07 ms | 0.3 ms | stays inline |
| run.start | 20.4 ms | 43.1 ms | stays inline (returns before execution; max within 7 ms of bar — watch) |
| run.get | 0.07 ms | 0.3 ms | stays inline |
| run.list | 0.9–1.3 ms (15 runs) | 2.1 ms | stays inline at normal size; scale probe: 28.9 ms median / 59.4 ms max over ~1000 stored runs (headroom ~1.7× median; not moved, median under bar) |
| approve.resolve | 6.8 ms | 8.3 ms | stays inline |
| sys.reload | 9.3 ms | 9.3 ms (1 sample, 1 real worker) | stays inline for 1 healthy worker; NOTE it calls the same `Supervisor::reload` as `worker.reload` (moved by spec 80) — cost scales with worker count and a worker slow to drain (≤5 s) or stop (≤1 s) would hold the worker. Not driven (no slow real worker); not estimated. |
| run.resume | 0.9 ms | 1.7 ms | stays inline (resume of a finished run) |
| session.close | 5.0 ms | 5.1 ms | stays inline |
| graph.apply | 4.9 ms | 11.5 ms | stays inline |
| event.subscribe (replay) | 2.2 ms (≈8-event journal) ; **102 ms median / 132 ms max for a 5000-event journal** | | **MOVED** to `spawn_blocking` (`is_heavy`): replay (`read_journal`) cost scales with journal length and crosses 50 ms. Heartbeat test `heartbeat_event_subscribe`: RED inline (tick late 299 ms), GREEN after move; own control `heartbeat_control_light_method_for_event_subscribe`. |

Not driven: none of the 11 was undriven. Limits: samples are single-machine; run.list 1000-run probe and the
5000-event replay probe are `#[ignore]`d test `inline_handlers_scale_probe` (`--ignored --nocapture`).
Moved: 1 of 11 (event.subscribe). The other 10 are "measured, fine inline".
