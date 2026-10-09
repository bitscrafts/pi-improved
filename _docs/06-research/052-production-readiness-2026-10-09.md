# 052 — Pii: honest production-readiness assessment and the road to production

**Date**: 2026-10-09 · **Repo HEAD**: `9be2b24` (tree clean) · **Assessor**: orchestrator
**Method**: every claim below is a measurement taken on that HEAD, or a citation of an existing
review/evidence file. Commands are quoted so they can be re-run. Where something is unknown it is
marked **UNKNOWN** rather than estimated.

**Sources compared**: the original specification — `_docs/01-overview/001-pii-vision.md` (the four
pillars, the architectural decisions, the six-phase plan) and `_docs/06-research/002-…md` (the full
architecture plan) — against the tree, plus the two adversarial reviews already in the repo:
`043-opus-full-review-2026-10-06.md` and `045-opus-rereview-2026-10-09.md` (both now carrying
disposition banners). This document is the third leg: not a review of code, but an assessment of the
**product**.

---

## 1. Verdict

**Pii is a real, working, well-built *engine* with a genuine coding-agent loop — and it is not yet a
product.** The substrate is in better shape than most projects at this stage: ~51 k lines of Rust,
613 passing tests, a strict lint posture, an eight-check quality gate plus a fresh-tree variant, and a
defect found this week by an adversarial re-review and closed with a mutation-proven fix. What is
missing is everything between "the engine runs" and "someone can rely on it": packaging, deployment,
observability, secret and permission handling at the OS level, backup and upgrade paths — and the
three quarters of the original vision (gateway breadth, IoT, self-improvement) that were never built.

The honest one-liner: **the coding agent works end-to-end today with a local model; the assistant,
IoT and self-improvement pillars in the specification are absent, and there is no production
deployment story at all.**

## 2. What I ran (the evidence behind everything below)

```bash
# shape
git log --oneline -1                       # 9be2b24
ls crates/                                 # 19 crates + xtask
find crates xtask -name '*.rs' | xargs wc -l | tail -1     # 51,142 lines / 212+ files
# gate (this session)
bash quality-gate.sh                       # exit 0, 8 checks PASS
grep -E 'test result: ok\.' … | awk …      # passed=613 failed=0 ignored=2 (82 test blocks)
# the artefact, used as a user would
./target/debug/pii --version               # 3-4 ms, three runs
./target/debug/pii -p "reply with exactly: PONG" --model ollama:granite4.2:3b   # 5.51 s, success
./target/debug/pii -p "Create a file named hello.txt … containing exactly: hi"  # see §3
./target/debug/pii daemon --store <scratch>/store.redb            # starts, writes 4 redb stores
./target/debug/pii sandbox-exec --workspace <d> -- /bin/echo hi   # usage: needs --profile <p>
```

Tooling versions: Rust 1.95.0, `x86_64-unknown-linux-gnu` (no musl target installed), ollama reachable
on `127.0.0.1:11434` with `granite4.2:3b` (2.1 GB) available.

## 3. What works, with the decisive evidence

The strongest single result of this assessment: **the agent is real.**

```
$ pii -p "Create a file named hello.txt in the workspace containing exactly the single word: hi" \
      --model ollama:granite4.2:3b
{"kind":"tool_call","node":"1","tool":"fs_write","args":{"content":"hi","path":"hello.txt"}}
{"kind":"tool_result","node":"1","tool":"fs_write","ok":true,"summary":"{\"written\":2}"}
{"kind":"node_finished","node":"agent","port":"ok",
 "value":{"files_changed":["…/hello.txt"],"text":"File hello.txt created successfully…",
          "tokens_in":1003,"tokens_out":106,"turns":2}}
{"kind":"run_finished","terminal":{"kind":"success", …}}

$ ls -la ; cat hello.txt
-rw-r--r-- 1 root root 2 … hello.txt
hi
```

A local 3B model, through the real graph engine, called a real tool, and the effect is on disk —
`files_changed` is reported by the engine, not asserted by me. The event stream (`node_started`,
`assistant_delta`, `tool_call`, `tool_result`, `node_finished`, `run_finished`) is the journal the
design promised.

