# Open items — what is done and what is still missing

**As of 2026-10-11.** One place to answer *"where are we and what is left"*. Every closed spec carries its own
closure section in `specs/`; this file carries only the **open** list, with the evidence path for each, so a
resume needs no archaeology. Workspace-level mirror: `/projects/docs/05-operations/RESUME-REGISTER.md`.

## Done, verified, committed (each with `fresh-gate` ALL PASS on its own commit)

| spec | what it put in the product |
| --- | --- |
| 110–122 | the `057` vision plan, stage B: providers (OpenAI-compatible kind + anthropic), channels (webhook/telegram/matrix), SOP engine, model tiers, MQTT, HAL, WASM host, and four load defects found by measurement (trajectory leak, write amplification, two data-loss races, flaky checks) |
| 123–125 | **the self-improvement loop, closed**: extraction (cited candidates, injection control) → verifier (shadow run of the real engine, `confirmed`/`refuted`/`inconclusive`) → promotion (exactly one action: `quarantine_graph`, enforced by the engine with a named reason) → outcome window → **automatic revert** when the metric did not improve |
| 126–127 | memory: entities/relationships with content-derived ids, then hot/warm/cold tiering with demotion (never silent deletion) |
| 128 | the Discord egress: a `post` node (capability scope, bounded call, provider error preserved, **idempotence key** so a retry never double-posts), plus `discord-triage` and `discord-digest` graphs |
| 129 | the local-model rule enforced: `ollama:` refused with a pointer, `[core].ollama_host` refused by name, **`llamacpp-server:<model>`** is the local kind (host from `[llama] host`), the in-process `llamacpp:<path>` loader untouched; the production example now runs `granite-4.2-3b` on the llama.cpp server |
| 130 | the boss launcher dials through `pii_uds` (its net-check exemption removed; the production caller passes a real observer) |
| 131 | a refused run is a **decision**: the daemon records a deny in the decision ledger, keyed by the promotion, accumulating in the store's `attempts` ring |
| 132 | the Discord channel can post **into a forum** (a thread with a tag resolved by name) and declares the hosts it dials — and its live proof exposed two pre-existing defects (below) |

**Live proofs run by the orchestrator, not by the worker:** the promotion loop end to end (125), the `post` node
through a real daemon (128), three refusals in the decision ledger (131), and a **real tagged thread in the user's
forum** (132, deleted afterwards).

## Open — with evidence and the next action

### 1. The audit trail is three trails (design fork — needs the user)
A human approval lives in `approval.redb` (`ApprovalStore`), a promotion action in `learn/ledger.jsonl`, and a
refusal in the heal decision ledger (spec 131). The decision ledger's own doc says *"the ledger must show a denied
promotion just as durably as an approved one"* — the denials are there now, the **allows** are elsewhere.
Evidence + the three candidates: `RESUME-REGISTER` → "the loop's audit trail is THREE trails".
**Next:** the user picks (A mirror / **B one writer: the promotion becomes an RPC the daemon applies and records**
/ C document the join).

### 2. The quarantine test flake — cause NOT established
`daemon::tests::a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert` failed in **two gate
runs** (both on a loaded machine) and passed in **eight clean rounds** (one binary ×4, one under load; full suite
×3; export full suite ×2). Its 2 s poll ceiling was raised to 60 s with the assumption stated (`5cb67a9`) — **that
was not the mechanism**. It uses a unique tempdir and an in-process daemon, so the fixed-path class (below) does
not explain it.
**Next:** the gate now writes the failing step's output to `<export>/gate-output.log` and echoes the failing block
(`fed5e37`) — **read the panic on its next occurrence; do not guess a fix.**

### 3. Fixed in passing, worth remembering
A **deterministic** defect found while hunting #2: `crates/pii-memory/tests/persistence.rs` opened a fixed store
path, so two concurrent runs of the same tree (a developer's `cargo test` plus a gate run, or a worker's test run)
collided on the redb lock — **7 of 7** concurrent rounds before, **0 of 4** after (`eac80dc`). The harness is
`/projects/_tmp/conc-persist.sh`; the lesson is DO-NOT-REPEAT 1.48 (run a suspect test twice concurrently).

### 4. `store_op` residual (g4)
`crates/pii-store/src/lib.rs:96,103,357` — the op/resource swaps need an induced redb failure to be provable, and
there is no test-only way to induce one today.
**Next:** needs a fault-injection seam (a small `#[cfg(test)]` hook) or a spec that states the residual honestly.

