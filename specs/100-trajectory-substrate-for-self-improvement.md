# Spec: 100 — the trajectory substrate: the first brick of the self-improvement pillar

**Project**: /projects/pi-improved
**Phase**: P0/P1, observability — user directive (2026-10-09): *"observability is the basis of the self
improvement loop pilar"*. Rationale: `_docs/06-research/053-…md` §8. This spec extends the contract
with CORRELATION and adds the durable trajectory sink; it does not build skill extraction.

## Context
Pillar 4 (`001-pii-vision.md`) is skills + procedural templates + trajectory logging for later
fine-tuning, with `outcome.verifier`, `user_signal`, `escalation` and mandatory `redaction` — and it is
UNBUILT (052 §5.1). Every element of it is an observation, so the cheapest route is to make the
existing observability stream *sufficient* as a trajectory corpus. Spec 97 landed the contract
(`Event { subsystem, level, kind, fields, span }`) but **without run/session correlation**, which a
trajectory requires.

## Requirements
R1. **Extend `pii-observe`** with correlation and outcome (additive, serde-stable):
    - `Event` gains `run: Option<RunId>`, `session: Option<SessionId>` (string newtypes), keeping `span`.
    - An explicit outcome type mirroring the engine's terminal: `Outcome { Verdict, reason, detail }`
      with `Verdict { Success, Failure, Cancelled, Suspended }`, carried by a dedicated event kind
      (`run.outcome`) — the record must carry the terminal **verbatim**, not a re-derived summary.
    - `Redaction` — a policy trait (`fn redact(&self, field: &str, value: &str) -> Redaction`) plus a
      `Redaction::Keep | Drop | Replace(String)` result and a `NoRedaction` default.
R2. **The trajectory sink** (`pii-observe` types; the file-writing implementation may live in
    `pii-core`/`pii-server` where the runtime lives — say which you chose and why):
    - append-only, one JSON object per line, one file per run at a **caller-supplied root**
      (`<root>/trajectories/<run_id>.jsonl`), created on first event of that run;
    - takes an `Arc<dyn Redaction>` and applies it to every field **at the sink** (never at the call
      site) — including `fields` keys and the outcome detail;
    - never deletes or truncates a trajectory; a write failure is reported as an observability event
      (`PersistError`-style) and never panics or aborts the run;
    - bounded: a per-run cap (bytes or events) with a `trajectory.truncated` event when hit, so a
      runaway run cannot fill the disk.
R3. **Wire it** so a real run produces a trajectory today: `pii -p` and `pii graph run` (and the daemon's
    run engine) attach the sink, honouring an env/config switch
    (`PII_TRAJECTORY_DIR`, default OFF so existing behaviour and tests are unchanged unless opted in),
    and the daemon's config `[core]` gains an optional `trajectory_dir`.
R4. **Prove it end-to-end, with the numbers**: run a real
    `pii -p "…" --model ollama:<a small local model>` with `PII_TRAJECTORY_DIR` set, and paste:
    - the resulting file path and line count;
    - the `run.outcome` line verbatim;
    - the line count WITH and WITHOUT redaction exercised (a test that redacts a field, showing the
      sink dropped/replaced it while the original event was unaffected);
    - your test must also assert a FAILED run records `Verdict::Failure` — **a failed run must never
      record success.**
R5. **A negative case per new measure (mandatory)**: for the outcome event and the truncation event,
    a test that shows the event does NOT appear when the thing did not happen (a successful run emits no
    failure outcome; a short run emits no truncation).
