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
