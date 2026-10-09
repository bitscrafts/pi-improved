#!/usr/bin/env bash
# run-rereview-resume.sh — finish the 044 re-review: resume the reviewer session and make it
# EMIT THE REPORT as its answer text.
#
# WHY THIS EXISTS (measured 2026-10-09): the first 044 dispatch (run-rereview-opus.sh) did real
# work — gates run, three findings established — but then spawned three BACKGROUND subagents and
# ended the run while waiting on them, so the CLI's final result object was a subagent
# task-notification and NO report was produced. Two lessons, both encoded here:
#   1. the brief must FORBID background subagents (a print-mode run has no turn to wait in);
#   2. the dispatch must stream (stream-json), not buffer — the answer must survive a mid-run exit.
#
#   bash scripts/run-rereview-resume.sh [extra-prompt-file]
#
# Exit: 0 ran · 2 window shut (nothing spent) · 1 run/filing/validity failure.
set -uo pipefail
export IS_SANDBOX="${IS_SANDBOX:-1}"

REPO=/projects/pi-improved
DOCS="$REPO/_docs/06-research"
WORK=/projects/_tmp/pii-rereview
REVIEWER_SESSION=ef17e6d6-1741-42d6-87c6-16ffc775d231
MODEL="${MODEL:-claude-opus-5-5}"
EFFORT_LEVEL="${EFFORT:-high}"
MAX_TURNS="${MAX_TURNS:-120}"
CLAUDE=/root/.local/bin/claude
RUNNER=/root/.hermes/skills/research/opus-review/scripts
AMEM_URL="${AMEM_URL:-http://10.10.10.217:7420}"
DEST_REL="_docs/06-research/044-opus-rereview-$(date -u +%Y-%m-%d).md"
POINTER="$DOCS/030-OPUS-START-HERE.md"
INDEX="$REPO/_docs/INDEX.md"
EXTRA="${1:-}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"; LOG="$WORK/resume-$TS.stream.jsonl"; ERRL="$WORK/resume-$TS.stderr"
mkdir -p "$WORK"
say() { printf '%s\n' "$*"; }

PROMPT="$(cat <<'EOF'
Produce the FINAL RE-REVIEW REPORT now, as your answer text. Nothing else.

RULES (hard):
- Do NOT spawn subagents / Task / background agents of any kind. A print-mode run has no turn in
  which to wait for them; last time that cost the whole run and produced no report.
- Do NOT edit, create or delete any file in the repo. Everything you need is already in this
  session's history: your own reads, the gates you ran, and your subagents' returned findings.
- Mark every claim: (C-run) you ran it, (C-read) you read it, (I) inferred. If something is
  unverified, put it in a NOT VERIFIED section — do not guess.

The orchestrator has INDEPENDENTLY reproduced two of the findings your run reported, so you may
cite them as confirmed (state them as (C-run by orchestrator, reproduced)):
 1. reachability-check can still be satisfied by a test-only reference: in a scratch export of
    the current tree with the real caller muted and a `#[cfg(test)] mod review_probe { fn f() {
    let _ = pii_model::resolve_with_transport("x", todo!()); } }` added to
    crates/pii-server/src/lib.rs, `cargo run -q -p xtask -- reachability-check` exits 0 (passes).
    With the caller removed and NO cfg(test) stand-in it exits 1. So spec 72's cfg(test) skipping
    does not cover this module shape.
 2. Three `pii-memory` tests are non-hermetic and fail on a fresh tree:
    crates/pii-memory/tests/lanes.rs:123, :158, :188 open hardcoded
    `concat!(CARGO_MANIFEST_DIR, "/_tmp/phase3/...")` paths and never call the file's own
    `tmp_root()` (which does `create_dir_all(root.join("phase3"))`). In the developer tree that
    directory exists, so the gate passes; in a scratch export the same three tests fail
    (`degrades_honestly_without_embedder`, `graph_lane_recalls_doc_via_shared_concept`,
    `memory_hit_carries_doc_text`) with "No such file or directory". Decide and state whether
    spec 74's hermetic-consolidate change is the root cause or merely the change that removed the
    accidental crutch that used to create it.

REPORT SHAPE (markdown, the whole answer):
1. Five-line verdict: which of 043's ranked candidates actually landed, which claims held, which did not.
2. Per-item findings with file:line and the label. Cover batches A-D as briefed (including the E0463
   `doctest = false` question: root cause or masking?), the two orchestrator-accepted claims, and which
   of the new tests you can make go red by mutation vs which are decoration.
3. NEW defects (including the two confirmed above, and anything you found beyond them).
4. NOT VERIFIED list.
5. A new RANKED candidate list, each with file:line and a pre-registered exit criterion a validator can run.
6. The single next most valuable change.

Length: thorough but not padded. This is the artefact that gets filed as 044; write it to be read.
EOF
)"
[ -n "$EXTRA" ] && PROMPT="$PROMPT

$(cat "$EXTRA")"

say "== PII RE-REVIEW (RESUME, report-only) — $TS"
say "   model: $MODEL effort=$EFFORT_LEVEL turns<=$MAX_TURNS  session: $REVIEWER_SESSION"
say "   tree : $(git -C "$REPO" rev-parse --short HEAD) / $(git -C "$REPO" status --porcelain | wc -l) changed"
[ "$(git -C "$REPO" status --porcelain | wc -l)" = "0" ] || { say "!! dirty tree — refusing"; git -C "$REPO" status --porcelain | sed 's/^/     /'; exit 1; }

