#!/usr/bin/env bash
# Real-agent fault scenarios for the composed `spec` graph, through the production devbox daemon (modules/devbox/daemon.sh), real transport.
#   scripts/archive/composed-real-faults.sh cancel | restart | concurrent | pi     (one scenario per call; each prints PASS/FAIL rows and a verdict)
# Every "agent is gone" row is paired with an "agent was ALIVE before the fault" row, so the check can fail. Spend: haiku/ollama-cloud, a few cents.
# Scratch repos live under _tmp/devbox-test/ ; nothing is committed by a worker. The loop must be paused and no worker in flight.
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
REPO=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}; SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii
SCEN=${1:-}; case "$SCEN" in cancel|restart|concurrent|pi|wall|impl|storm|stormkill) ;; *) echo "usage: $0 cancel|restart|concurrent|pi|wall|impl|storm|stormkill"; exit 2;; esac
TS=$(date +%s); LOGD=$REPO/_tmp/claude-batchC; mkdir -p "$LOGD"; LOG=$LOGD/composed-faults-$SCEN-$TS.log
X() { "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; }
say() { echo "$*" | tee -a "$LOG"; }
P=0; F=0; row() { if [ "$1" = ok ]; then P=$((P+1)); say "PASS $2"; else F=$((F+1)); say "FAIL $2"; fi; }
cli() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB $*  2>&1'"; }
alive() { X sh -c "'kill -0 $1 2>/dev/null && echo alive || echo gone'" | tail -1; }
status_of() { cli run get "$1" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4; }
wait_status() { local i s; for i in $(seq 1 "${2:-240}"); do s=$(status_of "$1"); case "$s" in succeeded|failed|refused|cancelled) echo "$s"; return; esac; sleep 3; done; echo timeout; }
journal() { "${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LB:-0}+1)) $C/pii.log'" 2>/dev/null; }
nth_spawn_pid() { journal | grep -a 'coder.spawned' | sed -n "${1}p" | grep -o '"pid":[0-9]*' | head -1 | cut -d: -f2; }
wait_spawn() { local i p; for i in $(seq 1 "${2:-120}"); do p=$(nth_spawn_pid "$1"); [ -n "$p" ] && { echo "$p"; return; }; sleep 0.5; done; }

AGENT=claude; MODEL=claude-haiku-5-5; PROVIDER=anthropic
[ "$SCEN" = pi ] && { AGENT=pi; MODEL=deepseek-v4.1-flash:cloud; PROVIDER=ollama-cloud; }
# make_run NAME DOCS_BRIEF : a scratch repo + input file; sets W_<NAME>, prints nothing
make_run() {
  local name=$1 dbrief=$2 W=$REPO/_tmp/devbox-test/faults-$SCEN-$TS-$1
  mkdir -p "$W/repo/specs"
  if [ -n "${SPECFILE:-}" ]; then cp "$SPECFILE" "$W/repo/specs/real.md"; else
  printf '# Spec: write PONG\n\n## Exit Criteria\n- [ ] `test -f hello.txt`\n- [ ] `grep -qx PONG hello.txt`\n' > "$W/repo/specs/real.md"; fi
  [ -n "${EXPECTED:-}" ] && printf '%s\n' $EXPECTED > "$W/repo/expected.txt"
  git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=f@local -c user.name=f commit -qm base
  uv run --no-project python - "$W/input.json" "$AGENT" "$MODEL" "$PROVIDER" "$(git -C "$W/repo" rev-parse HEAD)" "/projects/pi-improved/_tmp/devbox-test/faults-$SCEN-$TS-$name/repo" "$dbrief" ${BRIEFFILE:+"$BRIEFFILE"} <<'PY' || exit 2
import json, sys
p, agent, model, provider, base, workdir, dbrief = sys.argv[1:8]
open(p, "w").write(json.dumps({"agent": agent, "model": model, "provider": provider, "spec": "specs/real.md", "base": base, "workdir": workdir,
  "brief": (open(sys.argv[8]).read() if len(sys.argv) > 8 else "Implement specs/real.md: create hello.txt in the repository root containing exactly the line PONG. Do not commit and do not edit specs/real.md."),
  "docs_brief": dbrief, "max_budget_usd": 1.0}))
PY
  eval "W_$name=$W"
}
start() { local w; eval "w=\$W_$1"; X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start ${2:-spec} --input \"\$(cat /projects/pi-improved/_tmp/devbox-test/faults-$SCEN-$TS-$1/input.json)\" 2>&1'" | grep -o 'r[0-9]\{12\}' | head -1; }
SLOW='Documentation step. Build NOTES.md in the repository root one line at a time: issue 20 SEPARATE Bash tool calls, one after another, each exactly: sleep 4; echo "line N" >> NOTES.md   (N from 1 to 20). Do not combine them, do not skip any and do not stop early. Do not modify hello.txt or specs/real.md and do not commit.'
LONG='Documentation step. Build NOTES.md in the repository root one line at a time: issue 40 SEPARATE Bash tool calls, one after another, each exactly: sleep 20; echo "line N" >> NOTES.md   (N from 1 to 40). Do not combine them, do not skip any and do not stop early. Do not modify hello.txt or specs/real.md and do not commit.'
QUICK='Documentation step. Create NOTES.md in the repository root with one line saying what hello.txt contains. Do not modify hello.txt or specs/real.md and do not commit.'

