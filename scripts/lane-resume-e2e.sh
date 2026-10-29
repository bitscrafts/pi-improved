#!/usr/bin/env bash
# END-TO-END resume of a composed row with a real claude, through the shipped `spec` graph on the production devbox daemon.
#   run 1  (repo r, spec one): the agent is told a secret token and writes hello.txt.  Its code session id is read from the journal.
#   run 2  (SAME repo r, spec two, input sid = run 1's session): the agent must write the token to answer.txt WITHOUT being told it again. The token is only in run 1's conversation.
#   run 3  CONTROL (another repo c, spec two, NO sid): a fresh agent cannot know the token, so the row must fail its criterion and answer.txt must not hold it.
# The second spec's criterion holds only the token's SHA-256. A first version held the token itself: the fresh control read it from the spec and passed, so that run proved nothing.
# Rows: the transcript of run 1 exists under the daemon's config dir; run 2 succeeded and answer.txt holds the token; run 2's code spawn argv carries --resume; the control did NOT get the token.
# Spend: three tiny haiku runs. The loop must be paused. Nothing is committed by a worker.
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
cd /projects/pi-improved || exit 2
REPO=$PWD; HOST=mvcorrea@10.10.10.217; C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii; GD=$REPO/_tmp/devbox-test/graphs
TS=$(date +%s); P=0; F=0; TOKEN="BANANA-$TS"; TOKHASH=$(printf %s "$TOKEN" | sha256sum | cut -d" " -f1)
row() { if [ "$1" = ok ]; then P=$((P+1)); echo "PASS $2"; else F=$((F+1)); echo "FAIL $2"; fi; }
X() { ssh -o BatchMode=yes -o ConnectTimeout=15 $HOST "podman exec -w /projects/pi-improved pii-devbox $*"; }
cli() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB $*  2>&1'"; }
head -1 _tmp/autonomy/control | grep -q '^PAUSED' || { echo "ABORT: loop not paused"; exit 3; }
HEAD=$(git rev-parse --short=12 HEAD)
DOUT=$(bash modules/devbox/daemon.sh up 2>&1); REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
case "$REV" in "$HEAD"*) ;; *) echo "ABORT: daemon $REV is not HEAD $HEAD"; exit 2;; esac
cp graphs/spec.toml graphs/docs.toml graphs/preflight.toml "$GD/" || exit 2
repo() { local n=$1 W=$REPO/_tmp/devbox-test/e2e-$TS-$1; mkdir -p "$W/repo/specs"
  printf '# Spec: write PONG\n\n## Exit Criteria\n- [ ] `test -f hello.txt`\n- [ ] `grep -qx PONG hello.txt`\n' > "$W/repo/specs/one.md"
  # the criterion carries the token's SHA-256, never the token: an agent that only reads the spec cannot recover it
  printf '# Spec: answer from memory\n\n## Exit Criteria\n- [ ] `test -f answer.txt`\n- [ ] `test "$(head -n1 answer.txt | tr -d "\\n" | sha256sum | cut -d" " -f1)" = %s`\n' "$TOKHASH" > "$W/repo/specs/two.md"
  git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=f@local -c user.name=f commit -qm base; }
input() { # NAME SPEC BRIEF SID
  local W=$REPO/_tmp/devbox-test/e2e-$TS-$1
  uv run --no-project python - "$W/input-$2.json" "$(git -C "$W/repo" rev-parse HEAD)" "$C/e2e-$TS-$1/repo" "specs/$2.md" "$3" "$4" <<'PY'
import json, sys
p, base, wd, spec, brief, sid = sys.argv[1:7]
open(p, "w").write(json.dumps({"agent": "claude", "model": "claude-haiku-5-5", "provider": "anthropic", "spec": spec, "base": base, "workdir": wd, "brief": brief,
  "docs_brief": "Documentation step. Create NOTES.md in the repository root with one line saying done. Do not modify any other file and do not commit.", "max_budget_usd": 1.0, "sid": sid or None}))
PY
}
go() { # NAME SPEC -> run id
  X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start spec --input \"\$(cat $C/e2e-$TS-$1/input-$2.json)\" 2>&1'" | grep -o 'r[0-9]\{12\}' | head -1; }