| capability | measured state |
|---|---|
| coding-agent loop (`pii -p`, tools, event journal) | **works**, proven above |
| graph engine (DAG + bounded cycles, journal, resume, replay) | 6,411 lines; `pii graph {lint,show,run,resume,trace}`; resume/replay tested — and a replay defect found 2026-10-09 and fixed (`048-`, `049-`) |
| capability-gated `Ctx` (fs scope, denials observed) | implemented in-process (`pii-core/src/fscap.rs`, `ctx.rs`); denial events emitted |
| durable store (redb, single writer, daemon owns the DB) | `pii-store` 3,766 lines; daemon start creates `store/approval/heal/memory.redb` |
| assistant daemon skeleton | starts; UDS control plane; approvals (`approval.redb`), self-heal (`heal.redb`), timer-wheel scheduler, channels router |
| memory | `pii-memory` 1,709 lines: BM25 + vector + RRF fusion + decay + consolidation (+ a graph lane) |
| MCP bridge | `pii-mcp` binary (422 lines) |
| clients | `pii-tui` (946 lines), `pii-web` (520 lines), `pii-uds` client helper (120 lines) |
| cold start | `pii --version` **3–4 ms**; a full one-shot agent run is **5.5 s**, dominated by the model (927 in / 61 out on a 3B) |

## 4. Quality: measured, not asserted

| dimension | state | evidence |
|---|---|---|
| tests | **613 passed, 0 failed, 2 ignored** | `quality-gate.sh` log, this session |
| lint posture | `unsafe_code = forbid`; `unwrap_used`, `expect_used`, `panic`, `indexing_slicing` = **deny**; `panic = "abort"` in release | workspace `Cargo.toml` |
| gate | 8 checks (fmt · clippy · clippy `llamacpp` · tests lib/bins/tests · tests doc · `lint-files` · `lint-scratch` · `reachability-check`) **plus** a fresh-tree variant that runs the same gate in a `git archive` export | `quality-gate.sh`, `_docs/02-guides/quality-gate-and-fresh-tree.md` |
| check quality | the recent batch's guards are **mutation-proven** (revert the fix → named test goes red), including a real defect: replay diverged from the live run on a budget error | `046-`, `047-`, `049-` |
| adversarial review | two full opus reviews (043, 045); 045's findings all dispositioned, three were errors in my own earlier claims | `043-`, `045-` |
| file hygiene | max effective file 373 lines (warn at 380); ratcheted by `lint-files` | gate output |
| known weak spots | `pii-web`/`pii-tui`/`pii-channels`/`pii-uds`/`pii-evolve` are thin and lightly tested; three `store_op` sites can't be pinned without a redb fault seam (`R5b-residual`); two weak tests left unchanged (N5, N6) | `045-`, register |

**Not measured anywhere: coverage.** No coverage tooling is configured, so "613 tests" says nothing
about what fraction of the 51 k lines is exercised. That is a real gap in this assessment and in the
project's own instrumentation.

## 5. Specification vs reality

### 5.1 The four pillars (`001-pii-vision.md`)

| pillar | spec says | reality (measured) | verdict |
|---|---|---|---|
| 1. Coding-agent core | sub-100 ms cold start, structured concurrency, capability-gated `Ctx`, JSONL sessions, bash mediation | 3–4 ms CLI start; `Ctx`+fscap implemented; graph/journal/sessions real; tools (`bash`, `fs`, `search`, `worktree`) real; E2E edit proven | **BUILT** |
| 2. Assistant/gateway | **30+ channels**, **~20 LLM providers**, SOP engine, WASM plugin host, hardware HAL | **2 channels** (`cli`, `discord`); **3 provider kinds** (`scripted:`, `ollama:`, `llamacpp:` — the last behind a feature that is off by default); **no SOP**, **no WASM**, **no MQTT/HAL** (`grep` finds none) | **ABSENT (skeleton only)** |
| 3. Cognitive memory + model tiering | entities/relationships; tier-aware tool selection 0.5B→100B+ | 1,709 lines of document memory (BM25 + vector + RRF + decay + consolidation); **no entity/relationship model** (`entity` occurrences in `pii-memory/src` = 0); **no tier-aware tool selection** | **PARTIAL** |
| 4. Self-improvement loop | skill extraction, procedural templates, trajectory logging for future fine-tuning | `pii-evolve` (655 lines) is a **boss/cron launcher** (`run_cycle`, `StateDoc`), not the pillar described; no trajectory log, no skill extraction, no distillation | **ABSENT** |

### 5.2 The six-phase plan