# ---- preconditions: HEAD daemon, nothing in flight, loop paused
head -1 "$REPO/_tmp/autonomy/control" | grep -q '^PAUSED' || { say "ABORT: the loop is not paused"; exit 3; }
pgrep -af 'run-batchC|claude ' | grep -v 'grep\|flock\|composed-real' | head -2 | grep . && { say "ABORT: a worker is in flight"; exit 3; }
HEAD=$(git -C "$REPO" rev-parse --short=12 HEAD)
DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
say "== composed-real-faults $SCEN $TS: $AGENT/$MODEL/$PROVIDER; HEAD $HEAD daemon $REV"
case "$REV" in "$HEAD"*) ;; *) say "ABORT: the daemon is not at HEAD"; exit 2;; esac
cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$REPO/_tmp/devbox-test/graphs/" || exit 2
if [ "$SCEN" = wall ]; then   # the control graph: the same file with the wall limit removed and a different name
  grep -c '^max_wall_ms' "$REPO/graphs/spec.toml" | grep -qx 1 || { say "ABORT: spec.toml has not exactly one max_wall_ms line"; exit 2; }
  sed -e '/^max_wall_ms/d' -e 's/^name = "spec"$/name = "specnowall"/' "$REPO/graphs/spec.toml" > "$REPO/_tmp/devbox-test/graphs/specnowall.toml"
  grep -q '^name = "specnowall"' "$REPO/_tmp/devbox-test/graphs/specnowall.toml" && ! grep -q max_wall_ms "$REPO/_tmp/devbox-test/graphs/specnowall.toml" || { say "ABORT: control graph not built"; exit 2; }
fi
LB=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')

case "$SCEN" in
cancel)
  make_run a "$SLOW"; RID=$(start a); [ -n "$RID" ] || { say "ABORT: no run id"; exit 2; }; say "RUN $RID"
  P1=$(wait_spawn 1 90); P2=$(wait_spawn 2 150)
  [ -n "$P2" ] && row ok "the docs coder (2nd agent) spawned: pid $P2" || { row no "the docs coder never spawned"; P2=0; }
  A=$(alive "$P2"); [ "$A" = alive ] && row ok "CONTROL: the docs agent is ALIVE before the cancel (pid $P2)" || row no "CONTROL: the docs agent is $A before the cancel"
  say "cancel: $(cli run cancel "$RID" --kill --reason fault-test | head -2 | tr '\n' ' ' | cut -c1-160)"
  ST=$(wait_status "$RID" 60); journal | grep -a "\"run\":\"$RID\"" | grep -aq '"kind":"run.cancelled"' && row ok "run.cancelled is journalled for $RID (the store closes a killed cancel as $ST)" || row no "no run.cancelled event for $RID (status $ST)"
  sleep 3; A=$(alive "$P2"); [ "$A" = gone ] && row ok "the real docs agent is GONE after the cancel (pid $P2)" || row no "the real docs agent $P2 is still $A after the cancel"
  [ "$(cat "$W_a/repo/hello.txt" 2>/dev/null)" = PONG ] && row ok "the host's work survived (hello.txt PONG)" || row no "hello.txt is not PONG"
  [ -e "$W_a/repo/NOTES.md" ] && row no "NOTES.md exists: the docs agent finished before the cancel" || row ok "the docs step did not complete (no NOTES.md)"
  cli run trace "$RID" > "$LOGD/composed-faults-$SCEN-$TS.trace" 2>&1; grep -q ' 0 orphans' "$LOGD/composed-faults-$SCEN-$TS.trace" && row ok "run trace: 0 orphans" || row no "run trace reports orphans: $(tail -1 "$LOGD/composed-faults-$SCEN-$TS.trace")" ;;
