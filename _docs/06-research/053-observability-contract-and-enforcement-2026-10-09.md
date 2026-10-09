# 053 — Observability: measured state, the contract, and how to enforce it on every crate

**Date**: 2026-10-09 · **Repo HEAD**: `9be2b24` · **Status**: analysis + design (implements nothing)
**Directives this answers**: *"observability is a must"* (user, 2026-09-20) and the derived finding that
**a health surface must report IDENTITY, not just liveness** — *"the health endpoint must report store,
mounts, version, and counts to make divergences visible immediately"* (amem
`amem/observability/health-must-report-identity-not-just-liveness-2026-09-20`). The user's instruction
for this phase: **"we must enforce observability! via traits on every crate!"**

---

## 1. Measured current state (all numbers from `grep`/`sed` on HEAD)

There IS a contract today — one trait, in one crate, half-adopted:

```rust
// crates/pii-core/src/observe.rs
pub enum ObsEvent { Run(..), Journal { seq, event }, Denial { principal, cap }, PersistError { context, message } }
pub struct Metric { pub name: String, pub value: u64 }
pub trait Observer: Send + Sync + 'static { fn event(&self, &ObsEvent); fn metric(&self, &Metric); fn flush(&self); }
```

| crate | observability hits (`Observer|ObsEvent|observe::|Metric`) | raw `eprintln!`/`println!` |
|---|---|---|
| pii-core (the trait's home) | 44 | 0 |
| pii-server | 30 | 14 |
| pii-cli | 15 | **103** |
| pii-graph | 10 | 0 |
| pii-store | 2 | 0 |
| **pii-abi, pii-channels, pii-evolve, pii-http-fetch, pii-mcp, pii-memory, pii-model, pii-net, pii-proto, pii-tools, pii-tui, pii-uds, pii-web, pii-worker** | **0** | 22 total (web 8, mcp 7, evolve 5, net 2) |
| **totals** | 101 | **139** |

Five `Observer` implementations exist (`NullObserver`, `RecordingObserver`, `MultiObserver`,
`BridgeObserver` in the server bus, `NdjsonObserver` in the CLI). Metrics are emitted ad hoc from the
graph engine only (`graph.node.dead`, `graph.join.unsatisfiable`, `graph.batch.width`) and consumed by
tests. There are **no levels, no spans, no latency histograms, no subsystem tags, and no log file**.

### 1.1 The health surface that exists — and what identity it lacks

`sys.status` is real (`crates/pii-server/src/daemon/status.rs`). It reports: `uptime_ms`,
`runs_active`, `sessions`, `sandbox_profile` (a string), `protocol` version, `rss_kb`, workers
(name/state/pid/restarts), listeners (id/proto/addr), channels (name/kind/errors/cursor), memory docs.

Against the directive (*store, mounts, version, counts*), it is **missing**:

| required identity | today |
|---|---|
| **store identity** — which store file, its schema/format version, size, last write, open mode | **absent** (no field) |
| **mounts** — the resolved fs-allowed roots (workspace, `channels_scope`, runtime dir) actually in force | **absent** (only a profile *name*) |
| **version** — build identity (git sha/build id), binary version, config path + hash, model provider in force | only `PROTOCOL_VERSION` |
| **counts** — runs total, journal length, approvals pending, cron entries, store table counts | only `memory.docs` |

This is exactly the defect class the directive names: two daemons (or one daemon after a config
change) can report identical, healthy-looking status while serving **different stores**. Nothing in
the current payload can tell them apart.

## 2. What "enforced observability via traits on every crate" must mean

1. **One contract, reachable by every crate.** The trait must live where every crate can depend on it
   without dragging in the engine. Today it lives in `pii-core` (the engine); `pii-proto`, `pii-tui`
   and `pii-web` do not depend on `pii-core` at all, and `pii-store` depends only on `pii-abi`.
2. **A seam per crate, not a print.** Every crate must emit through the contract at its boundaries —
   open/close, denial, error, retry, queue depth, latency — instead of raw `eprintln!`.
3. **Identity, not just liveness**, in health: store, mounts, version, counts (§1.1).
4. **Enforcement that fails the gate** — the project's established pattern (`lint-scratch`,
   `reachability-check`): a new crate or a new raw print must break the build, not a convention
   nobody reads. A warning would not survive the next cheap worker run.
5. **Machine-checkable naming**, so tests and dashboards can assert on it: metric names
   `<subsystem>.<thing>`, event kinds `<subsystem>.<verb>`, health keys stable.

## 3. Proposed contract (design)

### 3.1 Where it lives: a dedicated `pii-observe` crate (decided)
Two candidates were weighed, and the decision is a **new crate**, `pii-observe`:

- **`pii-abi`** (today's kernel ABI) would work: it is traits + types, no transport, and 12 crates
  already depend on it. But its documented purpose is the **capability/ABI surface** (`Ctx`, `RunEvent`,
  `PiiError`), and it carries `serde_json`, `schemars`, `bitflags` for that purpose. Observability is a
  *cross-cutting* contract consumed by crates that need none of that (`pii-tools` has no ABI need;
  `pii-web`/`pii-tui` are clients). Folding it in makes one crate answer two unrelated questions.
- **`pii-observe`** (decided): one small crate, `serde` + `serde_json` only, holding the traits, the
  event/measure types, the health types, and — the real payoff — a **name registry of `pub const`
  metric/event names**, so tests and dashboards assert on symbols rather than string literals, and the
  enforcement lint has one place to look for the sanctioned vocabulary.

Shape: `pii-observe` (contract) ← every crate; `pii-core` **implements the runtime** (multiplexer,
sinks, registry, recording observer) and re-exports the contract for the 101 existing call sites, so the
migration is additive and the dependency direction stays contract → implementation.

### 3.2 The vocabulary
```rust
// pii-abi::observe
pub enum Level { Trace, Debug, Info, Warn, Error }        // structured levels, not prints
pub struct Event { subsystem: &'static str, level: Level, kind: &'static str, fields: Fields, span: Option<SpanId> }
pub enum Measure { Counter(u64), Gauge(i64), Histogram { count: u64, sum_us: u64, max_us: u64 } }
pub trait Observer: Send + Sync + 'static { fn event(&self, &Event); fn measure(&self, &Measure); fn flush(&self); }  // Observable
pub trait Observable { fn observer(&self) -> Option<&dyn Observer>; }   // the per-crate seam
pub trait Health { fn health(&self) -> SubsystemHealth; }               // identity per subsystem
```
- **`Observable`** is the trait the user is asking to enforce: every crate's public types expose the
  observer handle (or accept an `Arc<dyn Observer>` at construction), so the seam is structural.
- **`Health`** carries identity: each crate contributes its own `SubsystemHealth` (store path/schema/
  size/last-write, scopes in force, counts, build id) and the daemon aggregates.
- Keep `ObsEvent` as a compatibility shim over `Event` for the existing 101 call sites, or migrate
  them — a decision for the first spec (migration is mechanical; the shim is cheaper).

### 3.3 The health payload (v2, per the directive)
```json
{"build":{"version":"…","git":"…","target":"x86_64-unknown-linux-musl"},
 "config":{"path":"…","sha256":"…"},
 "stores":[{"name":"store","path":"/var/lib/pii/store.redb","schema":3,"bytes":1056768,"last_write_ms":…},
           {"name":"memory","path":"…"}],
 "mounts":[{"kind":"workspace","root":"/srv/work","mode":"rw"},{"kind":"channels","root":"…"}],
 "sandbox":{"profile":"strict","enforced_by":"in-process"},   // states the LAYER honestly
 "counts":{"runs":…,"journal_events":…,"approvals_pending":…,"cron":…,"memory_docs":…},
 "subsystems":[{"name":"store","state":"ok"},{"name":"net","state":"ok"},{"name":"channels","state":"degraded","detail":"cursor stalled 41min"}]}
```

### 3.4 Enforcement (fail-closed, in the gate)
A new xtask check — `lint-observability` — wired as a **failing** gate step, in the shape of the two
existing ones:
1. every crate in `crates/` must depend on `pii-observe` (the contract's home);
2. every crate must either have ≥1 `observer`/`Event`/`Measure` use site, **or** carry a named
   exemption comment with a reason (`// observability: none — pure types`);
3. **no raw `eprintln!`/`println!`/`dbg!` in `src/` outside a named allowlist** (the 139 sites above
   are the initial allowlist; the sweep spec empties it crate by crate);
4. near-miss control: re-introducing a raw print, or dropping a crate's seam, must make the check exit
   non-zero (`peercred.rs`-style re-introduction test, as done for `lint-scratch`).

## 4. Adoption matrix (what each crate must emit)

| crate | first events/measures | identity it contributes to health |
|---|---|---|
| `pii-store` | open/close, write latency histogram, **persist errors** (already an event), table counts | store path, schema version, bytes, last write |
| `pii-graph` | node start/finish latency, batch width (exists), dead/join-unsatisfiable (exist), replay divergence | run counts, journal length |
| `pii-net` | dial/accept, TLS failures, egress denials, listener state | listeners (exists), bytes in/out |
| `pii-tools` | per-tool call latency, output-cap hits, **deny-list hits** | tool call counts |
| `pii-model` | provider in force, request latency, token counts, provider errors | provider + model in force |
| `pii-worker` | spawn/ready/restart/fail transitions, RSS | workers (exists) + restart counts |
| `pii-server` | RPC method latency histogram, approvals pending, scheduler wakeups | aggregates everything above |
| `pii-channels` | poll latency, cursor lag, send failures | channel cursors (exists) + lag |
| `pii-memory` | search latency, index size, consolidation outcomes | doc counts (exists), index bytes |
| `pii-cli`, `pii-tui`, `pii-web`, `pii-mcp`, `pii-evolve` | their own errors/progress **through the contract**, replacing 123 raw prints | client-side view only |
| `pii-abi`, `pii-proto`, `pii-uds`, `pii-http-fetch` | types/contract only (named exemption) | — |

## 5. How this lands in P0 + P1 (the phases the user wants next)

- **P0 (deployable)** needs the *transport* for observability: a log file with rotation (today logs go
  to stderr), `--help` for `pii daemon`, a release/static build decision, and a container/systemd unit
  that captures the structured stream. **Spec 99.**
- **P1 (operable)** needs the *contract and identity*: the `pii-observe` contract crate (spec 97),
  the enforcement lint it makes possible (spec 98), health v2 identity — store, mounts, version, counts
  (spec 99), the per-crate adoption sweep (spec 100), then backup/restore + store schema version +
  metrics endpoint + runbook (spec 102).

## 6. Pitfalls this project has already paid for (carry into the specs)

- **A metric can measure the wrong subject.** Our own track record (laya: a metric that swallowed the
  exit code and produced a 70× fake win) — every metric introduced here needs a *negative* case: the
  count must not move when the thing did not happen.
- **"Recorded" is not "in force".** A config value or a profile name in health is a claim; the payload
  must report the *resolved* value (path actually opened, scope actually granted) — the directive's
  whole point.
- **A check that passes while the path is broken is worse than none.** The enforcement step needs its
  re-introduction control (item 4 of §3.4), as `lint-scratch` has.
- **Do not put the lint on the command line for all crates** (the spec-77 lesson: a `-W` flag
  propagated to 19 files of debt). Enable at source, allow by name with a reason.

## 7. NOT VERIFIED

- Whether `pii-proto`/`pii-tui`/`pii-web` can take a `pii-observe` dependency without a cycle (a first
  reading says yes: it would depend on `serde`/`serde_json` only, so no upward edge exists).
- The cost of migrating the 101 existing observer call sites (not attempted).
- Whether the redb stores expose a schema/format version we can report today (the spec must measure
  this before promising the health field).
- Overhead of per-RPC latency histograms under load (no measurements exist).

---

## 8. Observability is the substrate of Pillar 4 (user directive, 2026-10-09)

The user's framing: *"observability is the basis of the self improvement loop pilar"*. That reorders the
priority of this work and sharpens the contract, and it is worth stating plainly why.

The original vision's Pillar 4 is *"skill extraction, procedural templates, trajectory logging for
future fine-tuning of a local ~9B model"*, with trajectories carrying `outcome.verifier` (a programmatic
pass/fail), `user_signal`, `escalation`, and **mandatory `redaction`**. None of that is built (052 §5.1
says ABSENT). But note what it is made of: **every element of it is an observation** — a run's nodes and
tools, their latency and outcome, the verifier's verdict, what the user then did, and what was redacted.
A self-improvement loop is, mechanically, *querying a corpus of structured observations and promoting
what worked*.

So the honest ordering is not "observability, then later self-improvement". It is:

1. **this contract** (spec 97) — but with the contract carrying what a trajectory needs: **correlation**
   (`run`, `session`, `span`), an **outcome/verdict** notion, and a **redaction** seam;
2. **the trajectory substrate** (spec 100) — a durable, append-only, per-run trajectory sink written
   through the same contract, with redaction applied at the sink (never at the call site, or it will be
   forgotten);
3. only then can Pillar 4's extraction/distillation be built *on data the engine already produces for
   its own health* — which is also the cheapest possible route to that pillar: it reuses the same
   events, the same sink, the same schema.

**Consequences for the contract and the specs:**

- `Event` must carry `run: Option<RunId>` / `session: Option<SessionId>` / `span: Option<SpanId>` —
  spec 97 as written has `span` only, so **spec 100 extends the contract with the correlation ids** and
  is the first consumer that proves they are sufficient.
- There must be an explicit **outcome** concept (the vision's `outcome.verifier`): the graph already
  reports a terminal (`success`/`failure` with a reason) — the trajectory record must carry it verbatim,
  not a re-derived summary.
- **Redaction belongs in the sink**, with a named policy, because “mandatory redaction” enforced by
  convention is exactly the class of rule this project has watched fail (0c/0d).
- The trajectory file is an **untracked, append-only artefact** (like the autoresearch ledger): it must
  never be a reason for the git tree to be dirty, and it must never be deleted as "cleanup" (the
  never-delete-on-failure lifecycle from the vision's skill design).
- This makes the self-improvement pillar **measurable before it is clever**: the first milestone is
  "N runs' trajectories are on disk, each with an outcome and a redaction record", not "a skill was
  extracted".

A caution that the project has already paid for: **an observability-driven loop inherits every metric
mistake into its training data.** The laya lesson (a metric that swallowed the exit code and produced a
70× fake win) is the exact failure mode for a corpus that later teaches a model. Every measure and
outcome field introduced for this must therefore have a negative case in its tests: *it must not move
when the thing did not happen, and a failed run must never record a success.*