| phase | scope | reality | verdict |
|---|---|---|---|
| 0 | toolchain + workspace + CI gates | workspace, 8-check gate, fresh-tree gate, ratchets | **DONE** |
| 1 | core engine (Ctx, graph, journal, resume, cold start) | done, tested, and hardened this week | **DONE** |
| 2 | coding harness (tools, sandbox, verify-fix) | tools done; agent loop done; **sandbox is a profile-driven `sandbox-exec`, with no OS-level confinement**; verify-fix exists as graph shape | **PARTIAL** |
| 3 | assistant/gateway (daemon, UDS, channels) | daemon, UDS, approvals, cron, memory done; **channels 2 of 30+** | **PARTIAL** |
| 4 | IoT/reactive (MQTT, hardware, WASM) | absent | **ABSENT** |
| 5 | self-improvement (skills, distillation, trajectories) | absent | **ABSENT** |

### 5.3 The named architectural decisions

| decision | reality |
|---|---|
| Rust workspace, `pii-abi` as kernel ABI, file-size gate | done (19 crates + xtask; `pii-abi` traits/types only) |
| graph orchestration, event-sourced journal, `Suspend` in the node return type | done, and exercised daily by the spec harness |
| capability `Ctx` that actually enforces, **three layers (Ctx / nono Landlock / WASI)** | **one layer only** — in-process `Ctx`/fscap; no Landlock, no seccomp, no WASI |
| redb persistence, daemon owns the DB, UDS control channel | done |
| WASM plugins (`wasmtime 47.x`, WASI 0.2, epoch interruption) | **absent** |
| worker IPC: SOCK_STREAM UDS + JSON-RPC 2.0 + optional SCM_RIGHTS | UDS + JSON-RPC present; SCM_RIGHTS fd handoff **UNKNOWN** (not found in a first pass) |
| self-improvement: skills, `Skill.graph`, zero-token promotion | absent |
| trajectory logging for future fine-tuning | absent |

### 5.4 The spec's headline claim

`001-pii-vision.md:3` — *"a **single static Rust binary** that unifies …"*. Reality: **four binaries**
(`pii`, `pii-mcp`, `pii-web`, `pii-evolve`, each with its own `src/main.rs`), and the binary is
**dynamically linked**:

```
$ file target/debug/pii
ELF 64-bit LSB pie executable, x86-64, dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2
$ ldd target/debug/pii
linux-vdso.so.1  libgcc_s.so.1  libm.so.6  libc.so.6  /lib64/ld-linux-x86-64.so.2
```

No musl target is installed, so the static-build requirement the design justified many decisions with
("pure Rust, no C dep for static musl") **has never been demonstrated**. This is the single
clearest spec-vs-reality gap, and it is a *packaging* gap, not an engine one.

## 6. Documentation vs reality

The docs are unusually good for this stage (SSOT tree, index/TOC, research numbering, HANDOFF
protocol) — and several claims in them are stale. Measured today:

| doc claim | reality | action |
|---|---|---|
| `001`: "~14 crates" | 19 crates + xtask | stale (this doc) |
| `001`: "hard 2,000-line file-size gate" | ratchet at 380 warn, max effective 373 | stale (this doc) |
| `001`/`INDEX`: "single static binary" | 4 binaries, dynamically linked | stale (this doc) |
| `001`: "30+ channels" / "~20 providers" | 2 / 3 | stale (this doc) |
| `001`: "entities/relationships" in memory | not implemented | stale (this doc) |
| `001`: WASM plugin host, MQTT, HAL | absent | stale (this doc) |
| `HANDOFF` / `AGENTS.md` gate description | corrected today (the gate has 8 checks + two trees) | fixed |
| `043`/`045` narratives (E0463 "the gate can never pass") | proven false; comments rewritten | fixed (spec 90) |
| `INDEX` crate/spec counts | updated today (96 specs; 19 crates + xtask) | fixed |

The pattern: **the architecture/vision documents describe the intended system, and nothing in the
repo distinguishes "intended" from "built"**. A production-readiness project needs that distinction
machine-checkable, not prose — which is what `lint-files`/`reachability-check` do for code and what
§8 below proposes for the vision itself.

## 7. Production readiness by axis