restart)
  make_run a "$SLOW"; RID=$(start a); [ -n "$RID" ] || { say "ABORT: no run id"; exit 2; }; say "RUN $RID"
  wait_spawn 1 90 >/dev/null; P2=$(wait_spawn 2 150); [ -n "$P2" ] && row ok "the docs coder spawned: pid $P2" || { row no "no docs spawn"; P2=0; }
  DP=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1); say "daemon pid $DP"
  case "$DP" in ''|*[!0-9]*|0|1) row no "could not find the real daemon pid [$DP]: refusing to kill anything"; say "COMPOSED-FAULTS $SCEN: $P pass $F fail"; exit 1;; esac
  [ "$(X sh -c "'ps -p $DP -o args= | cut -c1-60'" | tail -1 | grep -c 'bin/pii daemon')" = 1 ] && row ok "the pid $DP IS the pii daemon, not a wrapper" || row no "pid $DP is not the pii daemon"
  X sh -c "'kill -9 $DP'" >/dev/null 2>&1; sleep 2
  [ "$(alive "$DP")" = gone ] && row ok "the daemon pid $DP is dead after SIGKILL" || row no "the daemon pid $DP is still alive after SIGKILL"
  A=$(alive "$P2"); [ "$A" = alive ] && row ok "CONTROL: the real docs agent SURVIVED the daemon's SIGKILL (an orphan, pid $P2)" || row no "CONTROL: the agent is $A right after the kill, so there is nothing to reap"
  LB2=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')
  DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); say "daemon again: $(printf '%s' "$DOUT" | tail -1 | cut -c1-80)"
  sleep 4; NEWDP=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1)
  [ -n "$NEWDP" ] && [ "$NEWDP" != "$DP" ] && row ok "a NEW daemon is running (pid $NEWDP, was $DP)" || row no "no new daemon: [$NEWDP] vs [$DP]"
  ST=$(status_of "$RID"); [ "$ST" = failed ] && row ok "the run is reconciled to failed ($ST)" || row no "the run status after restart is $ST (want failed)"
  [ -e "$W_a/repo/NOTES.md" ] && [ "$(wc -l < "$W_a/repo/NOTES.md")" -ge 20 ] && row no "the docs agent had FINISHED its work before the restart: the scenario proved nothing" || row ok "the docs agent had NOT finished its work ($(wc -l < "$W_a/repo/NOTES.md" 2>/dev/null || echo 0) of 20 lines)"
  A=$(alive "$P2"); [ "$A" = gone ] && row ok "the restart REAPED the real claude (pid $P2 gone)" || row no "the real claude pid $P2 is still $A after the restart: the reaper cannot find it"
  "${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LB2:-0}+1)) $C/pii.log | grep -a -E \"reap|orphan|interrupted\" | head -4'" 2>/dev/null | cut -c1-300 | tee -a "$LOG"
  X sh -c "'kill -9 $P2 2>/dev/null; true'" >/dev/null 2>&1 ;;