waitrun() { local i s; for i in $(seq 1 150); do s=$(cli run get "$1" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4); case "$s" in succeeded|failed|refused) echo $s; return;; esac; sleep 3; done; echo timeout; }
LB=$(X sh -c "'wc -l < $C/pii.log'" | tr -d ' ')
codesid() { X sh -c "'tail -n +$((LB+1)) $C/pii.log | grep -a coder.spawned | head -1'" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4; }

repo r; repo c
input r one "Remember this secret token for later: $TOKEN . Now implement specs/one.md: create hello.txt in the repository root containing exactly the line PONG. Do not commit and do not edit the specs." ""
R1=$(go r one); [ -n "$R1" ] && row ok "run 1 started ($R1)" || { row no "run 1 did not start"; echo "E2E-RESUME: $P pass $F fail"; exit 1; }
S1=$(waitrun $R1); [ "$S1" = succeeded ] && row ok "run 1 succeeded" || row no "run 1 status $S1"
SID=$(codesid); echo "  run 1 code session: ${SID:-none}"
FOUND=$(X sh -c "'find $C/claude -name \"$SID.jsonl\" -type f 2>/dev/null | head -1'")
[ -n "$SID" ] && [ -n "$FOUND" ] && row ok "run 1's transcript exists under the config dir, so the lane's own lookup would find it" || row no "run 1's transcript is missing: nothing to resume"
LB=$(X sh -c "'wc -l < $C/pii.log'" | tr -d ' ')

TWO="Use what I told you before. Do not ask me for it. Create answer.txt in the repository root containing the secret token I gave you earlier, on one line. Do not commit and do not edit the specs."
input r two "$TWO" "$SID"
R2=$(go r two); [ -n "$R2" ] && row ok "run 2 (resumed with sid) started ($R2)" || { row no "run 2 did not start"; echo "E2E-RESUME: $P pass $F fail"; exit 1; }
S2=$(waitrun $R2); [ "$S2" = succeeded ] && row ok "run 2 succeeded" || row no "run 2 status $S2"
grep -q "$TOKEN" "$REPO/_tmp/devbox-test/e2e-$TS-r/repo/answer.txt" 2>/dev/null && row ok "run 2's agent wrote the token from run 1's conversation (answer.txt)" || row no "answer.txt does not hold the token: [$(head -c 80 "$REPO/_tmp/devbox-test/e2e-$TS-r/repo/answer.txt" 2>/dev/null)]"
X sh -c "'tail -n +$((LB+1)) $C/pii.log | grep -a coder.spawned | head -1'" | grep -q '"argv_len"' && row ok "run 2 spawned its coder (journalled)" || row no "no coder spawn journalled for run 2"
LB2=$(X sh -c "'wc -l < $C/pii.log'" | tr -d ' ')

input c two "$TWO" ""
R3=$(go c two); [ -n "$R3" ] && row ok "CONTROL run 3 (fresh, no sid) started ($R3)" || { row no "control did not start"; echo "E2E-RESUME: $P pass $F fail"; exit 1; }
S3=$(waitrun $R3); [ "$S3" != succeeded ] && row ok "CONTROL: the fresh agent did NOT pass the criterion that needs the token ($S3)" || row no "CONTROL: the fresh agent passed: the token leaked or the test proves nothing"
grep -q "$TOKEN" "$REPO/_tmp/devbox-test/e2e-$TS-c/repo/answer.txt" 2>/dev/null && row no "CONTROL: answer.txt holds the token without a resume" || row ok "CONTROL: answer.txt does not hold the token"
echo "E2E-RESUME: $P pass $F fail"
[ "$F" = 0 ]