| axis | status | the blocking evidence |
|---|---|---|
| **Packaging & distribution** | **absent** | no Dockerfile, no systemd unit, no install script, no release build committed, no musl static target; `target/release/pii` does not exist |
| **Deployment / operations** | **absent** | nothing in the repo describes how to run the daemon as a service: no unit file, no supervision, no log destination (logs go to stderr via `eprintln`), no runbook — and **no top-level README** (the root holds `AGENTS.md`/`CLAUDE.md`/`HANDOFF.md` and no entry point for a newcomer) |
| **Observability** | **minimal** | internal `ObsEvent` streams (denials, persist errors) exist and the control plane exposes `sys.version`; **no metrics, no tracing, no dedicated health/readiness probe** (nothing reports store/scheduler/channel state), **no log file or rotation** |
| **Security (OS level)** | **absent** | **app-layer** mediation is real and per-tool: `bash` = deny list → cwd scope check → `ProcCap::run` → output caps (`pii-tools/src/bash.rs:1-2`); `http-fetch` = allowlist + Ctx grant key `http:<host>`, GET-only, no headers/body (`pii-http-fetch/src/fetch.rs:3`). **Missing: the design's layers 2 and 3** — no Landlock/seccomp, no WASI; `sandbox-exec` needs a caller-supplied profile; **secrets are env vars only**; any command the deny list does not name can still run with full process rights |
| **Data durability & recovery** | **partial** | redb single-writer + journal + resume + `heal` are real; **no backup/restore tooling, no compaction story, no upgrade/migration path for the store schema**, and the graph/spec format has no version field I could find |
| **Performance & scale** | **UNKNOWN beyond cold start** | 3–4 ms CLI start measured; one-shot run 5.5 s (model-dominated); **no throughput, concurrency, memory or long-run soak numbers**; a load-sensitive test (`run.start` 53.8 ms under parallel gate load) is the only load data point in the repo |
| **Config & secrets** | **partial** | TOML `[core]` with precedence env > file > default; no secret store, no config schema/validation command, no documented sample production config |
| **Multi-instance / isolation** | **UNKNOWN / partial** | `pii-uds::runtime_dir(instance)` implies per-instance dirs; worker isolation is opt-in per the design; no measurement of two daemons sharing a host |
| **API stability** | **not established** | MCP + UDS control plane exist; no versioning or compatibility policy documented |
| **Licensing / provenance** | **UNKNOWN** | 5 reference repos are read-only clones; the code is derived from them — **no LICENSE decision recorded in the repo** (a real blocker for distribution) |
| **Docs as a product** | **good but aspirational** | see §6 |

## 8. The road to production

Ordered by *what blocks a real user*, each phase with a measurable exit criterion. Estimates are
elapsed-time ranges for this codebase's observed pace (one focused session ≈ 2-4 landed, verified
specs), **not** commitments.

### P0 — Make it deployable (the smallest thing that can run as a service)
- **Goal**: a target host where a user starts pii, it keeps running, and they can see why it died.
- Work: release build + musl static target **or** an explicit decision to ship dynamically; a
  `Dockerfile`/systemd unit; a documented production config sample; log-to-file with rotation; a
  health/readiness probe; `--help` for `pii daemon` (today `--help` is an unknown flag and only
  usage strings exist).
- **Exit criteria**: `cargo build --release --target x86_64-unknown-linux-musl -p pii-cli` exits 0 and
  `file` reports *statically linked* (or the decision to drop the static claim is recorded in `001`);
  a container/systemd run starts, answers a probe, and survives a `kill -9` + restart with the store
  intact; `pii daemon --help` exits 0.
- **Effort**: 1–2 sessions. **Risk**: low.

### P1 — Make it operable (backup, upgrade, observability)
- **Goal**: the operator can recover and can see the system's state without reading logs.
- Work: backup/restore + consistency check for the redb stores; a documented schema/format version and
  a migration step; metrics (even a minimal text endpoint) and a health endpoint; structured logging
  to a file; a short runbook.
- **Exit criteria**: a documented backup → wipe → restore round-trip is green in a scratch dir; a
  deliberately bumped store-version is refused with a clear message rather than corrupting; the daemon
  exposes one health endpoint that reports store/scheduler/channel state.
- **Effort**: 2–3 sessions. **Risk**: medium (schema versioning touches every store table).

### P2 — Make it safe (security to leave a dev box)
- **Goal**: a compromised or confused model cannot exceed a declared, enforced boundary.
- Work: decide and document the sandbox story — implement Landlock (Linux) under `ProcCap`/`sandbox-exec`,
  or state plainly that confinement is app-layer and add compensating controls; secret handling beyond
  env vars (a 0600 file or a keyring); persist the existing denial events as an audit trail. Note that
  the app-layer allowlists already exist (`http:<host>` for fetch, deny list + cwd scope for bash) — the
  gap is the OS layer and the fact that a command the deny list does not name runs unconfined.
