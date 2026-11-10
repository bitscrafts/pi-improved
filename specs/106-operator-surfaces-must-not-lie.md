# Spec: 106 — the operator surfaces must not lie (three measured defects)

**Project**: <repo>
**Phase**: P0/P1 close-out — the residuals spec 105 flagged, plus one I measured while investigating them.
**Origin**: all three are *proven*, not suspected. Evidence below was produced by the orchestrator on HEAD
(`e2a47c8`) with a scripted model and a scratch runtime dir.

## The three defects, with their evidence

**D1 — `pii run ls` is ALWAYS empty, and a nonsense status looks exactly like "no runs".**
`crates/pii-server/src/engine/dispatch.rs` (~line 261):
```rust
let runs = status.map_or_else(Vec::new, |st| engine.store.runs_by_status(st).unwrap_or_default());
```
No filter ⇒ `Vec::new()`. And on the CLI side an unrecognised value is swallowed by
`serde_json::from_value(...).unwrap_or(RunListParams { status: None, .. })` — which then takes the same
empty path. Measured:
```
run ls                    → rc=0  []
run ls --status all       → rc=0  []
run ls --status bogus     → rc=0  []
run ls --status succeeded → rc=0  [{"run_id":"r000000000001","status":"succeeded",…}]   ← the store had it all along
```
An operator's primary "what is happening?" command reports an idle system while runs exist, and
`--status bogus` is **indistinguishable from "there are no runs"** with exit 0. Fail-open on the surface an
operator trusts most.