concurrent)
  make_run a "$QUICK"; make_run b "$QUICK"; RA=$(start a); RB=$(start b); say "RUNS $RA $RB"
  [ -n "$RA" ] && [ -n "$RB" ] && [ "$RA" != "$RB" ] && row ok "two distinct run ids" || { row no "run ids: [$RA] [$RB]"; }
  SA=$(wait_status "$RA" 120); SB=$(wait_status "$RB" 120)
  [ "$SA" = succeeded ] && [ "$SB" = succeeded ] && row ok "both runs succeeded ($SA, $SB)" || row no "statuses: $SA $SB"
  N=$(journal | grep -a 'coder.spawned' | grep -o '"session":"[^"]*"' | sort -u | wc -l); [ "$N" = 4 ] && row ok "four distinct agent sessions" || row no "distinct sessions: $N (want 4)"
  for n in a b; do eval "w=\$W_$n"; [ "$(cat $w/repo/hello.txt 2>/dev/null)" = PONG ] && [ -s "$w/repo/NOTES.md" ] && row ok "repo $n: hello.txt PONG and its own NOTES.md" || row no "repo $n incomplete"; done
  [ "$(git -C "$W_a/repo" rev-list --count HEAD)$(git -C "$W_b/repo" rev-list --count HEAD)" = 11 ] && row ok "nothing committed in either repo" || row no "a worker committed"
  cli run trace "$RA" | grep -q 'call docs' && cli run trace "$RB" | grep -q 'call docs' && row ok "each run's trace has its own call docs" || row no "a trace lacks the call" ;;
impl)
  mkdir -p /tmp/impl-$TS; SPECFILE=/tmp/impl-$TS/spec.md; BRIEFFILE=/tmp/impl-$TS/brief.txt
  cat > $SPECFILE <<'EOF'
# Spec: fizzbuzz as a shell program

## Overview
Implement FizzBuzz for 1..15 as shell scripts in the repository root.

## Requirements
- `fizz.sh` prints the numbers 1 to 15, one per line, replacing multiples of 3 with Fizz, of 5 with Buzz and of both with FizzBuzz.
- `check.sh` runs `sh fizz.sh` and compares its output with `expected.txt` (already present), exiting 0 only on an exact match.
- `README.md` has a line `Usage: sh check.sh`.