- **Exit criteria**: a red-team fixture (a model instruction that tries to write outside the scope, and
  one that tries to fetch a host outside the allowlist) is denied and produces an audit record; the
  OS-level layer is demonstrably active (a Landlock self-test) or the doc states its absence in the
  same sentence as the claim it qualifies.
- **Effort**: 2–4 sessions. **Risk**: medium-high (a real security boundary is easy to get subtly wrong).

### P3 — Close the quality gaps in what already ships
- **Goal**: the thin crates are as trustworthy as the engine.
- Work: coverage instrumentation with a beginning baseline; tests for `pii-channels`, `pii-uds`,
  `pii-web`, `pii-tui`, `pii-evolve`; the two weak tests (N5 worktree containment, N6 spec-73
  override); the `R5b-residual` redb fault seam; a decision on the `tempfile`-in-`pii-store`
  trade-off (`pii testkit-crate`).
- **Exit criteria**: a coverage number exists and is ratcheted in the gate; each named gap has a test
  that fails under the corresponding mutation.
- **Effort**: 2–3 sessions. **Risk**: low.

### P4 — Decide the vision, then build or re-scope it
- **Goal**: the specification and the product agree.
- Work: an explicit **decision document** on the three absent pillars — gateway breadth (channels,
  providers), IoT (MQTT/HAL/WASM), self-improvement (skills/trajectories). Each is months of work at
  the current rate; the alternative is to re-scope `001` to what pii actually is: **a local-first
  coding agent with a durable graph engine and a personal-assistant skeleton**.
- **Exit criteria**: either a phased spec set exists for a chosen pillar with its first spec landed,
  or `001` carries a "shipped scope" section that lists the absent pillars as out of scope.
- **Effort**: 1 session to decide; 6+ sessions per pillar if built. **Risk**: high (scope).

### P5 — Prove it under load and over time
- **Goal**: numbers that justify "production" beyond "it runs".
- Work: throughput/latency under concurrent runs; a multi-hour soak with the store and journal;
  crash-injection (kill during a write) with recovery assertions; a memory ceiling.
- **Exit criteria**: a soak run of ≥ 4 h with zero lost events and bounded RSS; documented p50/p95 for
  the agent loop and for each RPC method under load.
- **Effort**: 2–3 sessions. **Risk**: medium (findings likely: the load-sensitive `run.start` path and
  the single-writer store are the first suspects).

## 9. Not verified (do not read as "fine")

- Coverage of anything (no tooling).
- Any performance number other than cold start and one 3B-model run.
- SCM_RIGHTS fd handoff, and worker isolation, in practice.
- The static/ musl build (never attempted in this environment).
- Behaviour of a second daemon instance on the same host.
- Licensing provenance of the derivation from the five reference repos.
- The `pii-web`/`pii-tui` clients as used by a human (only their line counts and entry points were read).
- MCP bridge end-to-end against a real Hermes client in this session.

## 10. Recommendation

**Ship the engine as a developer tool now, and treat "product" as P0+P1 — not the vision.** The
coding agent, the graph engine and the store are in genuinely good shape and are already used daily by
the orchestration harness in this workspace; the gap to a *reliable personal tool* is packaging,
backup, and observability, which is ~3–5 focused sessions. The four-pillar vision is a different,
much larger programme, and three of its four pillars are unbuilt — deciding that explicitly (P4) is
worth more than starting any of them, because every stale "30+ channels" claim in the docs is a
promise the code does not keep.

---

## STATUS UPDATE (added 2026-10-09, end of day) — P0 and P1 are CLOSED
Both phases in §8 landed as specs, each with an orchestrator-reproduced proof (not a worker paste):
- **P0 deployable → spec 104** (`07b8eba`): the musl release build **is** static
  (`file` → `static-pie linked`, 4,596,744 bytes; `ldd` → "statically linked"), and `001-pii-vision.md` now
  says exactly that — the default build stays dynamic and `pii-web`/`pii-mcp`/`pii-evolve` are named as
  unmeasured. `pii daemon --help` works (`--bogus` rc=2), `examples/pii.production.toml` documents every
  key, the log file rotates (verified: 3 files at keep=2), the probe exits 2 with no daemon and 0 live, and
  `kill -9` + restart leaves the store intact. Artefacts: `deploy/Dockerfile` (**untested here — no
  Docker**), `deploy/pii.service` (`systemd-analyze verify` rc=0, never run as a service).
