# Pii — Orchestration Review Resumption (spec-38 verification + gate state)

**Date:** 2026-09-16 · **HEAD:** `533d833` (branch `master`) · **Author:** Claude Opus 5 (orchestrator, resumed cron session)
**Status:** analysis/verification document only — **nothing written to `specs/`, nothing committed.** Read-only verification plus this one research doc. Deliverables are saved for manual review per protocol.

This document resumes the opus orchestration session that produced
`_docs/06-research/026-codebase-analysis-and-phase5-plan.md` (2026-09-15). **026 remains the
authoritative full codebase map + debt/risk assessment (Part 1–Part 2) and Phase 5 plan
(Part 3); it is complete as of HEAD and is not re-litigated here.** This 027 adds the
post-026 spec-38 attempt, mechanically verified this session, and the resulting
next-phase state. All load-bearing claims below were verified by executing on the tree at
HEAD — not taken from handoff prose.

---

## 1. Session state delta since 026

| Item | 026-era claim | Verified this session (HEAD) |
|---|---|---|
| Quality gate | RED (E0463 doctest in `pii-abi`) | **still RED**. `_tmp/orch-analysis-20260915/quality-gate.log` tail confirms `FAIL: test` / `quality-gate: FAILED` / `exit=1` — `E0463 can't find crate for schemars/serde_json` in `crates/pii-abi/src/{journal,http}.rs` doctests. Root harness cause unchanged: the unified `cargo test --workspace` doctest step links a stale rlib after the `clippy --all-targets` feature-unification build. HEAD unchanged (`533d833`). |
| `pii-http-fetch` crate | not in workspace (026-Table 1.1 counted 18 members + xtask) | **landed untracked**, `crates/pii-http-fetch/` (581 lines: `lib.rs` 109, `fetch.rs` 144, `audit.rs` 105, `tests/fetch.rs` 223). Now member #19 in `Cargo.toml`. One intended side-effect of the spec-38 dispatch. |
| `examples/*.toml` fixtures | present | workhorse deleted 437 lines across 11 unrelated graph fixtures mid-run; **reverted clean via `git checkout -- examples/`** — no fixture damage remains. |
| `.pidag/config.toml` `[sdd]` | `max_iterations = 4` | **`max_iterations = 2`** (user directive, "2 tries then escalate, not 4"). BUT `ModelsConfig::models_for_iter` free→paid escalation still keys on `iteration >= 3` → with only 2 iterations the paid fallback never triggers inside a spec's loop (open inconsistency; see §3 action 5). |

## 2. Spec-38 verification (the interrupted run — root cause isolated, code is green)

Spec `specs/38-http-fetch-tool-via-pii-net.md` ("HTTP fetch via the pii-net chokepoint",
Track A / Phase 5.1 from 026 §5.1 / the orphaned original-35 re-queue) was dispatched via
`pidag sdd` against `ollama-cloud/glm-5.3-flash:cloud`. The run FAILED. **Verified this
session:**

1. **The implementation is sound.** `cargo test -p pii-http-fetch` → **11/11 pass**
   (5 unit + 6 integration) in ~2 s. Integration tests prove:
   - `fetch_basic_get` — GET returns body **and** lands in the audit log (`FEED: GET …`).
   - `fetch_https_routes_through_chokepoint` — HTTPS goes through a `NetService` fake.
   - `fetch_post_blocked` / `fetch_with_headers_blocked` — refusals audited as
     `FEED: DENIED …`, nothing reaches the chokepoint.
   - `audit_file_sink_is_deterministic` — a **file-backed** `AuditLog` matches the
     in-memory bytes byte-for-byte.
2. **The gate (quality-gate-{1..4}) is irrelevant to this failure** — it fails identically
   at HEAD on `pii-abi` doctests (E0463) before `pii-http-fetch` is ever exercised.
3. **The actual cause of "FAILED" is a spec authoring bug — exit-criterion R2 is
   structurally unsatisfiable.** `specs/38:36` reads
   `grep -qE '^FEED: GET ' /tmp/pii-net-audit.log`. There is **no such file and no code
   path that writes it**: `pii_http_fetch::audit::AuditSink` is either `File(PathBuf)` (a
   caller-supplied path) or `Memory(Arc<Mutex<Vec<String>>>)` (`audit.rs:15-20`); nothing in
   the tree constructs a `File(…/tmp/pii-net-audit.log)` sink or any `/tmp` sink. R2
   therefore fails no matter how good the code is. It also violates the workspace
   convention (tests/artifacts never in `/tmp`). **This is a planner fix, not a workhorse
   defect** — the workhorse did what the spec asked and exercised the loop the expected 4
   times (the dispatched DAG at `.pidag/38-*.json` still encodes the OLD 4-iteration
   policy, pre-dating today's `max_iterations=2` change; 4 ≡ the correct attempt count for
   the dispatch that actually ran).
