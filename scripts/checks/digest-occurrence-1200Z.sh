#!/usr/bin/env bash
# digest-occurrence-1200Z.sh — observe the 2026-10-19 12:00Z digest occurrence and write the evidence.
#
# 070 §1 step 2 is the only reading that earns a production verdict: at the occurrence, new digest runs
# == 1, new channel.posted == 1, channel.post_refused == 0, and at +15 min still 1. The numbers come from
# the STORE (since_deploy + the run set), not from a process-local counter (070 §5).
#
# Rule of this harness: it INJECTS nothing — the occurrence fires by itself — and it only reads. It writes
# one evidence file, incrementally, so a kill mid-wait still leaves what was seen.
set -uo pipefail
cd /projects/pi-improved || exit 1
PII=/root/.local/bin/pii
export PII_RUNTIME_DIR=/run/user/0/pii
# The evidence is written under _tmp while the observation runs: an untracked file in _docs would sit in a
# dirty tree for hours and the autonomy loop would sweep it into a "leftover" commit mid-write (cycle.sh
# treats a dirty tree that way by design). Copy it to _docs/05-operations/ and commit it once the verdict
# is written.
OUT=_tmp/occ-watch/DIGEST-OCCURRENCE-2026-10-19-1200Z.md
REV=$("$PII" health --json 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin).get("since_deploy",{}).get("revision","?"))')
say() { printf '%s\n' "$*" >> "$OUT"; }
snap() { # snap <label> — health since_deploy + the full run list, verbatim
  local label=$1 ts
  ts=$(date -u '+%H:%M:%SZ')
  say "## snapshot $label at $ts"
  say '```json'
  "$PII" health --json 2>/dev/null | python3 -c 'import sys,json; print(json.dumps(json.load(sys.stdin).get("since_deploy",{}), sort_keys=True))' >> "$OUT"
  "$PII" run ls 2>/dev/null >> "$OUT"
  say '```'
}
say "# The 12:00Z digest occurrence, observed (2026-10-19)"
say ""
say "Revision under observation: \`$REV\`. The armed run before the occurrence: r000000004353 (verified in the"
say "timer wheel for 1792411200000 = 12:00:00Z on 2026-10-19). The harness injects nothing; the occurrence fires"
say "by itself. Passive observation only — \`pii health\` and \`pii run ls\` are read-only RPCs."
say ""

# --- wait for 11:59:30Z, then the pre-occurrence snapshot ---
while [ "$(date -u +%s)" -lt "$(date -u -d '2026-10-19 11:59:30' +%s)" ]; do sleep 20; done
snap "A (pre-occurrence, 11:59:30Z)"
A_RUNS=$("$PII" run ls 2>/dev/null)
sedn() { "$PII" health --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('since_deploy',{}).get('$1'))"; }
A_POSTED=$(sedn posted); A_REFUSED=$(sedn post_refused); A_ARMED=$(sedn armed)
say ''
say "before: posted=$A_POSTED post_refused=$A_REFUSED armed=$A_ARMED"

# --- the occurrence: 12:00:00Z; first after-snapshot at 12:01:30Z ---
while [ "$(date -u +%s)" -lt "$(date -u -d '2026-10-19 12:01:30' +%s)" ]; do sleep 10; done
snap "B (+90s)"

# --- +6 min and +16 min: the "and at 09:05 still 1" half of the criterion ---
while [ "$(date -u +%s)" -lt "$(date -u -d '2026-10-19 12:06:00' +%s)" ]; do sleep 10; done
snap "C (+6 min)"
while [ "$(date -u +%s)" -lt "$(date -u -d '2026-10-19 12:16:00' +%s)" ]; do sleep 10; done
snap "D (+16 min)"
D_POSTED=$(sedn posted); D_REFUSED=$(sedn post_refused); D_ARMED=$(sedn armed)
say ''
say "after: posted=$D_POSTED post_refused=$D_REFUSED armed=$D_ARMED"