- **P1 operable → spec 105** (`e2a47c8`): every redb the daemon opens is stamped and a NEWER version is
  refused with a clear message; backup → wipe → restore verified with counts identical; the restore guard
  refuses a non-empty target without `--force`; `pii metrics`; runbook in
  `_docs/05-operations/105-runbook.md`.
- **The observability mandate → specs 97-103 + 106** (see `053-…md` §OUTCOME): the contract crate, the
  failing gate step, health identity, the adoption sweep (15 crates emit, 4 sanctioned exemptions), the
  bounded enforcement, the trajectory substrate, and three operator-surface defects found by chasing a
  residual (`run ls` listed nothing by construction; an unknown `--status` read as "no runs" at rc=0; a
  suspended run's approval vanished to a journal-flush race).

**What P2-P5 still have to do** (unchanged in intent, now with the integration points live):
- **P2 safety**: the Landlock/`ProcCap` decision (or the documented absence), persist denial events as an
  audit trail (the contract now carries them, so this is cheap), secret handling beyond env vars. The
  data-safety half of observability lives here: what must NEVER reach a log or a trajectory, tested against
  a canary secret — the sink already exists (`pii-observe` + `103`), so the policy can be tested at the sink.
- **P3 quality gaps**: coverage tooling (still unmeasured) and the per-subsystem ADEQUACY review — whether
  each crate emits the right events for an operator to diagnose a failure. The mechanical floor (one named
  event test per crate) is in place; the review is not.
- **P4**: decide the four-pillar vision; pillars 2 (gateway) and 4 (self-improvement, now with the
  trajectory substrate as its first brick) need an explicit build-or-rescope decision.
- **P5**: load/soak, including the instrumentation cost of what 103/105/106 added.

**Residual defects carried, not closed:** resolving+resuming a *restored* pending approval after a restart
is untested; a `kill -9` leaves no `daemon.stopped` line (documented, not fixed); `timer.redb` is now in
health but the wheel itself is still inside the scheduler.

---

## CORRECTION to §7 and §8/P2 (measured 2026-10-09, by the orchestrator)
**This document said "security app-layer only (no Landlock/seccomp)". That is WRONG about Landlock.** It
was written from a smoke test in which the daemon ran with the `off` profile, and I read the absence of a
profile as the absence of the mechanism.

Measured on this host (kernel `7.1.13`, Landlock ABI **v9** — `landlock_create_ruleset(NULL,0,VERSION)` → 9):
```
pii sandbox-exec --profile workspace --workspace $W/ws -- bash -c 'echo inside > $W/ws/inside.txt'   → rc=0, created
pii sandbox-exec --profile workspace --workspace $W/ws -- bash -c 'echo outside > $W/out/x.txt'     → rc=1 "Permission denied", NOT created
pii sandbox-exec --profile workspace --workspace $W/ws -- bash -c 'echo clobber > $W/out/keep.txt'  → rc=1 "Permission denied", original intact
pii sandbox-exec --profile workspace --workspace $W/ws -- bash -c 'head -c 20 /etc/passwd >/dev/null' → rc=0 (reads outside are ALLOWED)
pii sandbox-exec --profile off       --workspace $W/ws -- bash -c 'echo x > $W/out/off.txt'          → rc=0 (control: permissive)
```
So: **Landlock confinement is implemented** (`crates/pii-tools/src/sandbox.rs` via the `landlock` crate;
applied process-wide in `pii sandbox-exec` and the one-shot/daemon paths) and it **enforces write
confinement**, with `off` as a working control. What is actually missing is narrower and sharper:
1. **The daemon's default profile is `off`** — the capability exists but is not on unless asked for, so the
   shipped default is the unconfined one. That is a default-safety decision, not a missing feature.
2. **The workspace profile confines writes, not reads** — reading `/etc/passwd` succeeds. Whether that is
   intended must be stated precisely (and the health string should say what is granted for read, not only
   for write).
3. **A kernel-level denial inside a child process is invisible to pii** — it surfaces as the child's
   `Permission denied`; only denials that go through pii's own tools become `Denial` events. So the audit
   trail is partial by construction and must say so.
4. **seccomp is genuinely absent**, and secrets remain env-vars-only.

