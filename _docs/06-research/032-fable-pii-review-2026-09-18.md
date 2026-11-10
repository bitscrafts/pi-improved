# pii review: findings and candidate improvements (2026-09-18)

> **Status update 2026-10-09:** the findings below were ALL implemented as specs 62-69 (`883d768` … `55a8328`; ledger in HANDOFF.md, summary commit `ef4d782`). "Findings only / nothing committed" is historical; the current review is `043-opus-full-review-2026-10-06.md`.

Status: findings only. No spec, no source change, nothing committed.
Companion to the 031 ground-truth doc in this directory (crate map,
dependency graph, raw file sizes, deferred-item list). This doc does not
repeat that material and does not re-propose any of its deferred items.

## Scope and method

What was actually executed this session:

- The repo's own line gate was reproduced by hand: `xtask/src/lint_files.rs`
  was read in full and its counting rule applied (via `sed` and `grep`) to
  every file in the 031 list plus the next tier of large files.
- Call-site counts for `PiiError::Internal` were measured per file, and
  every downstream `match`/`matches!` on that variant was searched for.
- The gate scripts (`quality-gate.sh`, `xtask ci`, `scripts/archive/mvp-verify.sh`),
  the pidag config, and the spec exit criteria were cross-checked for what
  is enforced where.
- `git diff --stat 1fc2d00 HEAD -- crates xtask Cargo.toml Cargo.lock` is
  empty: the Rust tree is byte-identical to the last commit that HANDOFF
  records as gate-verified, and `target/debug` artifacts are from 01:49
  the same night.

What could NOT be executed: `cargo clippy`, `cargo test`, `cargo fmt`,
`cargo xtask`, and the prebuilt binaries under `target/debug`. Command
execution in this session was limited to read-only text tools; every
attempt to invoke cargo (directly, via PATH override, via a login shell,
via the quality-gate script, or from a delegated subprocess) was denied.
Items 3 and 5 of the brief that depended on a live run are therefore
reported as "not verified this run" and the evidence below is structural,
not behavioural. A fresh `./quality-gate.sh` run should precede any spec
drafted from this doc. Note also that the gate logs HANDOFF cites as
evidence (`_tmp/gate-spec*.log`) are no longer on disk; `find _tmp -name
'*.log'` returns nothing.

## Finding 1: the "13 files over 450 lines" list measures the wrong thing

The enforced policy is `xtask lint-files`, and it counts EFFECTIVE lines:
non-blank, non-comment, and excluding every `#[cfg(test)]` body (F15,
`lint_files.rs:15-53`). Fail threshold 450, warn threshold 380. The 031
list was measured with raw `wc -l`, which is not the policy metric.

Measured with the gate's own rule (cut at the first `#[cfg(test)]`, then
drop blank and `//` lines):

```
file                                              raw   effective
crates/pii-model/src/ollama.rs                    801      341
crates/pii-store/src/heal.rs                      681      268
crates/pii-store/src/lib.rs                       627      360
crates/pii-store/src/session.rs                   570      304
crates/pii-graph/src/sched/state.rs               532      210
crates/pii-server/src/engine/runs.rs              530      230
crates/pii-model/src/lib.rs                       521      157
crates/pii-graph/src/render.rs                    513      133
crates/pii-core/src/ctx.rs                        512      268
crates/pii-server/src/lib.rs                      511      329
crates/pii-server/src/node_kinds/nodes/heal.rs    490      275
crates/pii-graph/src/validate.rs                  481      335
crates/pii-server/src/daemon/config.rs            474      179
```

Not one of the thirteen is in the warn band, let alone over the cap. The
largest by the real ruler is `pii-store/src/lib.rs` at 360, ninety lines
under the cap. This is consistent with HANDOFF's spec-51 row ("lint-files
now reports zero warnings across the entire workspace"), which was true
then and is still true now by this measurement.

Consequence for the brief's item 1: no split is REQUIRED by policy for any
of the thirteen. The cross-cutting root cause asked for in item 2 is not a
code pattern at all; it is that twelve of the thirteen carry inline test
modules that make up 48 to 74 percent of their raw line count, and the
thirteenth (`pii-server/src/lib.rs`, no test module) is 36 percent blank
lines and doc comments. Splitting any of them "for the 450 rule" would be
work the rule does not ask for.