### 5. laya lane (g6)
Policy-file loader spec; SPEC-E worker-lane wiring; merge `autoresearch/gate-lat2` (keep `e972452`). Two gate design
gaps filed 2026-10-10: **no heartbeat**, and **the agent cannot bootstrap the gate that gates it**.
**Next:** a spec for the policy loader; the merge is a decision (which branch wins).

### 6. `pii testkit` crate (g8) — candidate
`tempfile` is now a normal dependency of `pii-store` and `mod testing` is `pub`, so a small crate could own
scratch/store/fixture helpers for every crate's tests. Evidence: `crates/pii-store/src/testing.rs`.
**Next:** worth it only if a second or third crate duplicates those helpers — check before building.

### 7. Discord surface — decisions, not code
The forum requires a tag and **the adapter now applies one** (`alert` on failure-flavoured text, `status` on
cron/digest output, `chat` default), proven on the real path. pii's own channel can post into the forum (spec 132).
**Open:** (a) make the forum the *home* channel (every proactive Hermes message opens a thread — chat-hostile), or
keep `hermes` as home; (b) which tag a cron delivery should carry by default (`status` today, because the adapter
sees the `Cronjob Response:` prefix first); (c) the graphs `discord-triage`/`discord-digest` exist but are not
wired into a deployment (they need a `[[channel]]` entry and a token).

### 8. laya deployment
The running provider **is** llama.cpp (`laya-Q8_0.gguf` on `:8083`, `/props` → llama.cpp), and lance; there is **no
Dockerfile** in `/projects/laya-rs` and the deployment is systemd on the lnx host. The local `ollama.service`
(127.0.0.1:11434, unreachable from this container — measured `000`) is legacy and **must not be part of the final
version**; disabling it is the user's host action. `ollama-cloud` is a *cloud account* (`https://ollama.com/v1`),
a separate question.
**Next:** the user names where a Dockerfile/quadlet should live, or confirms systemd-only.

### 9b. The routine gate runs no tests
The per-item gate is `quality-gate.sh --routine`, whose step list is `gate-steps-quick.txt`: **fmt, clippy, build**.
`test (lib/bins/tests)` lives in `gate-steps.txt` (the commit and release boundary), so an item can regress the
workspace suite and still be recorded as "gate green". Measured 2026-10-18: `03e729a` (179's real fix, committed
by a cycle as a leftover) left `daemon::lag::tests::lagged_run_finished_still_starts_next_cron_firing` and
`…reactive_loop_lag_rearms_cron_chain_for_lost_run_finished` red at `HEAD` — `cargo test -p pii-server --lib`
= **127 passed, 2 failed** — and no cycle noticed for 1.5 h because no cycle ran the suite.
**Next:** measure one warm `cargo test --workspace` against `_tmp/gate-target` (minutes?), then decide whether the
routine gate gains a `test (lib)` step or the item boundary gains a cheaper "the crates the diff touches" step.
Recorded as spec 183's cause; the gate change itself is not authored yet because the cost is unmeasured.

**Measured 2026-10-18 06:18–06:25Z** (orchestrator, clean tree, HEAD `136b23e`): `cargo test -j 2 --workspace
--lib --bins --tests` with `CARGO_TARGET_DIR=_tmp/gate-target/test-gate` took **293 s (4.9 min)** and paid a
**cold** build — that cache is keyed to the export's source root, so the routine gate, which runs in the repo
tree, would build once into a new `test-repo-gate` dir before it is warm; scoped to one crate,
`cargo test -j 2 -p pii-server --lib --bins` took **96 s**. Neither is the 45-minute figure that blocked a
per-item test step, so the cost question is answered: a diff-scoped test step is affordable, a full-suite one is
affordable once warm. The gate change still needs its own spec.