## Exit Criteria
- [ ] `test -f fizz.sh && test -f check.sh && test -f README.md`
- [ ] `sh fizz.sh | diff -q - expected.txt`
- [ ] `sh check.sh`
- [ ] `grep -q 'Usage: sh check.sh' README.md`
EOF
  echo "Implement specs/real.md in this repository: write fizz.sh, check.sh and README.md exactly as the spec says, run sh check.sh to confirm it exits 0, and do not edit specs/real.md or expected.txt. Do not commit anything." > $BRIEFFILE
  EXPECTED="1 2 Fizz 4 Buzz Fizz 7 8 Fizz Buzz 11 Fizz 13 14 FizzBuzz"
  make_run a "$QUICK"
  cp $SPECFILE /tmp/impl-$TS/spec-never.md; printf '%s\n' '- [ ] `test -f NEVER.txt`' >> /tmp/impl-$TS/spec-never.md
  SPECFILE=/tmp/impl-$TS/spec-never.md; make_run b "$QUICK"
  RA=$(start a); RB=$(start b); say "RUNS $RA (four criteria)  $RB (a fifth criterion the work cannot meet)"
  SA=$(wait_status "$RA" 200); SB=$(wait_status "$RB" 200)
  [ "$SA" = succeeded ] && row ok "A: the real implementation run succeeded" || row no "A status $SA"
  for f in fizz.sh check.sh README.md; do [ -s "$W_a/repo/$f" ] && row ok "A wrote $f" || row no "A is missing $f"; done
  (cd "$W_a/repo" && sh fizz.sh | diff -q - expected.txt >/dev/null && sh check.sh >/dev/null) && row ok "A: the program really prints FizzBuzz (checked here, not by the agent)" || row no "A: the program is wrong"
  TA=$(cli run trace "$RA")
  printf '%s\n' "$TA" | grep -qE '^[0-9]+ verify ' && printf '%s\n' "$TA" | grep -qE '^[0-9]+ reverify ' && printf '%s\n' "$TA" | grep -qE '^[0-9]+ done ' && ! printf '%s\n' "$TA" | grep -q unverified && row ok "A: verify and reverify ran and the run reached done (no unverified)" || row no "A: the trace is not verify, docs, reverify, done"
  [ "$(printf '%s\n' "$TA" | grep -c 'pid=[0-9]')" = 2 ] && row ok "A: exactly two agent calls (code and the docs coder)" || row no "A: agent calls: $(printf '%s\n' "$TA" | grep -c 'pid=[0-9]')"
  [ "$(git -C "$W_a/repo" rev-list --count HEAD)" = 1 ] && [ "$(git -C "$W_a/repo" diff --stat -- specs expected.txt | wc -l)" = 0 ] && row ok "A: spec and expected.txt untouched, nothing committed" || row no "A: a worker touched the spec or committed"
  [ "$SB" = failed ] && row ok "CONTROL: B (an unmeetable fifth criterion) FAILED" || row no "CONTROL: B status $SB (want failed)"
  cli run trace "$RB" | grep -q 'unverified' && row ok "CONTROL: B's trace reaches the unverified refusal" || row no "CONTROL: B never reaches unverified"
  TB=$(cli run trace "$RB"); [ "$(printf '%s\n' "$TB" | grep -c 'pid=[0-9]')" = 1 ] && ! printf '%s\n' "$TB" | grep -qE '^[0-9]+ docs__docs ' && row ok "CONTROL: B was stopped at verify, the docs coder never ran (one agent call)" || row no "CONTROL: B went on to run the docs coder"
  [ "$(cat "$W_b/repo/fizz.sh" 2>/dev/null | wc -l)" -gt 0 ] && (cd "$W_b/repo" && sh check.sh >/dev/null 2>&1) && row ok "CONTROL: B's program itself is correct, only the unmeetable criterion failed it" || row no "CONTROL: B's program does not pass its own check"
  say "KNOWN GAP (spec 293, not counted): the judge's per-criterion result (4 of 5) is not printed by run trace, get or explain" ;;
wall)
  make_run a "$LONG"; make_run b "$LONG"; RA=$(start a spec); RB=$(start b specnowall); say "RUNS $RA (spec, with the limit)  $RB (specnowall, the default 600 s)"
  [ -n "$RA" ] && [ -n "$RB" ] && row ok "both runs started" || { row no "run ids [$RA] [$RB]"; say "COMPOSED-FAULTS wall: $P pass $F fail"; exit 1; }
  SA=$(wait_status "$RA" 600); SB=$(wait_status "$RB" 600)
  DA=$(cli run get "$RA" | grep -o '"dur_ms":[0-9]*' | cut -d: -f2); DB=$(cli run get "$RB" | grep -o '"dur_ms":[0-9]*' | cut -d: -f2)
  say "A: $SA after ${DA:-?} ms   B: $SB after ${DB:-?} ms"
  [ "$SA" = succeeded ] && row ok "A (spec, wall limit set) SUCCEEDED" || row no "A status $SA"
  [ "${DA:-0}" -gt 600000 ] && row ok "A ran PAST 600 s (${DA} ms), so the limit really mattered" || row no "A took ${DA:-?} ms: it never reached 600 s, nothing was proven"
  [ "$(wc -l < "$W_a/repo/NOTES.md" 2>/dev/null || echo 0)" -ge 40 ] && row ok "A's docs agent did all 40 steps" || row no "A's docs agent wrote $(wc -l < "$W_a/repo/NOTES.md" 2>/dev/null || echo 0) of 40 lines"
  [ "$SB" = failed ] && row ok "CONTROL: B (default wall) FAILED" || row no "CONTROL: B status $SB (want failed)"
  [ "${DB:-0}" -gt 600000 ] && row ok "CONTROL: B ran past 600 s before failing (${DB} ms)" || row no "CONTROL: B ended after ${DB:-?} ms, before 600 s"
  journal | grep -a "\"run\":\"$RB\"" | grep -aiE 'wall|budget|limit' | head -3 | cut -c1-260 | tee -a "$LOG"
  [ "$(cli run get "$RB" | grep -c 'exited=0')" -ge 2 ] && [ "$SA" = succeeded ] && [ "$SB" = failed ] && row ok "CONTROL: the two graphs differ ONLY in the limit; both agents exited 0 in B, so the wall limit is what failed it" || row no "CONTROL: B's agents did not both exit 0"
  cli run explain "$RB" | grep -qi 'wall' && row ok "B's explain names the wall limit" || say "KNOWN GAP (spec 293, not counted): run explain/get/trace do not name why B failed; the engine records 'max_wall_ms exceeded' and the CLI never prints it"
  [ "$(cat "$W_b/repo/hello.txt" 2>/dev/null)" = PONG ] && row ok "CONTROL: B's host work was complete (hello.txt PONG), the only fault is the limit" || row no "CONTROL: B hello.txt not PONG" ;;