if [ -n "${DRY_RUN:-}" ]; then say "DRY RUN — would stream-dispatch the resume prompt above."; exit 0; fi

say ""; say "-- probe"
probe="$(timeout 180 "$CLAUDE" -p "Reply with exactly: PONG55" --model "$MODEL" --max-turns 1 --output-format json 2>&1 || true)"
printf '%s' "$probe" | grep -q PONG55 || { say "   NOT OPEN (nothing spent):"; printf '%s\n' "$probe" | tail -4 | sed 's/^/     /'; exit 2; }
say "   open"

say ""; say "-- dispatch (stream-json; live log: $LOG)"
( cd "$REPO" && timeout 9000 "$CLAUDE" --resume "$REVIEWER_SESSION" -p "$PROMPT" \
    --model "$MODEL" --effort "$EFFORT_LEVEL" --permission-mode bypassPermissions \
    --allowedTools "Read,Grep,Glob,Bash" --max-turns "$MAX_TURNS" \
    --output-format stream-json --verbose --include-partial-messages ) >"$LOG" 2>"$ERRL"
RC=$?; say "   exit=$RC  (log $(wc -c <"$LOG") bytes)"

RESULT_FILE="$WORK/opus-rereview-result-$TS.md"
python3 - "$LOG" "$RESULT_FILE" <<'PY'
import json, sys, pathlib
log, out = sys.argv[1], sys.argv[2]
final = None; chunks = []
for line in open(log, errors="replace"):
    line = line.strip()
    if not line.startswith("{"): continue
    try: d = json.loads(line)
    except Exception: continue
    if d.get("type") == "result":
        final = d
    ev = d.get("event") or {}
    if d.get("type") == "stream_event" and ev.get("delta", {}).get("type") == "text_delta":
        chunks.append(ev["delta"].get("text", ""))
text = ""
if final and isinstance(final.get("result"), str) and len(final["result"]) > 200:
    text = final["result"]
elif chunks:
    text = "".join(chunks)
if text:
    pathlib.Path(out).write_text(text)
    print(f"   report text: {len(text)} chars -> {out}")
else:
    print("   !! no report text in the stream")
if final:
    print(f"   session={final.get('session_id')} turns={final.get('num_turns')} stop={final.get('stop_reason')} cost=${final.get('total_cost_usd')} api_error={final.get('api_error_status')}")
    mu = final.get("modelUsage") or {}
    for k, v in mu.items():
        print(f"   model {k}: ${v.get('costUSD')} in={v.get('inputTokens')} out={v.get('outputTokens')}")
PY

[ -s "$RESULT_FILE" ] || { say "!! nothing to file (no report text). Stream log: $LOG"; exit 1; }
if grep -qEi "session limit|usage limit|credit balance|rate limit|insufficient credits|you've hit your" "$RESULT_FILE"; then
  say "!! VALIDITY GATE: error text, not a report:"; head -3 "$RESULT_FILE" | sed 's/^/     /'; exit 1
fi
BYTES=$(wc -c <"$RESULT_FILE"); say "   report: $BYTES bytes, $(wc -w <"$RESULT_FILE") words"
[ "$BYTES" -ge 5000 ] || { say "!! VALIDITY GATE: $BYTES bytes is below the re-review floor"; exit 1; }

say ""; say "-- read-only proof"
[ "$(git -C "$REPO" status --porcelain | wc -l)" = "0" ] && say "   tree clean — read-only respected" \
  || { say "   !! tree changed:"; git -C "$REPO" status --porcelain | sed 's/^/     /'; }

say ""; say "-- filing: $DEST_REL"
MODEL="$MODEL" BRIEF="$DOCS/044-REREVIEW-BRIEF-2026-10-09.md" \
  INVENTORY="$DOCS/041-APPENDIX-codebase-inventory.txt" SESSION_ID="$REVIEWER_SESSION" \
  bash "$RUNNER/file-opus-review.sh" "$REPO" "$RESULT_FILE" "$DEST_REL" || say "   !! filing failed"

python3 - "$REPO" "$DEST_REL" "$POINTER" "$INDEX" <<'PY'
import pathlib, sys, datetime
repo, dest_rel, pointer, index = sys.argv[1:5]
name = pathlib.Path(dest_rel).name
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
line = (f"- [`{dest_rel}`]({name}) — the claude+opus RE-REVIEW ({stamp}): batches A-D judged by behaviour,\n"
        f"  with the two findings the orchestrator reproduced independently.\n")
ip = pathlib.Path(index)
if ip.exists() and name not in ip.read_text():
    s = ip.read_text(); ip.write_text(s + ("" if s.endswith("\n") else "\n") + line); print("   INDEX updated")
pp = pathlib.Path(pointer)
if pp.exists() and name not in pp.read_text():
    s = pp.read_text()
    banner = (f"> **{stamp} — THE RE-REVIEW HAS RUN: [`{dest_rel}`]({name}).** Batches A-D judged by behaviour;\n"
              f"> read it with 043 before acting on any candidate below.\n\n")
    parts = s.split("\n\n", 2)
    pp.write_text("\n\n".join(parts[:2]) + "\n\n" + banner + (parts[2] if len(parts) > 2 else "")); print("   pointer updated")
PY

bash "$RUNNER/store-report-in-amem.sh" "$REPO/$DEST_REL" "$AMEM_URL" pii "pii/review/rereview-$(date -u +%Y-%m-%d)" 14000 \
  || say "   !! amem store failed (the filed copy stands)"

say ""; say "DONE — report at $REPO/$DEST_REL (stream log: $LOG). The orchestrator reviews, then commits."