If splits are still wanted for readability rather than policy, the
ranking in the "if you split anyway" section below is by effective size
and seam quality, but it is deliberately placed after the real findings.

## Finding 2: the line gate is not part of the pidag quality gate

`quality-gate.sh` (the script `.pidag/config.toml` names as
`quality_gate_script`) runs fmt, clippy, build, test, and doctests. It
never runs `cargo xtask lint-files`. The line gate is enforced only by:

- `cargo xtask ci` (`xtask/src/main.rs:74`), a manual invocation;
- `scripts/archive/mvp-verify.sh:53`, a manual, live-daemon suite;
- per-spec exit criteria, where a spec author remembered to list it
  (54 of the 77 spec files mention it; the other 23 do not).

HANDOFF's own history shows the gap is real, not theoretical: the spec-40
row records a full green gate while `xtask lint-files` had a hard error on
`xtask/src/silent_noop.rs` (458 effective lines), which was only fixed
four specs later. A workhorse implementing a spec whose exit criteria omit
`lint-files` can push a file over 450 and the SDD loop will pass it.

A second divergence in the same area: `quality-gate.sh` carries the F-01
fix (clippy and doctests in their own `CARGO_TARGET_DIR`, `cargo build`
before test, lib/bins/tests split from doc). `xtask ci` (`main.rs:84-96`)
and `mvp-verify.sh:44-49` still run `clippy --all-targets` followed by
`cargo test --workspace` in the shared `target/debug`, which is exactly
the shape F-01 diagnosed as racing on schemars/serde_json rlib hashes
(E0463). Whoever next runs `cargo xtask ci` cold may hit the bug the gate
script already fixed. The fix is to make those two callers invoke
`quality-gate.sh` for the cargo steps, or to copy its isolation.

## Finding 3: the line counter mis-counts one attribute shape

`count_effective_lines` (`lint_files.rs:19-53`) sets `in_cfg_test` when it
sees `#[cfg(test)]`, then tracks brace depth from the NEXT line. If the
next line is another attribute with no braces, depth stays at zero, the
function exits test mode immediately, and the whole test module that
follows is counted as production code.

Concrete instance today: `crates/pii-abi/src/failover.rs:260-262` is
`#[cfg(test)]` then `#[rustfmt::skip]` then `mod tests {`. Measured:

```
failover.rs production part (lines 1-260):   196 effective
failover.rs test body (lines 263-432):        163 effective, wrongly counted
what lint-files reports:                      359
what it should report:                        196
```

At 359 it is 21 lines from a spurious warning and 91 from a spurious
FAIL, on a file whose real production size is under 200. The same quirk
skips one production line after any single-item `#[cfg(test)]` (two
instances: `xtask/src/memory_eval.rs:178`, `xtask/src/main.rs:374`),
which under-counts by one; harmless, but the same missing "attribute
stack" logic. The fix is a few lines (while in test mode, keep consuming
lines that start with `#[` before beginning brace tracking) plus two
tests that would fail on the current code: one with `#[rustfmt::skip]`
between the attribute and the module, one with a bare `#[cfg(test)] fn`.

## Finding 4: F-11 pass 2 has an obvious target and a hidden blocker

Per-file `PiiError::Internal(` counts, largest first (all non-abi):

```
64  crates/pii-store/src/lib.rs
58  crates/pii-store/src/session.rs
29  crates/pii-store/src/mem/read.rs
18  crates/pii-store/src/mem/write.rs
16  crates/pii-store/src/timer.rs
15  crates/pii-store/src/channel.rs
14  crates/pii-store/src/approval.rs
10  crates/pii-model/src/llamacpp.rs
10  crates/pii-store/src/mem/mod.rs
```

The natural next file is `crates/pii-store/src/session.rs`: 58 sites,
of which 56 are literally `.map_err(|e| PiiError::Internal(format!("store:
<op> <resource>: {e}")))` (the exact pre-pilot shape of `heal.rs`) and 2
are the fault-injection returns at lines 30 and 239. It is an `impl Store`
extension in the same crate as the pilot, so `op`/`resource` names fall
out of the existing strings mechanically. Safety checks done: no code
outside `pii-abi` pattern-matches `PiiError::Internal`, and no test
outside `session.rs` depends on the `"store: ..."` display text.
Classification is unchanged (`StoreOp` is neither retryable nor terminal,
same as `Internal`), so `report_persist_error` and the LRU trim paths in
`pii-server` behave identically.