**D2 — a SUSPENDED run has no visible approval.**
Measured with a `gated` graph (one `gate` node, `policy = "p"`):
```
run start gated → {"run_id":"r000000000001"}   rc=0
run ls --status suspended → [{"run_id":"r000000000001","status":"suspended","seq":5}]  ← it IS suspended
approve ls                → []     ← nothing to approve
health --json             → approvals_pending: 0
```
The path is `Event::RunSuspended → daemon::react → on_suspended → read_journal → find(Suspended{key ==
key}) → approvals::request(...)`. Both the journal lookup and the call itself fail silently: the lookup
`return`s with no log, and the call is `let _ = approvals::request(...)` (`daemon/mod.rs` ~353). Worse, the
id is derived from the POLICY name — `let approval_id = key.strip_prefix("approval:").unwrap_or(key)` —
so every run using policy `p` would collide on ONE approval row (`put_approval_if_absent` then returns
"already there" and the second run's request is swallowed).

**D3 — `health` omits a store the daemon opens.**
`timer.redb` exists on disk next to the other five, is stamped and is backed up, but `pii health --json`
lists `['main','memory','approval','heal','channel']` — five of six. The timer wheel lives inside the
scheduler, so nothing reaches `sys.status`.

## Requirements
R1. **`run.list` without a status returns every run** (say the order you choose — e.g. status then id, or
    newest first — and document it). `--status X` keeps filtering. Do not break the existing callers that
    pass a status (`subscribe`, the inline-measurement tests).
R2. **An unknown `--status` value is an ERROR**: non-zero exit and a message naming the accepted values
    (`running|suspended|succeeded|failed`). The server must not swallow a params parse failure into "no
    filter"; report it as a bad-params error. Paste `--status bogus` failing.
R3. **A suspended run MUST produce a visible, RUN-SCOPED pending approval.** After `run start gated`:
    `approve ls` lists it, `health --json` shows `approvals_pending >= 1`, and **two runs on the same
    policy produce TWO distinct approval rows** (the id must not be the policy name). Paste the two rows.
R4. **A suspension that cannot produce an approval is a finding, never a silent skip**: log/report it (an
    observability event through `pii-observe`, or a non-zero path), including when the journal lookup
    fails. No `let _ =` on that call.
R5. **`health` lists EVERY store the daemon opens**, `timer.redb` included, and a test asserts the reported
    set equals the set of store files the daemon actually creates on disk — so a store added later cannot
    be silently omitted. Mutation control: register the set from a hand-written list in the test and show
    the test FAILS when a store is left out.

## Exit Criteria
- [ ] `run ls` on a store with n runs prints n (paste; and paste that it was `[]` before)
- [ ] `run ls --status bogus` exits non-zero with a message naming the accepted values (paste the rc and text)
- [ ] `run ls --status suspended|succeeded` still work (paste)
- [ ] `run start gated` → `approve ls` shows the pending approval and `health` shows `approvals_pending >= 1`
      (paste before/after)
- [ ] two runs on the same policy → two distinct approval rows, with their ids (paste) — the mutation
      control is: make the id the policy name again and show the second row disappears
- [ ] a suspension whose approval cannot be created is reported, not swallowed (paste the evidence: the
      event/log line, or the failing path)
- [ ] `pii health --json` lists six stores including `timer`, and the set-equality test exists; removing a
      store from the reported set turns it RED (paste)
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed

## Guardrails
- Do NOT relax a test to make a new behaviour pass; the divergence tests must be able to fail.
- A status value the wire cannot express must be refused at the EDGE (CLI or param parse), not silently
  dropped.
- Emit through `pii-observe`; no new raw prints beyond the file's budget.
- Do not leave a daemon running (kill what you start; `pgrep -af 'pii[ ]daemon'` empty in the report).
- Scratch under `<scratch>/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
**Closed, and one of MY OWN diagnoses in this spec was WRONG — corrected here.**

**My D2 explanation was wrong.** I claimed the approval id was derived from the policy name
(`key.strip_prefix("approval:")` → `p`) and that runs would collide on one row. The worker proved
otherwise and is right: the gate already mints a RUN-SCOPED id (`approval:<blake3>`), and my reading came
from `engine/bus.rs` where `format!("approval:{policy}")` is a DISPLAY string, not the key. The real cause
was a **race**: `RunSuspended` is broadcast when the event is appended, BEFORE the write-behind journal
flush, so `on_suspended` read the journal too early, found nothing, and `return`ed with no log. Fixed in a
new `daemon/suspend.rs` by retrying the lookup (up to 1s) and reporting anything still unhandleable to the
observer AND stderr, with the `let _ =` removed. Lesson (mine): a defect can be real while the mechanism I
assert in a spec is invented — the spec's evidence was measured, its explanation was not.

**Reproduced by the orchestrator on the fixed tree (my own scratch, my own runs):**
```
D1  run ls                        → 3 runs: r…002 suspended, r…003 suspended, r…001 succeeded   (was [])
    run ls --status bogus          → rc=2  "error: run ls: unknown --status 'bogus'
                                            (accepted: running|suspended|succeeded|failed)"
    run ls --status suspended      → 2      run ls --status succeeded → 1
D2  pii approve ls                 → 2 rows            (was [])
    ids 5299f70e5b449cf1447f9c0b6b6ecbff (run r…003) and dd45234fe0f5d1066f6622b173ce96fd (run r…002)
    distinct: True  ← two runs, same policy, two approvals: the id is run-scoped
    health --json approvals_pending → 2                 (was 0 while a run waited for a human)
D3  health --json stores           → ['approval','channel','heal','main','memory','timer']
    on disk                        → approval channel heal memory store timer  ← sets now MATCH (six)
    daemons left running: 0
```
Worker's mutation controls (its report): policy-name-as-id → the integration test and
`approval_row_uses_the_key_id_not_the_policy` both FAILED, then restored; dropping `timer` from the
reported set → `store_set_check_fails_when_a_store_is_omitted` FAILS. Unreported-suspension paths are
covered by `missing_journal_entry_is_reported` and `unusable_approval_key_is_reported`.
Gates: `quality-gate.sh` ALL PASS (incl. `PASS: lint-observability`) · `fresh-gate --worktree` PASS ·
clippy `-D warnings` · `fmt --check`. Worker left scratch under `<scratch>/` per the no-deletion rule.