pi)
  make_run a "$QUICK"; RID=$(start a); [ -n "$RID" ] || { say "ABORT: no run id"; exit 2; }; say "RUN $RID"
  ST=$(wait_status "$RID" 300)
  [ "$ST" = succeeded ] && row ok "the pi/ollama-cloud composed run succeeded" || row no "status $ST"
  [ "$(cat "$W_a/repo/hello.txt" 2>/dev/null)" = PONG ] && row ok "hello.txt PONG written by pi" || row no "hello.txt is [$(cat "$W_a/repo/hello.txt" 2>/dev/null)]"
  [ -s "$W_a/repo/NOTES.md" ] && row ok "the docs coder (pi) wrote NOTES.md" || row no "no NOTES.md"
  N=$(journal | grep -a 'coder.spawned' | grep -o '"session":"[^"]*"' | sort -u | wc -l); [ "$N" -ge 2 ] && row ok "two distinct pi sessions ($N)" || row no "distinct sessions: $N"
  journal | grep -a 'coder.spawned' | grep -o '"agent":"[a-z]*"' | sort -u | tr '\n' ' ' | tee -a "$LOG"; echo
  cli run trace "$RID" | grep -q 'call docs' && row ok "the trace has call docs" || row no "no call in the trace" ;;
storm)
  # 8 runs at once. c1..c3 claude ok, p1..p3 pi ok, u claude with an UNMEETABLE criterion, x claude SLOW docs that is CANCELLED mid-docs.
  printf '# Spec: write PONG\n\n## Exit Criteria\n- [ ] `test -f hello.txt`\n- [ ] `grep -qx NEVER hello.txt`\n' > /tmp/storm-unmeetable-$TS.md
  for n in c1 c2 c3; do make_run $n "$QUICK"; done
  AGENT=pi; MODEL=deepseek-v4.1-flash:cloud; PROVIDER=ollama-cloud; for n in p1 p2 p3; do make_run $n "$QUICK"; done
  AGENT=claude; MODEL=claude-haiku-5-5; PROVIDER=anthropic
  SPECFILE=/tmp/storm-unmeetable-$TS.md make_run u "$QUICK"; make_run x "$SLOW"
  declare -A RID
  for n in c1 c2 c3 p1 p2 p3 u x; do RID[$n]=$(start $n); done
  say "RUNS: $(for n in c1 c2 c3 p1 p2 p3 u x; do printf '%s=%s ' $n "${RID[$n]}"; done)"
  [ "$(printf '%s\n' "${RID[@]}" | grep -c '^r[0-9]\{12\}$')" = 8 ] && [ "$(printf '%s\n' "${RID[@]}" | sort -u | wc -l)" = 8 ] && row ok "eight distinct run ids started at once" || { row no "run ids: ${RID[*]}"; say "COMPOSED-FAULTS $SCEN: $P pass $F fail"; exit 1; }
  # cancel x once its docs coder has begun writing (NOTES.md has a line): alive before, gone after, others unharmed
  for i in $(seq 1 120); do [ -s "$W_x/repo/NOTES.md" ] && break; sleep 3; done
  XP=$(X sh -c "'for p in \$(pgrep -f claude); do [ \"\$(readlink /proc/\$p/cwd)\" = $W_x/repo ] && echo \$p; done'" | tr '\n' ' ')
  [ -n "${XP// /}" ] && row ok "CONTROL: x's docs agent is ALIVE and working before the cancel (pids $XP, NOTES.md has $(wc -l < "$W_x/repo/NOTES.md") lines)" || row no "CONTROL: no live agent of x before the cancel"
  say "cancel x: $(cli run cancel "${RID[x]}" --kill --reason storm-test | head -2 | tr '\n' ' ' | cut -c1-120)"
  sleep 4; LIVE=""; for p in $XP; do [ "$(alive $p)" = alive ] && LIVE="$LIVE $p"; done
  [ -z "$LIVE" ] && row ok "x's real agent is GONE after the cancel" || row no "x's agent still alive:$LIVE"
  declare -A ST
  for n in c1 c2 c3 p1 p2 p3 u; do ST[$n]=$(wait_status "${RID[$n]}" 200); done
  ok=0; for n in c1 c2 c3 p1 p2 p3; do [ "${ST[$n]}" = succeeded ] && ok=$((ok+1)); done
  [ "$ok" = 6 ] && row ok "all six good runs succeeded while x was cancelled and u was refused (3 claude, 3 pi)" || row no "good runs: c1=${ST[c1]} c2=${ST[c2]} c3=${ST[c3]} p1=${ST[p1]} p2=${ST[p2]} p3=${ST[p3]}"
  [ "${ST[u]}" = failed ] && row ok "CONTROL: u (unmeetable criterion) failed, so success is not the default" || row no "u ended ${ST[u]} (want failed)"
  [ "$(status_of "${RID[x]}")" = failed ] && row ok "x ended failed (a killed cancel closes as failed)" || row no "x status $(status_of "${RID[x]}")"
  for n in c1 c2 c3 p1 p2 p3; do eval "w=\$W_$n"; [ "$(cat $w/repo/hello.txt 2>/dev/null)" = PONG ] && [ -s "$w/repo/NOTES.md" ] || row no "repo $n: hello.txt or NOTES.md wrong"; done
  [ "$(cat "$W_u/repo/hello.txt" 2>/dev/null)" = PONG ] && [ ! -e "$W_u/repo/NOTES.md" ] && row ok "u wrote hello.txt but its docs step never ran (refused at verify)" || row no "u repo state wrong"
  # isolation: every repo has only its own files; nothing committed by a worker
  bad=0; for n in c1 c2 c3 p1 p2 p3 u x; do eval "w=\$W_$n"; [ "$(git -C $w/repo rev-list --count HEAD)" = 1 ] || bad=$((bad+1)); [ "$(ls $w/repo | grep -v -E '^(hello.txt|NOTES.md|specs|expected.txt)$' | wc -l)" = 0 ] || bad=$((bad+1)); done
  [ "$bad" = 0 ] && row ok "no repo has a foreign file or a worker commit (8 repos)" || row no "$bad isolation violations"
  SPAWN=$(journal | grep -a 'coder.spawned' | wc -l); DIST=$(journal | grep -a 'coder.spawned' | grep -o '"session":"[^"]*"' | sort -u | wc -l)
  [ "$SPAWN" = "$DIST" ] && [ "$DIST" -ge 14 ] && row ok "every agent spawn has its own session ($DIST distinct for $SPAWN spawns)" || row no "sessions: $DIST distinct for $SPAWN spawns (want equal and >= 14)"
  LEFT=$(X sh -c "'pgrep -f \"[c]laude --\" | wc -l; pgrep -f \"[p]i -p\" | wc -l'" | tr '\n' ' '); [ "$(echo $LEFT | tr -d ' 0')" = "" ] && row ok "no agent process is left in the container ($LEFT)" || row no "agent processes left: $LEFT: $(X sh -c "'pgrep -af \"[c]laude --\"; pgrep -af \"[p]i -p\"'" | cut -c1-100 | head -3 | tr '\n' '|')"
  for n in c1 p1 x; do cli run trace "${RID[$n]}" | grep -q ' 0 orphans' && row ok "run $n: 0 orphans" || row no "run $n trace: $(cli run trace "${RID[$n]}" | grep orphans)"; done ;;