The blocker: the pilot's literal `PiiError::StoreOp { op: String::from(..),
resource: String::from(..), detail: format!(..) }` is five lines per site
against one. Commit `c8fb4d9` converted 15 sites in `heal.rs` for a net
+82 lines. Applying the same literal to `session.rs`'s 58 sites adds
roughly 230 effective lines, taking it from 304 to about 535, which FAILS
the 450 gate. `pii-store/src/lib.rs` (64 sites) would go from 360 to
about 615. Pass 2 as a straight copy of the pilot therefore cannot land
without either splitting the file first (unnecessary, per finding 1) or
adding a one-line constructor. No such constructor exists today (`grep
"fn store_op" crates/pii-abi/src` finds only test names). A
`PiiError::store_op(op: &str, resource: &str, detail: impl Display)`
helper in `pii-abi/src/error.rs` makes every site a one-liner, keeps
`session.rs` near 304, and lets `heal.rs` shrink back by ~60 lines when
retrofitted. This should be the first step of pass 2, not an afterthought.

## Checked and found clean (no finding)

- `#[ignore]`: 10 tests, all real-model acceptance tests loading a 2.2 GB
  GGUF, all with reason strings. Nothing is silently skipped.
- `#[allow(clippy::too_many_lines)]`: 20 sites, each with a reason
  comment. Two sit in `pii-graph/src/validate.rs` (`validate` at ~176
  lines, `v6_unsatisfiable_join` at ~113); see the split note below.
- `PiiError` enum shape: not re-litigated, per the 031 doc.
- Sources are unchanged since the last gate-verified commit (see Scope).

## If you split anyway: ranking by effective size and seams

None is required. If the team wants to trim for readability, these three
have the best seams and the most effective code; everything else in the
031 list is under 280 effective lines and should be left alone.

1. `crates/pii-store/src/lib.rs` (360 effective). Largest by the real
   ruler and it mixes three concerns: `Store::open` plus counters and the
   test fault switch (lines 34-185), graph blobs (`put_graph`/`get_graph`/
   `all_graphs`, 215-285), and the run journal (`append_event(s)`,
   `get_run`, `read_journal`, snapshots, resume keys, `runs_by_status`,
   286-512). The same crate already uses the `impl Store` extension-file
   pattern (`session.rs`), so `store/{lib,graphs,runs}.rs` needs no new
   idea. Doing this AFTER an F-11 constructor exists avoids touching the
   64 error sites twice.
2. `crates/pii-model/src/ollama.rs` (341 effective, 801 raw). Three
   separate test modules (lines 420-801) are 48 percent of the file; the
   precedent for moving them to `ollama/tests.rs` with an inner
   `#![cfg(test)]` is `node_kinds/agent/tests.rs` (spec 43). Production
   code has one clean seam: streaming (`chat_streaming`, `drain_lines`,
   `OllamaStreamLine`, `build_stream_request`, lines 179-420) versus
   request/wire building (`build_request`, `message_value`, `tool_value`,
   `send`, `chat`).
3. `crates/pii-graph/src/validate.rs` (335 effective). Not the biggest,
   but the only one whose size shows up as TWO `too_many_lines` allows;
   `validate` is one 176-line function of V-rule checks and
   `v6_unsatisfiable_join` is a second 113-line one. Extracting the
   per-rule checks into `validate/rules.rs` and the join analysis into
   `validate/join.rs` removes both allows rather than working around them.

Why not the others: `heal.rs` (268) was just touched by the pilot and
will shrink further with the constructor; `session.rs` (304) is a single
coherent `impl Store` extension; `pii-server/src/lib.rs` (329) is
re-exports and daemon wiring with no internal seam; the remaining eight
are all under 280 effective.

## Candidate improvements (ranked)