R6. **BOUND THE EXEMPTION MECHANISM (orchestrator finding, 2026-10-09):** today an exemption is a
    comment inside the crate's own source (`const EXEMPT_MARK: &str = "// observability: none"`, parsed by
    `xtask/src/lint_observability.rs`), so any run can exempt any crate by writing one line — spec 98's
    lint went green exactly that way for 19 crates. Move the exemption set OUT of the crate sources into
    a checked-in registry `xtask/observability-exemptions.txt`, one `<crate>  # reason` per line, and
    change the lint to read it. It must then:
    - fail when a crate is exempt in the registry but ALSO carries a real seam (a dead exemption must not
      linger) and when a crate has neither a seam nor a registry entry (unchanged R1a/R1b);
    - report the registry size in its summary line, so an added exemption is a visible, reviewable diff
      in ONE file rather than a line hidden in a crate;
    - keep the sanctioned set minimal and named: `pii-abi`, `pii-proto`, `pii-uds`, `pii-observe` (pure
      types / the contract itself). Any entry beyond that set is a finding, not a pass.
    Remove the in-source `// observability: none` lines from every crate as part of this.
R7. **AN ADEQUACY FLOOR, not just presence:** for every crate that is NOT exempt, add ONE test that
    asserts a specific event or measure from that crate is emitted with the expected name (use
    `pii_observe::names` consts, not literals) — e.g. `pii-store` asserting a persist-error event,
    `pii-net` an egress denial, `pii-tools` a deny-list hit. The report must include a table
    `crate → test name → the name asserted`, and the tests must actually run (paste the run). A seam that
    nothing ever exercises is the same defect class this repo keeps finding: the check must be able to
    fail.
R8. Do NOT build skill extraction, distillation, or any learning loop in this spec — this spec ends at
    "trajectories are on disk, correlated, redacted, with verbatim outcomes". Any temptation to infer
    from them belongs to a later spec. `specs/` stays read-only for you.

## Exit Criteria
- [ ] **the placeholder debt is GONE**: `grep -rc 'pending the spec 100 adoption sweep' crates/ | grep -v ':0'` is empty, and `cargo run -q -p xtask -- lint-observability` reports an exempt-crate count equal to the sanctioned set only (`pii-abi`, `pii-proto`, `pii-uds`, `pii-observe` — each with the reason for being exempt, or a real seam if it emits)
- [ ] **every other crate has a real seam**: the lint reports 0 crates exempt outside the sanctioned set in `xtask/observability-exemptions.txt`, i.e. R1b passes on actual `pii_observe::` use; `grep -rn '// observability: none' crates/` is EMPTY (the mechanism moved out of the sources)
- [ ] **the exemption is bounded**: with a 5th crate added to the registry, the lint exits non-zero (control, pasted); with a listed crate given a real seam, the lint exits non-zero as a dead exemption (control, pasted)
- [ ] **the adequacy floor**: one passing test per non-exempt crate asserting a named event/measure from that crate, with the `crate → test → name` table in the report
- [ ] the raw-print allowlist shrank: `wc -l < xtask/observability-allowlist.txt` is reported before/after, and every remaining entry has a reason on the line (list the remaining count; 0 is the goal but a named few is acceptable if each is justified)
- [ ] `cargo test -p pii-observe` exits 0, including: serde round-trip of the extended `Event`, the redaction matrix (Keep/Drop/Replace), the truncation cap, and the "failed run records Failure" case
- [ ] an end-to-end run with `PII_TRAJECTORY_DIR` set produces `<root>/trajectories/<run_id>.jsonl`; paste the path, `wc -l`, and the `run.outcome` line
- [ ] a run WITHOUT the env var produces no trajectory file (and no behaviour change to stdout ndjson)
- [ ] with redaction configured, a chosen field is absent/replaced in the trajectory while the same field is intact in the stdout stream (paste both)
- [ ] `cargo test --workspace --lib --bins --tests` exits 0; `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0; `lint-files` clean
- [ ] the report lists every file changed and names the exact env var and config key that enable the sink

## Guardrails
- REDACTION AT THE SINK, never at the call site. A policy that can be forgotten is not a policy.
- The trajectory root must default OFF; no existing test may start writing files.
- Never delete, truncate or rewrite a trajectory file — append only, and report failures as events.
- Do NOT put secret material into the contract types, and do NOT log env values or tokens.
- Do NOT change the wire types in `pii-proto` in this spec.
- Do NOT edit any file under `specs/`.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.