stormkill)
  for n in a b c d e f; do make_run $n "$SLOW"; done
  declare -A RID; for n in a b c d e f; do RID[$n]=$(start $n); done
  say "RUNS: ${RID[*]}"; [ "$(printf '%s\n' "${RID[@]}" | sort -u | grep -c '^r[0-9]\{12\}$')" = 6 ] && row ok "six distinct run ids" || { row no "run ids: ${RID[*]}"; exit 1; }
  for i in $(seq 1 150); do c=0; for n in a b c d e f; do eval "w=\$W_$n"; [ -s "$w/repo/NOTES.md" ] && c=$((c+1)); done; [ "$c" = 6 ] && break; sleep 3; done
  say "docs coders writing: $c of 6"; [ "$c" = 6 ] && row ok "CONTROL: all six docs coders are mid-work before the kill" || row no "only $c of 6 docs coders were working: the scenario proves nothing"
  PIDS=""; for n in a b c d e f; do eval "w=\$W_$n"; PIDS="$PIDS $(X sh -c "'for p in \$(pgrep -f claude); do [ \"\$(readlink /proc/\$p/cwd)\" = $w/repo ] && echo \$p; done'" | tr '\n' ' ')"; done
  NP=$(echo $PIDS | wc -w); [ "$NP" -ge 6 ] && row ok "CONTROL: $NP real agent processes alive before the kill" || row no "only $NP agent processes alive"
  DP=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1)
  case "$DP" in ''|*[!0-9]*|0|1) row no "could not find the daemon pid [$DP]: refusing to kill anything"; say "COMPOSED-FAULTS $SCEN: $P pass $F fail"; exit 1;; esac
  [ "$(X sh -c "'ps -p $DP -o args= | cut -c1-60'" | tail -1 | grep -c 'bin/pii daemon')" = 1 ] && row ok "pid $DP IS the pii daemon" || { row no "pid $DP is not the daemon"; exit 1; }
  X sh -c "'kill -9 $DP'" >/dev/null 2>&1; sleep 1
  [ "$(alive $DP)" = gone ] && row ok "daemon $DP dead after SIGKILL" || row no "daemon still alive"
  A=0; for p in $PIDS; do [ "$(alive $p)" = alive ] && A=$((A+1)); done
  [ "$A" -ge 1 ] && row ok "CONTROL: $A of $NP agents survived the daemon's death (orphans the restart must reap)" || row no "CONTROL: every agent was already gone, nothing to reap"
  DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); sleep 6
  NEW=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1); [ -n "$NEW" ] && [ "$NEW" != "$DP" ] && row ok "a new daemon is running (pid $NEW)" || row no "no new daemon: [$NEW]"
  A=0; for p in $PIDS; do [ "$(alive $p)" = alive ] && A=$((A+1)); done
  [ "$A" = 0 ] && row ok "the restart reaped all $NP real agents" || { row no "$A of $NP agents still alive after the restart"; for p in $PIDS; do X sh -c "'kill -9 $p 2>/dev/null; true'" >/dev/null 2>&1; done; }
  f=0; for n in a b c d e f; do [ "$(status_of "${RID[$n]}")" = failed ] && f=$((f+1)); done
  [ "$f" = 6 ] && row ok "all six runs reconciled to failed" || row no "$f of 6 runs failed after the restart"
  done_n=0; for n in a b c d e f; do eval "w=\$W_$n"; [ "$(wc -l < $w/repo/NOTES.md 2>/dev/null || echo 0)" -ge 20 ] && done_n=$((done_n+1)); done
  [ "$done_n" = 0 ] && row ok "no docs coder had finished its 20 lines: the kill hit real work" || row no "$done_n docs coders had finished before the kill" ;;
esac
say "COMPOSED-FAULTS $SCEN: $P pass $F fail   (log $LOG)"
[ "$F" = 0 ]