1. **F-11 pass 2: `PiiError::store_op` constructor, then `session.rs`.**
   Add `pub fn store_op(op: &str, resource: &str, detail: impl
   std::fmt::Display) -> Self` to `crates/pii-abi/src/error.rs` with a
   display test; retrofit the 15 literal sites in `pii-store/src/heal.rs`
   to it; convert all 58 `PiiError::Internal` sites in
   `pii-store/src/session.rs` (56 `map_err` sites plus the 2 injection
   returns) and update that file's `# Errors` doc lines. Exit criteria a
   validator can check: `grep -c "PiiError::Internal(" session.rs` is 0,
   `grep -c "PiiError::StoreOp {" heal.rs` is 0, `grep -c
   "PiiError::store_op(" session.rs` is 58, `xtask lint-files` passes,
   effective lines of `session.rs` within 320, `cargo test -p pii-store
   -p pii-server` green. Follow-on passes (`lib.rs` 64, `mem/read.rs` 29)
   become mechanical once the constructor exists.
2. **Wire `xtask lint-files` into `quality-gate.sh` and de-duplicate the
   gate shapes.** Add a fourth step (`cargo run -q -p xtask -- lint-files`,
   seconds, no workspace build beyond xtask) so the pidag loop enforces
   the 450 rule on every spec regardless of exit-criteria authoring. In
   the same change make `xtask ci` and `scripts/archive/mvp-verify.sh` call
   `quality-gate.sh` for fmt/clippy/build/test instead of carrying the
   pre-F-01 shared-target-dir sequence. Exit criteria: `grep -c
   lint-files quality-gate.sh` is 1; `grep -c "cargo test" xtask/src/
   main.rs scripts/archive/mvp-verify.sh` is 0; gate green.
3. **Fix `count_effective_lines` attribute handling.** Consume attribute
   lines that follow `#[cfg(test)]` before starting brace tracking. Two
   new tests in `lint_files.rs` that fail today: `#[cfg(test)]
   #[rustfmt::skip] mod tests { <200 lines> }` counts 10 not 210, and
   `#[cfg(test)] fn helper() {}` followed by a production line counts that
   line. Exit criterion: `failover.rs` reports 196, not 359.
4. **Make the gate the single ruler for size reporting.** Add a
   `lint-files --report [N]` mode that prints the top-N files by
   effective lines with raw counts alongside, so future ground-truth docs
   and reviews measure with the enforced rule instead of `wc -l`. Then
   correct the 031 doc's list (docs-only). Small, but it is what would
   have prevented this review's brief from targeting thirteen files that
   pass the policy.
5. **Optional readability splits**, in the order given above
   (`pii-store/src/lib.rs`, `ollama.rs`, `validate.rs`), only after items
   1 and 2 land, and only if raw size is judged to hurt review. Not a
   policy requirement.

## Follow-up: live execution + full-crate pass (2026-09-18, run 2)

Completes the two gaps in the first pass: every cargo gate was actually
executed this time, and the fifteen crates the first pass did not read
were covered. Findings 1-4 above stand unchanged. Nothing in the source
tree was modified; the working tree is still byte-identical to `80a6ce4`
apart from this document. Toolchain: `cargo 1.95.0`, `rustc 1.95.0`,
matching `rust-toolchain.toml`. `cargo` is not on the default PATH in
this environment; `quality-gate.sh` exports `/usr/local/cargo/bin` and
every command below was run with that same export.

### (a) Gate output and what it proves

Run in the order and target-dir layout `quality-gate.sh` uses (clippy and
doctests isolated, build before test), plus a second clippy run in a
brand-new target dir so the result could not be a cached verdict.

```
command                                                    exit  notes
cargo fmt --all --check                                      0   empty diff
cargo clippy --workspace --all-targets -- -D warnings        0   cached (target/clippy-gate, artifacts 01:49 > sources 01:21)
  same, CARGO_TARGET_DIR=target/clippy-gate-run2 (fresh)     0   221 targets compiled from scratch, same output
cargo build --workspace                                      0   zero warnings
cargo test --workspace --lib --bins --tests                  0   63 binaries, 505 passed, 0 failed, 10 ignored
CARGO_TARGET_DIR=target/doctest-gate cargo test --doc        0   17 lib targets, 0 doctests exist
cargo run -q -p xtask -- lint-files                          0   prints nothing on success
cargo metadata --locked                                      0   Cargo.lock in sync (see (c))
```

