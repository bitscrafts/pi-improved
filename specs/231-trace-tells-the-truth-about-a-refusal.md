# Spec: trace tells the truth about a refusal — the full reason, and the image's real status

## Overview
M0 run `r000000000210` on the host (2026-10-20) printed

    6 code coder 1+31 pid=- image=missing crit=- fail=Error: failed to open 2048 locks in /libpod_rootless_lock_1000: permission denie rc=-

The operative word is cut. The 80-character cut is deliberate on the human line (spec 224 R2), but the
daemon's trace reply carries nothing longer: `pii run trace <id> --reason <code>` is answered
`error: unknown flag --reason` / `usage: pii run trace <run_id> [--db <path>]` (rc 2), the local store
dump is unavailable while the daemon holds the store, and the daemon log's refusal lines name no run id
(measured 2026-10-20). So the only source a live verdict has is a display clip — the M0 script's reason
came from exactly that string.

The same line carries a second false status: `image=missing` for a node whose image probe never ran to a
verdict. The image is present (`podman image inspect --format {{.Digest}} localhost/pii-agent:1` prints
`sha256:2684dee9…` outside the sandbox, rc 0); the runtime refused at its own lock, and the node reported
the absence of an answer as the absence of an image.

## Requirements
R1. `pii run trace <id> --reason <node>` is answered through the daemon (the existing `Method::RunTrace`
    wire), returning the node's failure/refusal string exactly as journalled — byte for byte — and its
    length. The human display line keeps its 80-character cut.
R2. The trace line's image slot distinguishes three states: a digest (a probe answered), `missing` (the
    runtime said the image is absent), and `probe-failed` (the runtime could not answer; why is on the
    same line's `fail=`). A node refused before the runtime answered may not print `missing`.
R3. `scripts/m0-daemon-starts-a-container.sh` takes its verdict reason from the structured reply, and
    when the string it reads is at the clipping boundary it says so in the verdict line instead of
    presenting a truncated reason as the measurement.
R5. The the host re-measurement is recorded, not gated: with R1 in place, `pii run trace <id> --reason
    <node>` against the live daemon prints the reason ending in `permission denied`, and the doc quotes it.
R4. Controls: a 400-character reason survives the wire intact (asserted byte-for-byte); a genuinely
    absent image still prints `missing`; the display line still prints at most 80 characters.

## TDD Contract
- pii-cli `tests/trace_reason_is_not_clipped.rs`: a store fixture with a failed node whose reason is 400
  characters; `trace --reason` must print it in full and a 400 must not be 80. RED at HEAD: the flag does
  not parse (`unknown flag --reason`, rc 2).
- pii-cli `tests/trace_image_status.rs`: two fixtures — a refusal after a probe that answered (digest), a
  refusal at the runtime (`probe-failed`), and an absent image (`missing`, the control that must not
  disappear).
- pii-server: the trace reply builder passes the journalled string unclipped (unit test beside the
  existing trace reply tests).
- Live: M0's verdict on the host prints the reason ending in `permission denied`.

## Exit Criteria
- [ ] cargo test -p pii-cli --test trace_reason_is_not_clipped
- [ ] cargo test -p pii-cli --test trace_image_status
- [ ] cargo test -p pii-cli --lib
- [ ] grep -q 'probe-failed' crates/pii-cli/src/cmd_trace.rs
- [ ] grep -q 'permission denied' _docs/05-operations/M0-2026-10-20.md

## Guardrails
- Do NOT remove the 80-character cut from the human line, and do NOT widen it to 400.
- Do NOT require the store lock: the answer comes through the daemon, as the run's other queries do.
- Do NOT let `probe-failed` swallow the real `missing` case — the control in R4 is what keeps it honest.