# --- the verdict, computed from the run lists (set arithmetic, not counters) ---
# The run list is ~270 KB of JSON: passing it as one argv exceeded ARG_MAX on 2026-10-19 and the verdict step
# died with "Argument list too long" while every snapshot was already on disk. Hand it over as a file.
printf '%s' "$A_RUNS" > "$OUT.runsA.json"
python3 - "$OUT" "$OUT.runsA.json" "$A_POSTED" "$A_REFUSED" "$A_ARMED" "$D_POSTED" "$D_REFUSED" "$D_ARMED" <<'PY' >> "$OUT"
import json, sys
out = sys.argv[1]
before = open(sys.argv[2]).read()
posted0, ref0, armed0, posted1, ref1, armed1 = sys.argv[3:]

def rows(raw):
    raw = raw.strip()
    if not raw:
        return []
    d = json.loads(raw)
    return d if isinstance(d, list) else d.get("runs", [])

text = open(out).read()

def block_after(label):
    i = text.find(f"## snapshot {label}")
    if i < 0:
        return None
    k = text.find("```json", i)
    j = text.find("```", k + 7)
    return text[k + 7:j]

def runs_at(label):
    b = block_after(label)
    if b is None:
        return None
    cand = [l for l in b.splitlines() if l.strip().startswith("[")]
    if not cand:
        return None
    return {r.get("run_id"): r.get("status") for r in rows(cand[-1])}

print("\n## verdict")
base = {r.get("run_id"): r.get("status") for r in rows(before)}
seen = {}
for label in ("B", "C", "D"):
    seen[label] = runs_at(label)
    if seen[label] is None:
        print(f"\nVERDICT: INCOMPLETE — snapshot {label} has no run list")
        raise SystemExit(0)

def tally(label):
    new = {k: v for k, v in seen[label].items() if k not in base}
    term = {k: v for k, v in new.items() if v in ("succeeded", "failed")}
    susp = {k: v for k, v in new.items() if v == "suspended"}
    run = {k: v for k, v in new.items() if v == "running"}
    # A FIRED run does not appear as a new row: the store updates the armed run IN PLACE when its timer fires
    # (measured 2026-10-19: r000000004353 suspended at 11:59 -> succeeded at 12:01, same row). So "fired" is a
    # STATUS TRANSITION out of suspended, and the only legitimately new row is the next occurrence's re-arm.
    fired = {k: v for k, v in seen[label].items() if v in ("succeeded", "failed") and base.get(k) == "suspended"}
    return new, term, susp, run, fired

for label in ("B", "C", "D"):
    new, term, susp, running, fired = tally(label)
    print(f"\n{label}: fired={sorted(fired)} new rows={len(new)} terminal={len(term)} suspended(re-arm)={len(susp)} running={len(running)}")
    for k, v in sorted(new.items()):
        print(f"  {k}  {v}")

b_new, b_term, b_susp, b_run, b_fired = tally("B")
d_new, d_term, d_susp, d_run, d_fired = tally("D")
c_fired, c_term = tally("C")[4], tally("C")[1]
print(f"\nstore counters: posted {posted0} -> {posted1} (delta {int(posted1 or 0) - int(posted0 or 0)}); "
      f"post_refused {ref0} -> {ref1} (delta {int(ref1 or 0) - int(ref0 or 0)}); armed {armed0} -> {armed1}")

checks = {
    "exactly one digest run FIRED (one transition out of suspended)": len(b_fired) == 1,
    "no second run appeared by +6 min": len(c_fired) == 1 and len(c_term) == 0,
    "no second run appeared by +16 min": len(d_fired) == 1 and len(d_term) == 0,
    "armed count unchanged at 1": str(armed1) == "1",
    "exactly one new post (posted +1)": int(posted1 or 0) - int(posted0 or 0) == 1,
    "no refused post (post_refused +0)": int(ref1 or 0) - int(ref0 or 0) == 0,
}
print("\n| check | result |")
print("|---|---|")
for k, v in checks.items():
    print(f"| {k} | {'PASS' if v else 'FAIL'} |")
print("\n" + ("VERDICT: 070 §1 STEP 2 PASS — one digest run, one post, no refusal, the armed run back to 1"
              if all(checks.values()) else
              "VERDICT: 070 §1 STEP 2 NOT MET — the failing rows above are the finding, not a harness error"))
print("\nWhat this does and does not earn: step 2 alone is one occurrence. The verdict 070 names needs step 6")
print("as well — seven days of exactly one digest per day with no unplanned starts, counted from this store.")
PY