Every `test result:` line was `ok. N passed; 0 failed`. The 10 ignored are
the known real-model tests (8 in `pii-model/tests/real_granite.rs`, 2 in
the `pii-model` lib), unchanged from the first pass. Per-binary counts for
the crates this run cares about: `pii` (the cli binary) 19, `pii_web` 9,
`pii_uds` 2, `pii_worker` 2 + `supervisor` 7, `pii_net` 10 + `egress` 12
+ `http_codec` 5 + `accept_pool` 3, `pii_tools` 25, `pii_mcp` 3 +
`mcp_eof` 1, `pii_memory` 4 + `consolidate` 3 + `lanes` 8 + `persistence`
1, `pii_tui` 5 + `render` 5, `pii_evolve` 0 unit + `launcher` 3 +
`state_tests` 2, `pii_http_fetch` 5 + `fetch` 6, `pii_channels` 7,
`pii_proto` 7 + `wire_roundtrip` 11, `pii_abi` 44, `pii_core` 23.

What this proves: the tree at `80a6ce4` is gate-green end to end, which
HANDOFF asserted and the first pass could not confirm. What it does not
prove: the gate is only as strong as its tests, and section (b) shows two
crates where green tests coexist with broken behaviour.

Three observations from the raw output, none of which fail the gate:

1. **`clippy.toml:22` carries a dead rule.** Clippy prints, once per
   compiled target (88 lines per run), `warning: 'println' does not refer
   to a reachable macro` for the `disallowed-macros` entry `{ path =
   "println", ... }`. The path has to be `std::println` to resolve. As
   written the rule never fires. It is harmless only because
   `[workspace.lints.clippy] print_stdout = "deny"` in the root
   `Cargo.toml` already covers the same ground, which is why the 42
   `println!` uses outside `pii-cli` all sit under an explicit
   `#[allow(clippy::print_stdout)]`. Fix is one line: change the path to
   `std::println` or delete the entry. The warning is the only non-blank
   line clippy emits today, so removing it makes a real regression
   visible instead of buried at line 89 of the same noise.
2. **The doctest step tests nothing.** All 17 lib targets report `0
   passed`. There are exactly two fenced blocks in doc comments in the
   whole workspace (both in `pii-model/src/llamacpp_tools.rs`, one bare
   and one `json`), so the step is a compile-only check. Not a defect,
   but the gate's fourth step costs a full isolated build of every crate
   for zero assertions. Worth knowing before anyone reads "doctests pass"
   as evidence of anything.
3. **`lint-files` is silent on success.** It prints nothing and exits 0.
   Fine for a gate, but it means a log cannot distinguish "ran and passed"
   from "did not run". A one-line summary (`N files checked, max effective
   M`) would also serve item 4 of the prior candidate list.

### (b) Findings from the previously-skipped crates

The `pii-cli` command files and `pii-web` were read in full, and the
remaining thirteen crates were read for their logic-bearing files. Every
finding below was re-checked against the source lines cited before being
written down.

**Finding 5: `pii-web` memory search is broken (wrong parameter key).**
`crates/pii-web/src/main.rs:144-148` (`api_memory`) forwards the search
term as `{"query": ...}`. The daemon handler
`crates/pii-server/src/daemon/methods.rs:216-219` (`memory_search`) reads
`params.get("q")` and returns `InvalidParams` with the text `memory.search
needs 'q'` when it is absent. The cli sends the right key
(`cmd_ctl.rs:134`, `{"q": q, "k": k}`). The web UI's memory page
(`index.html:122` calls `/api/memory?q=`) therefore always renders a
`BAD_GATEWAY` error body. The nine `pii-web` tests cover flag parsing and
the loopback default only; no test exercises any route handler, which is
exactly how this survived. One-token fix, plus a test that fails today.

**Finding 6: the `worktree` tool does not validate `signature` and
bypasses the FS capability gate.** `crates/pii-tools/src/worktree.rs:57-
72`: `sig` is taken verbatim from the tool-call JSON (schema requires only
`type: string`; the "blake3 hex" is a description, not a constraint) and
used as `dir.join(sig)`. `Path::join` replaces the whole base for an
absolute `sig` and appends `..` components literally for a relative one.
The result goes to `git worktree add -b heal/<sig> <path> HEAD` through
`/bin/sh -c` (line 82-91); `shell_quote` prevents shell injection but not
path escape. Unlike `fs.rs`/`search.rs`/`bash.rs`, this tool never goes
through `Ctx::fs()`/`FsCap` (the canonicalise-then-scope-check layer in
`pii-core/src/fscap.rs`), so the workspace scope that the 012 doc
describes as "real and enforced at the capability layer" is not applied
to the one tool that creates directories. Reachable from any agent node
that lists `worktree`, and from every MCP client, since `pii-mcp/src/
main.rs:40` registers `ToolSet::builtin` wholesale. Fix: reject anything
that is not 8-64 lowercase hex characters, then assert the joined path
starts with `dir` after the join; two tests (`../x`, `/tmp/x`) that fail
today. This is the only finding in this document with a security
dimension and it should go first.