**And that run found the suite RED at HEAD.** `crates/pii-server/tests/cron_runtime.rs:92`
`a_finished_cron_run_is_started_again_inside_the_runtime` fails deterministically (twice, alone in 15.1 s and
15.3 s): *"the cron was started 1 time(s); the re-arm never ran a second"*. It has been red since `03e729a`
(179's parked rule: a run suspended on its timer is not re-armed) — its 15 s ceiling encodes the pre-179
behaviour that produced 516 runs in four minutes — and it is a step of `gate-steps.txt`, so
`quality-gate.sh --fresh`, the gate a deploy card must quote, is **red at HEAD**. Authored as spec 184 / row A51,
which is now first in the queue.

### 9a. CLOSED 2026-11-01 — the gate certifies the tree it sits in, and the commit step now gates the index

**Verified closed today.** Spec 315 / row A179 landed as `e8a04f2` and I re-ran its proof rather than trusting the
row: `pre-commit-builds.sh` builds `HEAD` + the staged index before the commit exists, `cycle.sh:441` calls it under a
600 s timeout **above** the commit, a red leaves the index staged and does not spend an attempt, a timeout is reported
as a killed check rather than a failure, and `pre-commit-builds.test.sh` passes 21/21 including
`a-missing-repo-refuses-with-2` and `the-cycle-calls-it-before-the-commit`. The row had been marked `done` with **no
commit recorded**, so it was one of the 182 "not judgeable" rows — annotated to `e8a04f2`, which brings the audited
count to 5 and keeps violations at 0. The original text is kept below as the reasoning that led to the spec.


`067 §Q1` decided the item boundary runs the quick gate **in place** and the release boundary runs `fresh-gate`.
The hole that decision leaves was measured twice on 2026-10-17: `7f98624` (175) committed a call to
`pii_server::resolve_graph_dir`, a function that existed only in the uncommitted working tree, so `HEAD` did not
build `pii-cli` (`069 §0.1`, rc=101); and `HEAD`'s `Cargo.lock` was stale for a committed `pii-channels`
dev-dependency, so `cargo metadata --locked` fails on a clean export of `HEAD` (rc=101 measured 2026-10-18, rc=0
once the lock was committed). Both passed the loop's gate because it ran on the dirty tree.
**Next:** a spec where the commit step gates the tree it is about to commit — `git archive HEAD` plus `--overlay`
of the staged paths, which `xtask fresh_gate` already supports — and refuses the commit on red. Not authored yet
because the cost is unmeasured (a cold export build per item) and `067 §Q3` asks for caches keyed by source root:
measure one overlay gate, then decide between gating the whole pending tree and gating only the crates the item
touches.

**MEASURED 2026-10-30 — the cost, and the decision.** `git archive HEAD` into
`/projects/_tmp/stage-gate-cost-20261030T002931Z`: `cargo metadata --locked` **0 s**; `cargo check -q --workspace`
in a **new** target dir **38 s**; the same check of a second export against that target **8 s**. So the affordable
door is not "a fresh-gate before every commit" (45-48 min, §9b) but the build of the pending tree against the warm
`_tmp/head-builds-target` spec 245 already keeps: ~8 s per item. The choice between the two options above falls to
"gate the whole pending tree, with a build check, not the test list" — the tests stay at 314's touched-crate step.
Authored as **spec 315 / row A179**. One further fact decides the design: `head-builds.sh` exists and its own header
calls `--worktree` "the pre-commit door", but `cycle.sh:435` runs it **after** `git commit` and without `--worktree`
— detection of a broken `HEAD`, not prevention, and keyed on the worktree rather than the index a commit carries.

### 9c. The digest is scheduled only when a channel credential exists — and this host has none (CORRECTED 2026-11-01)

**Not a code gap — a correction to my own first reading.** I filed this as "the live config schedules nothing, so
the digest occurrence cannot fire", as if the schedule had been lost. It has not: `deploy/install-pii.sh` appends
the `[[cron]] name = "digest"` block **only when `HAVE_CHANNEL=1`**, which it sets when `PII_DISCORD_TOKEN` *and*
`PII_DISCORD_CHANNEL_ID` are both present (`install-pii.sh:102,128,141-142`). Neither is set on this host, the
installer says so in its own output (`no pii.env written, the daemon will report channels: []`), and therefore
`cron_entries: 0` and `armed: 0` are the correct, intended readings — a digest that cannot post is not scheduled
at all, which is better than one that fires and refuses.

**CORRECTED AGAIN 2026-11-01 (Opus review) — the credential is on disk and I said it was not.**
`/root/.config/pii/pii.env` exists, mode 0600, 245 bytes, 2026-10-19, and holds **both** `PII_DISCORD_TOKEN`
(72 chars) and `PII_DISCORD_CHANNEL_ID`. So nothing needs to be "placed". Two things are actually missing:

1. **The hand-started daemon has none of it.** The unit carries `EnvironmentFile=-/root/.config/pii/pii.env`, so a
   *systemd* start would load the env; this container has no systemd bus, the daemon is started by hand, and
   `/proc/<pid>/environ` holds no `PII_DISCORD` variable at all (measured). The daemon therefore reports
   `channels: []` and cannot post even though the credential is right there.
2. **The config has no `[[cron]] digest`** because that block is appended only when `HAVE_CHANNEL=1` at *install*
   time, and the install ran without the variables exported.

The fix is one command sequence, not a code change: source `pii.env`, re-run `deploy/install-pii.sh` (its config
half appends the digest block), then start the daemon with that same environment. It has not been done because
the first digest would post to a real channel — that is the operator's decision, not the loop's.

### 9. Smaller filed items
- the two audit trails should be unified — see #1;
- ~~a CLI `learn revert` racing the daemon's sweep (filed with 125)~~ — closed by spec 149: a daemon-owned store takes the revert as a request; without one the CLI applies it directly under a cross-process writer lock (`learn/.writer.lock`) and prints `path=direct`;
- **resumed runs of a quarantined graph are not refused** (filed with 125);
- `pii-evolve`'s direct socket dial — **closed by 130**;
- unify the promotion JSONL with the decision ledger — see #1.

## How to re-verify any of this without me

```bash
cd /projects/pi-improved
cargo test -p pii-channels -p pii-server -p pii-store -p pii-learn   # the suites
cargo run -q -p xtask -- deps-check && cargo run -q -p xtask -- net-check \
  && cargo run -q -p xtask -- doc-drift-check && cargo run -q -p xtask -- lint-observability
bash quality-gate.sh                                     # the 14-step gate in this tree
cargo run -q -p xtask -- fresh-gate                      # the CI-equivalent run in an export of HEAD
bash /projects/_tmp/verify132-live.sh                    # the live Discord proof (needs the token env)
bash /projects/_tmp/verify131-live.sh                    # three refusals → the ledger row
bash /projects/_tmp/conc-persist.sh 4                    # must report 0 of 4 collisions
```

### 10. Two orphan test daemons from a gate run, nine days old (REAPED 2026-11-01)

`_tmp/tools-gate/debug/pii daemon` pids 583964 (21 MB) and 585460 (18 MB), started 2026-10-22 by a gate's own test,
holding their redb stores and no listening sockets. They were not production (pid 1744962 is) and nothing referenced
them, but a running process must not be killed silently either — so they were reaped with argv, age, socket count and
the production-survived check written to `/projects/_tmp/reaped-test-daemons-20261101T235957Z.log`. Cause: a gate
step starts a daemon and does not reap it. **Follow-up worth one line in that step: kill what you start.**

### 11. The gate's red was DISK PRESSURE, and the gate was causing it (found and fixed 2026-11-02)

`prompt_returns_before_run_finishes` (`crates/pii-server/tests/latency.rs:97`, a 50 ms budget on `session.prompt`)
began failing consistently — 120, 161, 168, 194, 235, 309 ms — and it was **not** the code: the same `git archive
HEAD` export passes in **0.03 s** on `/tmp` and fails on `/projects`, in a clean environment, on an idle 32-core box.

The cause was the volume's latency under pressure: `/projects` is `ext4` and was **91 % full**, where a 4 KB
write+fsync measured **21.9 ms median (69.2 ms max)** against **0.1 ms** on the overlay that backs `/tmp`.

**The gate was filling the volume itself**: a failed gate keeps its export, and one of them
(`/projects/_tmp/fresh-gate-2106840`) was **28 GB**. So: gate fails → keeps 28 GB → volume slower → the latency
assertion fails → the next gate runs → and the loop feeds itself.

Two fixes, both verified:
1. **`FRESH_GATE_TARGET=/projects/_cache/targets`** when running the gate. `fresh_gate.rs:227` documents this as the
   warm-cache door ("the export's TREE must be fresh; its COMPILER OUTPUT does not have to be"); it stops each run
   from building its own 28 GB target. With it, `fresh-gate --worktree` returned **PASS, rc 0** and the disk *ended
   healthier than it started* (49 G → 76 G free).
2. Reclaiming the leftover export, with a manifest, under the repo's own rule that a non-kept `fresh-gate-*` dir is
   safe to remove (`fresh_gate.rs:74,110`).

**The lesson worth keeping:** a gate that measures latency is a gauge of the machine as much as of the code, and this
one had a feedback loop into its own input. Before believing a red on a latency/composite assertion, check the volume
and the export count — and read `df` before reading the diff.
