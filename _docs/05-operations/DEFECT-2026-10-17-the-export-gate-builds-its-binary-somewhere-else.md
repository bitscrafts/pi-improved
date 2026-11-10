# DEFECT 2026-10-17 — the export's gate builds `pii` somewhere else, and its tests look for it in the tree

**Found by** the two parked rows (A22/155, A27/160) refusing to close: both were parked as `gate-failed`, and the
gate's own log named three `pii-worker` tests failing with `spawn …/target/debug/pii: No such file or directory`
— a path that exists in this tree. The loop was **unable to close any gated item** while this held, because no
`fresh-gate` run made by the loop could ever be green.

## The measured fact

From the gate run the loop made on 2026-10-16 22:53 (export kept at `<scratch>/fresh-gate-624085`):

```
----- control::tests::a_reload_reports_what_it_drained stdout ----
panicked at crates/pii-worker/src/control.rs:645:25:
called `Result::unwrap()` on an `Err` value: Internal(
  "spawn <scratch>/fresh-gate-624085/target/debug/pii: No such file or directory (os error 2)")
…
test result: FAILED. 5 passed; 3 failed  →  failing: control::tests::{a_reload_reports_what_it_drained,
                                          invoke_overtaken_by_a_reload_is_not_dropped,
                                          stale_reader_eof_after_reload_does_not_kill_the_new_worker}
error: test failed, to rerun pass `-p pii-worker --lib`   → the step aborts, so later targets never ran
```

and, in the same export, on disk:

```
$ ls <scratch>/fresh-gate-624085/target/
clippy-gate  doctest-gate  gate-steps  hal-gate  llamacpp-gate      # no `debug/`, so no `pii`
$ head -1 <scratch>/fresh-gate-624085/target/gate-steps/build.log
Compiling pii-cli v0.1.0 (<scratch>/fresh-gate-624085/crates/pii-cli)
   Finished `dev` profile … in 2m 06s
$ ls -l _tmp/gate-target/debug/pii
-rwxr-xr-x … 348139680 Oct 16 23:30 …/gate-target/debug/pii         # the build went HERE
```

The `build` step passed. The binary it produced was never in the tree, because the gate ran with
`CARGO_TARGET_DIR` pointing at the loop's compiler cache:

- `scripts/autonomy/env.sh` exports `FRESH_GATE_TARGET=_tmp/gate-target` (a comment there
  records the 2026-10-15 intent: the export's **tree** must be fresh, its **compiler output** need not be);
- `scripts/autonomy/cycle.sh:27` sources `env.sh`, and its gate line (L240) runs
  `cargo run -q -p xtask -- fresh-gate --worktree`;
- `xtask/src/fresh_gate.rs:199-206` reads `FRESH_GATE_TARGET` and sets `CARGO_TARGET_DIR` for the exported
  gate to it.

So the export's gate builds into the cache. Three test helpers, however, resolved the `pii` binary as
`env!("CARGO_MANIFEST_DIR")/../../target/debug/pii` — the **tree's** `target/`, which in that configuration is
empty. Cargo aborts the `test (lib/bins/tests)` step at the first failing target, so only the three unit tests
were reported; the ten tests in `crates/pii-worker/tests/supervisor.rs` were never reached.

## The class, measured, not assumed

The tree path was made absent (`mv target/debug/pii …`, restored by the script's `EXIT` trap) and cargo was
pointed at another target dir — the export's configuration, in one command
(`_tmp/autonomy/proof-red.sh`, log `_tmp/autonomy/logs/proof-red-20261017.log`):

```
===== pii-worker --lib control::
called `Result::unwrap()` on an `Err` value: Internal("spawn target/debug/pii: No such file or directory (os error 2)")
test result: FAILED. 2 passed; 3 failed
===== pii-worker --test supervisor
test result: FAILED. 0 passed; 10 failed
```

Thirteen tests, two files, one expression. The same expression exists once more outside `pii-worker`
(`crates/pii-server/src/engine/inline_measure_tests.rs:107`), behind the step that never got to run.

## The fix

Resolve the binary from the running test executable, which is always in the target dir the caller built into:
`current_exe()` → pop the test binary → pop `deps` → `dir.join("pii")`. Three helpers, one per file, each
carrying the reason (`crates/pii-worker/src/control.rs`, `crates/pii-worker/tests/supervisor.rs` — four call
sites including the two wrapper scripts written as text — and
`crates/pii-server/src/engine/inline_measure_tests.rs`). No assertion was weakened, no timeout raised, no test
skipped; the spawned path is the one the caller's build produced, which is also what `proc_argv0` now permits.

## The green, in the same configuration

Tree path absent, binary present only in the compiler cache — `_tmp/autonomy/proof-green.sh`, log
`_tmp/autonomy/logs/proof-green-20261017.log`:

```
== the tree path is ABSENT: ls: cannot access 'target/debug/pii': No such file or directory
== the binary the tests must find: … _tmp/proof-target/debug/pii
pii-worker --lib control::    test result: ok. 5 passed; 0 failed
pii-worker --test supervisor  test result: ok. 10 passed; 0 failed
```

The same three files run green **in this tree** as before (`bash quality-gate.sh` step `test (lib/bins/tests)`
and `test (doc)` are the loop's next acceptance).

## What this does not yet claim

- **A `fresh-gate` PASS on HEAD, run by the loop, recorded** — that is the acceptance this fix exists to make
  reachable, and it is the next cycle's run on A22/155. Until a verdict exists, the fix is proven at the level
  of the failing step, not of the whole gate.
- The third site (`pii-server`) is proven green in the same configuration but its **red** was not measured
  separately: it sits behind the same step, with the same expression, and its file is the one the abort hid.
- `xtask/src/rss_gate.rs:67,95` and `xtask/src/promote_build.rs:110` carry the same shape for worktrees they
  create themselves; they are not gate steps, and are left alone rather than changed blind.

## Why this is not a weakened check

The gate still fails any tree whose `pii` binary the caller did not build. What changed is which **target
directory** the tests consult: the one cargo was told to use, instead of one that only exists when nobody
redirects it. The old form passed on an idle developer machine and failed in the CI-equivalent export — the
exact inversion the project's own rule warns about (believe `fresh-gate`, not the local gate).