**Finding 7: worker restart backoff is dead code, contrary to spec F3.5.**
`crates/pii-worker/src/control.rs:190-196` reads
`spec.restart_backoff_ms` and hands it to `respawn(sup, backoff)`;
`respawn` at line 279 names the parameter `_backoff_ms` and never sleeps.
`specs/05-phase3-assistant-gateway.md:109` (F3.5) says the child "is
respawned after `restart_backoff_ms`", and `spec.rs:37-38` defaults it to
50 ms. A worker that dies at startup is respawned back-to-back until
`max_restarts_in_window` trips. The cap still prevents an unbounded
storm, so severity is medium, but the knob operators are given is a
no-op. The existing test `supervisor.rs:133-141` asserts `took_ms <
2000`, which an immediate respawn also satisfies; a test asserting
`took_ms >= restart_backoff_ms` with a 200 ms spec would fail today.

**Finding 8: `same_layer_only` in memory consolidation is a no-op.**
`crates/pii-memory/src/consolidate.rs:159-161` is `fn layer_of(_store,
_id) -> u8 { 0 }`, so the guard at line 122 can never see two different
layers. Both production callers set the flag to `true`
(`pii-server/src/node_kinds/nodes/memory.rs:62` and
`daemon/methods.rs:293`), and the only test sets it `false`
(`pii-memory/tests/consolidate.rs:26`). Cross-layer merges are therefore
possible in production while the config says they are not. Either wire
`layer_of` to the store's real layer field or delete the flag; do not
leave a config knob that reads as a guarantee.

**Finding 9 (minor): the fetch audit line hardcodes `200-OK`.**
`crates/pii-http-fetch/src/lib.rs:95-97` writes `-> 200-OK
status={status}` for every non-error response, so a 404 is logged as
`200-OK status=404`. The real status is present, so nothing is lost, but
the log is documented as a replayable record and the literal is
misleading. Trivial fix.

**Finding 10 (minor): control-socket chmod failure is discarded.**
`crates/pii-cli/src/cmd_daemon.rs:249`: `let _ =
std::fs::set_permissions(&sock_path, 0o600)`. The comment at 238-240
calls this chmod part of the mode discipline that justifies the
`disallowed_methods` allow on the bind above it. Defence in depth holds
because the socket dir is 0700, so this is a logging gap, not a hole: on
failure, print to stderr and continue.

**`pii-cli` command files: true state.** There is no
`crates/pii-cli/tests/` directory at all. The 19 tests in the `pii`
binary are inline in `main.rs` (2), `ndjson.rs` (8), `cmd_trace.rs` (4)
and `cmd_oneshot.rs` (5); `cli_util.rs` has none despite HANDOFF's spec-58
note. The eight files the 031 doc flagged have zero `#[cfg(test)]`,
`#[test]` or `#[tokio::test]` between them (verified by grep on each):

- `cmd_worker_exec.rs` (136 raw): covered indirectly. It is the reference
  worker that `pii-worker/tests/supervisor.rs` spawns as a real
  subprocess (`worker_spec()` at line 46; used by the kill-9, restart,
  reload and stderr-flood tests). Not a gap.
- `cmd_ctl.rs` (226), `cmd_sys.rs` (92): thin argv-to-JSON glue over
  `pii_proto::client::Client::call`. Low risk untested, except
  `run_ls`'s client-side `--graph` filter in `cmd_sys.rs`, which is pure
  and cheap to test.
- `cmd_run.rs` (231): `Common::parse`, `root_ctx` (CapSet/Budget build,
  path canonicalisation) and `report()` (Terminal to exit code 0/1/4) are
  pure and untested. Real gap, cheap to close.
