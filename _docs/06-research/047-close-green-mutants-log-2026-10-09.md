# Spec 92 mutation log (each mutant applied alone, restored by cp from backup; tree verified byte-identical after)
| mutant | result | test + assertion |
|---|---|---|
| R2a graph body errors -> unlimited fallback | RED | `resume_fail_closed_on_graph_body_fault` (Err swallowed: spawn_resume returned Ok -> `expect_err` "internal: store: injected get_graph fault" panic); `resume_fail_closed_on_graph_body_unparsable` `expect_err("unparsable graph body must refuse the resume")` |
| R2b limits forced u64::MAX | RED | `resume_carries_recovered_finite_limits` `assert!(found, "resumed run must enforce the recovered max_tokens=0: ...")` (journal shows Success) |
| R3b recover_finished removed from Lagged arm | RED | `daemon::lag::tests::reactive_loop_lag_rearms_cron_chain_for_lost_run_finished` "lost RunFinished must be recovered by the loop's Lagged arm: {\"r-cron-old\": \"job\"}" |
| R5b-1 begin_write/db swap (lib.rs:96) | **GREEN** | none - failure needs a redb begin_write error; no test-only way to induce it |
| R5b-2 read/run_counter swap (:103) | **GREEN** | none - needs a redb table.get error |
| R5b-3 open/graph_body swap (:113) | RED | `open_table_failures_render_op_resource_and_detail` assert_eq left "store op graph_body on open: graph_body is of type Table<u8, u8>" right "store op open on graph_body: ..." |
| R5b-4 commit/events swap (:357) | **GREEN** | none - needs a redb commit error |
| R5a/prefix open/path swap (:91) | RED | `open_failure_renders_structured_store_op` assert starts_with("store op open on <dir>: ") |
Baseline (no mutant): all of the above tests pass (pii-server --lib 62 ok, pii-store 30 ok).
