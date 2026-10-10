# Spec: 109 — no hang on the door, and no dead names in the vocabulary

**Project**: /projects/pi-improved
**Phase**: P3 close-out (the two things spec 108 left open, plus one defect it found by accident).

## Context — all three are measured, not suspected
1. **A client can hang FOREVER.** Spec 108's worker found it by accident: `pii_proto::client::Client::connect`
   never returns if the peer accepts the connection and closes it WITHOUT answering the hello. A hang is
   worse than a crash for an operator: no error, no exit code, no log — the caller waits. **Reproduce it
   first and paste the evidence (a timeout showing it did not return), then fix it.**
2. **Two names are registered and emitted NOWHERE**: `HEALTH_STATE_CHANGED` and `OBSERVE_EVENTS_DROPPED`.
   The name registry is the vocabulary the lint and dashboards read; a name nothing emits is a promise the
   code does not keep. Either emit it or remove it with the reason (and if removed, `ALL` must shrink —
   the completeness rule (R1e) already forces them to stay in `ALL` while the const exists).
3. **Three adequacy leftovers from spec 108's own report** (`_docs/06-research/054-coverage-and-adequacy-2026-10-10.md`):
   - the graph metrics (`graph.node.dead`, `graph.join.unsatisfiable`, `graph.batch.width`) never reach the
     daemon log (flooding risk) — the journal records them;
   - the memory search INSIDE an agent turn is unobserved **and swallows its error**;
   - `pii-http-fetch` has no dependents, so its events cannot fire; and `pii tui` still defaults to the
     `off` sandbox.

## Requirements
R1. **Bound the handshake.** `Client::connect` (and any peer-read on the way to a ready connection) must
    have a bounded wait with a clear error naming the endpoint and the elapsed bound. Requirements:
    - a DEFAULT bound that is short enough to be useful (say 5s) and overridable (name it);
    - a peer that closes without answering → **error**, not a hang (test);
    - a peer that stays silent → **error at the bound**, not a hang (test);
    - a stopwatch assertion: the failing connect returns within the bound + a small slack (paste the number);
    - the RED evidence first: show the hang (e.g. `timeout 10` killing the call) before the fix.
R2. **No dead names.** For each of `HEALTH_STATE_CHANGED` and `OBSERVE_EVENTS_DROPPED`: either emit it (with
    a test that can fail and a mutation control), or delete the const from `names.rs` — and if you delete it,
    say why nothing should emit it. A name in `ALL` that no code path can produce must not survive this spec.
R3. **The graph metrics reach an operator-visible surface without flooding.** Choose and implement: a
    counter/aggregate in the metrics surface (`pii metrics`), or a sampled/rate-limited log line, or an
    explicit decision that the journal IS the surface (then the runbook must say how to read them, and the
    report must say which query). Do not emit one log line per tick without a bound.
R4. **The agent-turn memory search stops swallowing its error**: emit what failed (through the contract,
    names registered) and make the failure visible to the caller — or state, with evidence, why swallowing
    is correct here. Silence is not an answer.
R5. **`pii-http-fetch` and the `pii tui` sandbox**: for the fetch crate, either wire it to a real consumer or
    give it a sanctioned exemption entry with a truthful reason (the exemption registry is bounded —
    adding a FIFTH crate means changing the sanctioned set, which must be justified in the report); for the
    TUI, decide the sandbox default and make it consistent with the daemon (a test).
R6. **Every new assertion needs a mutation control** (paste one per requirement, or state why none exists).

R7. **Make the transport failures VISIBLE (the payoff of `_docs/06-research/055-exemption-audit-2026-10-10.md`).**
    The audit found that two exempt crates own real I/O — `pii-proto`'s `client.rs` (the handshake, the
    `hello` skew, `send_line`) and `pii-uds`'s `client_connect` (*"the ONE `UnixStream::connect` site outside
    the daemon"*) — so the failures a user meets first are unobservable AND `ClientError` has no consumer
    that reports it. Requirement: emit those failures through `pii-observe` (endpoint, the bound that
    expired, the skew detail) as NAMED events registered in `names.rs`/`ALL`, and surface them where a user
    sees them (the CLI/TUI error path), not only in a log.
R8. **Tighten the exemption rule so "nothing to observe" is mechanically decidable.** A crate-wide exemption
    is INVALID if any module of that crate imports `tokio`, `std::net`, `std::io` or `std::process`: such a
    module owns failures. The lint must (a) scan the exempt crate's `src/**/*.rs` for those imports, (b) fail
    the gate naming the crate and the module, (c) accept a module-scoped entry (`<crate>:<module>  # reason`)
    for the dormant part with a reason, and (d) keep the existing sanctioned-set and dead-exemption rules.
    Expected effect from the audit: the sanctioned set shrinks to `pii-observe` (permanent: it is the
    contract) and `pii-abi` (no I/O — verified: 9 modules of types/traits plus pure classifiers), while
    `pii-proto` and `pii-uds` stop being exempt at crate level. If you disagree with a verdict, say so with
    evidence instead of keeping the entry.
    **Near-miss control (mandatory):** an exempt crate with only types/consts and none of those imports must
    still be ACCEPTED (that is `pii-abi`), and re-introducing e.g. `use tokio::net::UnixStream;` into an
    exempt crate must turn the lint RED. Paste both.

## Exit Criteria
- [ ] the hang is reproduced BEFORE the fix (paste the timeout evidence) and the failing connect now returns
      within its bound with a clear error (paste both)
- [ ] all three tests of R1 pass and the stopwatch numbers are pasted
- [ ] each dead name is either emitted (test + mutation) or deleted, with the reason stated (paste
      `grep -rn` showing no code path emits it, or the test showing it does)
- [ ] the graph metrics have a bounded, documented surface (paste the query or the metrics output)
- [ ] the agent-turn memory search reports its failure (paste the test), or the report justifies silence
- [ ] R7: the transport failures are emitted as NAMED events (paste the event line with the endpoint and
      the bound) and surfaced in the CLI/TUI error path; the audit's verdicts (095 doc) are honoured or
      contested with evidence
- [ ] R8: the lint fails on a crate-wide exemption covering a module with I/O (paste), accepts the
      types-only crate (paste the near-miss), and the exemption registry reflects the new set
- [ ] `pii-http-fetch` and the TUI sandbox are decided and tested; if the sanctioned set changed, the reason
      is in the report (and the registry's four-entry rule was updated deliberately, not quietly)
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability` and `PASS: coverage-floor`;
      `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed and every new `names.rs` const

## Guardrails
- **Do NOT fix the hang by making the caller swallow an error.** The requirement is a bounded, REPORTED
  failure.
- Do not delete a name to satisfy R2 if something SHOULD emit it — emitting is the better answer; deleting
  silently drops a capability.
- Coverage must stay at or above the floor (76.99% measured, floor 74.0 in `coverage-floor.txt`); if your
  changes move it, report both numbers.
- Emit through `pii-observe`; no new raw prints beyond the file's budget.
- Scratch under `/projects/_tmp/`; the disk is at ~97% used, so keep exports small and delete your own.
  Do not leave a daemon running.
- Do NOT commit; leave the tree for the orchestrator.