- `cmd_daemon.rs` (342): bootstrap ordering, exit-code selection
  (2/4/5/130), `self_uid()` parsing of `/proc`. Real logic, untested,
  and the hardest of the eight to unit-test. The exit-code mapping and
  `self_uid` are the testable slices.
- `cmd_soak.rs` (171): status matching on the literal strings
  `"succeeded"`/`"failed"` from `RunGet`. Brittle and untested; a rename
  on the daemon side would silently make every soak run report failure.
- `cmd_sandbox_exec.rs` (73): exit-code coercion `code().unwrap_or(1) as
  u32 & 0xff`; small, pure, untested.
- `cmd_tui.rs` (106): connect-mode resolution (`loopback`/`auto`/path)
  is filesystem-coupled; untested, low risk.

**`pii-web`: is 9 tests adequate?** No, but not because of the count.
The crate is an axum server with ten routes (`main.rs:249-260`), each
hard-wired to one read-only `Method` and forwarded to the daemon UDS at
`<runtime_dir>/control.sock`; no write methods are exposed, no CORS
layer is configured (the `tower-http` `cors` feature in `Cargo.toml` is
unused), and it refuses to start if the socket is absent. All nine tests
are `#[test]` on `parse_web_args` and two constants. Zero tests touch a
handler, the daemon call, or the error mapping, and Finding 5 is the
direct consequence. Adequate coverage here is not more flag tests; it is
one `#[tokio::test]` that stands up a fake UDS responder and drives each
route once, asserting the exact JSON the daemon receives. Also worth
recording: `--host 0.0.0.0` is accepted (`parse_web_args_host` tests it)
and the HTTP surface has no authentication of its own, so off-loopback
exposure hands any network client the daemon's full read surface under
pii-web's own uid, bypassing the peer-uid check. That is a documented
design choice, not a bug, but the README for the crate should say it.

**Other crates, one line each.** `pii-net` (30 tests): the
best-engineered crate read this run; DNS-pin, codec, TLS and accept pool
are all tested; the only lenience is that both `http_codec.rs:229-231`
and `service/dechunk.rs:54-56` consume a `\r` without confirming the
following `\n`. `pii-core` (23): `FsCap` canonicalise-then-check with a
symlink-escape test, `ProcCap` env scrubbing and process-group kill;
fine, and it is the layer Finding 6 bypasses. `pii-abi` (44): fine.
`pii-proto` (18): fine. `pii-uds` (2 tests, 120 LOC): client-side
connect and runtime-dir resolution only; the bind/peercred logic lives in
`pii-server`, so the thin count is proportionate. Its `HOME` fallback to
`/root` (`lib.rs:51`) is a real assumption that only holds because this
host runs as root. `pii-channels` (7): fine; `discord.rs:1-4` module doc
still says TLS is "cut to Phase 3b" while `DEFAULT_API_BASE` is https and
`pii-net` now does TLS (stale comment). `pii-tui` (10), `pii-evolve` (5),
`pii-tools` beyond Finding 6 (25, Landlock profile is fail-closed with a
self-probe): fine. `pii-mcp` (4): thin for a surface that re-exports
every builtin tool, including the one in Finding 6. No hardcoded
`/tmp` or `/projects/` paths remain in production code (spec-61 was the
last).

### (c) Security and dependency check

- **`Cargo.lock` is committed** (`git ls-files` lists it), last touched
  in `3865737` (2026-09-18 00:16), after the last `Cargo.toml` change
  (`eab311f`, 2026-09-17). `cargo metadata --locked` exits 0 and leaves
  the lockfile unmodified, so it is in sync. (`--offline` fails only
  because non-Linux transitive crates such as `android_system_properties`
  are not in the local registry cache; that is a cache state, not a lock
  problem.)
- **351 locked packages.** Current versions for the security-relevant
  ones: `tokio 1.53.1`, `hyper 1.11.1`, `axum 0.8.9`, `rustls 0.23.44`,
  `ring 0.17.14`, `serde_json 1.0.151`, `schemars 1.2.2`, `redb 4.2.0`,
  `landlock 0.4.7`, `llama-cpp-2 0.1.156`, `tower-http 0.6.11`,
  `crossterm 0.29.0`. Nothing stands out as stale, and no `git =`,
  `[patch]`, or out-of-tree `path =` dependencies exist.