4. **Out-of-scope damage and no-commit are the protocol-relevant facts.** The run has been
   left uncommitted for review (intended). The only verified out-of-scope side effect
   (fixture deletion) was already reverted. `specs/38` is uncommitted and un-owned → must
   get a planner owner (026 F-03 pattern repeats).

### Recommended R2 fix (planner decides; not applied here)

R2 should reference a **real, committed, in-tree** artifact. The most faithful criterion
(existing test already does this): build an `HttpFetchTool` over a `pii_net::NetService`
test fake + `AuditLog::to_file(<_tmp audit path>)`, run one GET, and assert a line
starting `FEED: GET ` — i.e. promote `tests/fetch.rs::audit_file_sink_is_deterministic` /
`fetch_basic_get` into the exit criterion via a small script under `scripts/` (not `/tmp`),
or fold file-audit into the crate contract so the criterion greps a real path. The exact
text's-worth of R1 already proves the code; R2 only needs to follow the "no `/tmp`"
convention and name a path a node actually writes. A `scripts/spec-refs-check.sh`
(checking every backticked path under a spec's Exit Criteria exists — recommended in 026
§5.0.6) would have caught this before dispatch.

## 3. Next actions (in order — same ordering as 026 §3, unchanged except the R2 fix gate)

1. **[planner] Adopt/fix `specs/38` R2** to a real in-tree audit path (or drop it with a
   documented rationale), adopting the working-tree crate + crate as part of the committed
   spec — giving the unowned working-tree changes (`specs/38`, `crates/pii-http-fetch/`,
   `specs/33` edit, doc 026/027, `HANDOFF.md`) a single owner.
2. **[planner] Re-run spec 38 to a genuine green** via `pidag sdd` (now with
   `max_iterations = 2`), gate red at HEAD notwithstanding — note the gate will still block
   the `quality-gate-*` node until 5.0.1.
3. **[workhorse via pidag] Phase 5.0.1 — make `quality-gate.sh` literally green** (fix the
   E0463 feature-unification doctest defect, or make the split invocation the script's
   SSOT). This is the real prerequisite for **every** later spec's validation (026 F-01,
   §5.0.1). Acceptance: `./quality-gate.sh; echo $?` → `0` three consecutive runs, one
   after `cargo clean`.
4. Then the remainder of Phase 5.0 (D1 TLS lease, D2 channel scope, D3/D6 session
   persistence, 5.0.5 feature flag), Track A (A2–A5), etc. — **unchanged from 026 Part 3.**
5. **[planner/pidag] Reconcile `max_iterations=2` vs the `iteration >= 3` paid escalation**
   (config/`models_for_iter` mismatch); today a spec with a free-model-only loop exhausts
   its 2 iterations without ever attempting the paid fallback. Decide: lower the paid
   threshold to `>=2`, or drop `free→paid` escalation from the description of intent.
6. **[operator] Disk hygiene (026 F-21)** once back at a normal gate-passing cadence: `mv`
   (never delete) `pidag.redb.stale-*` and oversized `*.events.jsonl` to `_tmp/_trash/`.

**Do-not-commit scope of this session:** nothing below the handoff/tree diff was committed;
all of the above is left for manual review, exactly as the original orchestration session
was configured.

---

## Appendix — guardrail compliance & method

- `specs/` **not written or edited** this session (R2 fix above is a *recommendation*, left
  to the planner).
- All work foreground, read-only where possible; the only side-effectful command was
  `cargo test -p pii-http-fetch` (compiles + runs the crate's tests; no source writes).
- Nothing committed; no gate/pidag/pipeline run; no files deleted.
- Files written: this document only.
- Evidence: `crates/pii-http-fetch/**` read in full; `.pidag/38-*.json` read; `.pidag/config.toml`; `.gitignore`; `Cargo.toml` workspace members; `_tmp/orch-analysis-20260915/quality-gate.log` tail; `git rev-parse/status`.
- 026 §Appendix A.1 assumptions remain in force for the review portions referenced here.