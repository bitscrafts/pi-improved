#!/usr/bin/env bash
# PROOF of the lane-resume miss. Two real haiku runs of the shipped `spec` graph on a trivial spec, differing in ONE line: graph specA = graphs/spec.toml unchanged (renamed); graph specB = the same with "CLAUDE_CONFIG_DIR" added to the code node's env_pass. After each run, look for the code node's session transcript under the daemon's config dir (the place the lane's resume lookup searches). Expected: A has none (the measured miss), B has one (the control that shows the fix). Spend: two tiny haiku runs. Loop paused by the caller.
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
cd /projects/pi-improved || exit 2
REPO=$PWD; HOST=mvcorrea@10.10.10.217; C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii; GD=$REPO/_tmp/devbox-test/graphs
TS=$(date +%s); P=0; F=0
row() { if [ "$1" = ok ]; then P=$((P+1)); echo "PASS $2"; else F=$((F+1)); echo "FAIL $2"; fi; }
X() { ssh -o BatchMode=yes -o ConnectTimeout=15 $HOST "podman exec -w /projects/pi-improved pii-devbox $*"; }
cli() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB $*  2>&1'"; }
head -1 _tmp/autonomy/control | grep -q '^PAUSED' || { echo "ABORT: loop not paused"; exit 3; }
HEAD=$(git rev-parse --short=12 HEAD)
DOUT=$(bash modules/devbox/daemon.sh up 2>&1); REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
case "$REV" in "$HEAD"*) ;; *) echo "ABORT: daemon $REV is not HEAD $HEAD"; exit 2;; esac
cp graphs/docs.toml graphs/preflight.toml "$GD/" || exit 2
LINE='env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR"]'
[ "$(grep -cF "$LINE" graphs/spec.toml)" = 2 ] || [ "$(grep -cF "$LINE" graphs/spec.toml)" = 1 ] || { echo "ABORT: the env_pass line is not as expected"; exit 2; }
sed -e 's/^name = "spec"$/name = "specA"/' graphs/spec.toml > "$GD/specA.toml"
# B: add the name only on the line of the node `code`
uv run --no-project python - "$REPO/graphs/spec.toml" "$GD/specB.toml" <<'PY' || exit 2
import sys, re
s = open(sys.argv[1]).read().replace('name = "spec"\n', 'name = "specB"\n', 1)
i = s.index('id = "code"'); j = s.index('env_pass = [', i); k = s.index(']', j)
assert 'CLAUDE_CONFIG_DIR' not in s[j:k]
s = s[:k] + ', "CLAUDE_CONFIG_DIR"' + s[k:]
open(sys.argv[2], "w").write(s)
PY
diff <(sed 's/specA/spec/' "$GD/specA.toml") graphs/spec.toml >/dev/null && row ok "specA is graphs/spec.toml unchanged (name only)" || row no "specA differs from spec.toml"
[ "$(diff "$GD/specA.toml" "$GD/specB.toml" | grep -c '^[<>]')" = 4 ] && row ok "specB differs from specA by the env_pass line and the name only" || row no "specA and specB differ by more than the intended lines"
mk() { local n=$1 W=$REPO/_tmp/devbox-test/resume-$TS-$1; mkdir -p "$W/repo/specs"; printf '# Spec: write PONG\n\n## Exit Criteria\n- [ ] `test -f hello.txt`\n- [ ] `grep -qx PONG hello.txt`\n' > "$W/repo/specs/real.md"
  git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=f@local -c user.name=f commit -qm base
  uv run --no-project python - "$W/input.json" "$(git -C "$W/repo" rev-parse HEAD)" "$C/resume-$TS-$n/repo" <<'PY'
import json, sys
p, base, wd = sys.argv[1:4]
open(p, "w").write(json.dumps({"agent": "claude", "model": "claude-haiku-5-5", "provider": "anthropic", "spec": "specs/real.md", "base": base, "workdir": wd,
  "brief": "Implement specs/real.md: create hello.txt in the repository root containing exactly the line PONG. Do not commit and do not edit specs/real.md.",
  "docs_brief": "Documentation step. Create NOTES.md in the repository root with one line saying what hello.txt contains. Do not modify hello.txt or specs/real.md and do not commit.", "max_budget_usd": 1.0, "sid": None}))
PY
}
run1() { local n=$1 g=$2; mk $n; local o; o=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start $g --input \"\$(cat $C/resume-$TS-$n/input.json)\" 2>&1'"); echo "$o" | grep -o 'r[0-9]\{12\}' | head -1; }
waitrun() { local i s; for i in $(seq 1 120); do s=$(cli run get "$1" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4); case "$s" in succeeded|failed|refused) echo $s; return;; esac; sleep 3; done; echo timeout; }
LB=$(X sh -c "'wc -l < $C/pii.log'" | tr -d ' ')
codesid() { X sh -c "'tail -n +$((LB+1)) $C/pii.log | grep -a coder.spawned | head -1'" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4; }
for pair in "a specA" "b specB"; do set -- $pair; n=$1; g=$2
  R=$(run1 $n $g); [ -n "$R" ] && row ok "$g started ($R)" || { row no "$g did not start"; continue; }
  S=$(waitrun $R); [ "$S" = succeeded ] && row ok "$g run succeeded" || row no "$g status $S"
  SID=$(codesid); echo "  $g code session: ${SID:-none}"
  FOUND=$(X sh -c "'find $C/claude -name \"$SID.jsonl\" -type f 2>/dev/null | head -1'")
  if [ "$n" = a ]; then [ -z "$FOUND" ] && row ok "specA (graph as shipped): NO transcript under the daemon's config dir: the resume lookup cannot find it" || row no "specA already has a transcript: the theory is wrong"
  else [ -n "$FOUND" ] && row ok "specB (one env_pass name added): the transcript IS under the config dir ($(basename "$FOUND"))" || row no "specB still has no transcript: the missing env_pass name is not the whole cause"; fi
  LB=$(X sh -c "'wc -l < $C/pii.log'" | tr -d ' ')
done
echo "RESUME-PROOF: $P pass $F fail"
[ "$F" = 0 ]