- **One exact pin:** `ratatui = "=0.30.2"` in the workspace table,
  introduced in `09809b8` with no comment saying why. Either it should
  carry a reason comment (as the `CC = clang` override in
  `.cargo/config.toml` does) or be relaxed to `0.30`.
- `unsafe_code = "forbid"` is workspace-wide and clippy's fresh run
  confirms no `#[allow(unsafe_code)]` anywhere. `cargo audit`/`cargo deny`
  are not configured and were not run; that is the one thing a real
  dependency audit would add.

### (d) Candidate improvements, merged and re-ranked

Items from the first list keep their content; ranks change because run 2
surfaced behavioural bugs, which outrank structural work. Items 1-4 are
each one small spec with a test that fails today.

1. **Validate `signature` in the worktree tool and scope-check the joined
   path** (Finding 6). `crates/pii-tools/src/worktree.rs`: reject
   non-hex or wrong-length input with `PiiError::Invalid`; after the join,
   error if `wt.strip_prefix(&dir)` fails. Exit criteria: two new tests
   (`../escape`, `/abs/path`) fail before and pass after; the existing
   worktree tests still pass; `cargo test -p pii-tools -p pii-mcp`
   green.
2. **Fix the `pii-web` memory key and add one route-level test**
   (Finding 5). Change `"query"` to `"q"` at `main.rs:147`; add a
   `#[tokio::test]` that binds a temp UDS, answers one `memory.search`
   request, and asserts the received params contain `q`. Exit criteria:
   the new test fails on the current key and passes after; `cargo test -p
   pii-web` green.
3. **Apply `restart_backoff_ms` in `respawn`** (Finding 7). Sleep for the
   configured backoff before `spawn_and_hello`; keep `took_ms` measured
   from death detection so E13's `< 2000` ceiling still holds with the
   50 ms default. Exit criteria: new supervisor test with
   `restart_backoff_ms = 200` asserts `took_ms >= 200`, failing today;
   `restart_completes_under_2s` still passes.
4. **Wire or remove `same_layer_only`** (Finding 8). Preferred: implement
   `layer_of` against the store's layer field and add a consolidate test
   with two docs on different layers and the flag `true`, asserting no
   merge. Exit criteria: that test fails today; both production callers
   unchanged.
5. **F-11 pass 2: `PiiError::store_op` constructor, then `session.rs`.**
   Unchanged from the first list (was item 1). Still the right first
   step for the error-typing sweep, and still blocked on the constructor
   for the line-count reason given in Finding 4.
6. **Wire `xtask lint-files` into `quality-gate.sh` and de-duplicate the
   gate shapes.** Unchanged from the first list (was item 2). Add to it:
   make `lint-files` print a one-line success summary so gate logs show
   it ran (observation (a).3).
7. **Repair `clippy.toml`'s `println` entry** (observation (a).1). Change
   the path to `std::println` or delete the entry as redundant with
   `print_stdout = "deny"`. Exit criterion: `cargo clippy` output contains
   no `does not refer to a reachable macro` line. One-line change; ranked
   here rather than higher only because the workspace lint already
   enforces the rule.
8. **Fix `count_effective_lines` attribute handling.** Unchanged (was
   item 3).
9. **Close the cheap `pii-cli` unit-test gaps.** `cmd_run.rs`
   `Common::parse` and `report()`, `cmd_sys.rs` `run_ls` filter,
   `cmd_sandbox_exec.rs` exit-code coercion, `cmd_daemon.rs` exit-code
   mapping and `self_uid`. All pure; no daemon needed. Replace the
   `cmd_soak.rs` string match with the `RunStatus` enum from `pii-proto`
   if one is exposed, otherwise pin the strings with a test against the
   daemon's serialisation.
10. **Small hygiene, batchable into one docs/comments spec:** `200-OK`
    literal in `pii-http-fetch` (Finding 9); log the chmod failure in
    `cmd_daemon.rs:249` (Finding 10); reason comment or relaxation for
    the `ratatui` pin; stale "TLS cut to Phase 3b" comment in
    `pii-channels/src/discord.rs`; note in `pii-web`'s crate doc that
    `--host` off loopback exposes the daemon's read surface without
    authentication.
11. **Make the gate the single ruler for size reporting.** Unchanged
    (was item 4).
12. **Optional readability splits.** Unchanged (was item 5). Still not a
    policy requirement.
