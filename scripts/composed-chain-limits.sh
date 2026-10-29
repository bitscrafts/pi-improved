#!/usr/bin/env bash
# Real-agent LIMIT scenarios for composed call chains, through the production devbox daemon (real transport). One scenario per call; each prints PASS/FAIL rows and a verdict.
#   scripts/composed-chain-limits.sh restart | concurrent | pi | loop      (spend: haiku / ollama-cloud, a few cents each)
# restart    SIGKILL the daemon while a real agent works at the leaf of a 3-level chain: the agent is an orphan (CONTROL: alive after the kill), the new daemon reconciles the run and REAPS it.
# concurrent four 3-level chains at once: four distinct sessions, each repo with its own result, nothing shared.
# pi         the same chain with pi (ollama-cloud) at the leaf instead of claude.
# loop       a call inside a loop with max_entries = 2 (spec 306): exactly 2 real agent spawns, then the run fails NAMING the call; CONTROL: the same loop with a higher limit stops on the loop budget instead.
# midfail    a 3-level chain whose MIDDLE graph refuses after its call returned: explain and the trace must name the refusing node by its call path and give the reason; CONTROL: the same graphs without the refuse node succeed.
# The loop must be paused and no worker in flight. Nothing is committed by a worker.
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
REPO=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)
SCEN=${1:-}; case "$SCEN" in restart|concurrent|pi|loop|midfail) ;; *) echo "usage: $0 restart|concurrent|pi|loop|midfail"; exit 2;; esac
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}; SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii; GD=$REPO/_tmp/devbox-test/graphs
TS=$(date +%s); export TS; LOGD=$REPO/_tmp/claude-batchC; mkdir -p "$LOGD"; LOG=$LOGD/composed-limits-$SCEN-$TS.log
X() { "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; }
say() { echo "$*" | tee -a "$LOG"; }
P=0; F=0; row() { if [ "$1" = ok ]; then P=$((P+1)); say "PASS $2"; else F=$((F+1)); say "FAIL $2"; fi; }
cli() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB $*  2>&1'"; }
alive() { X sh -c "'kill -0 $1 2>/dev/null && echo alive || echo gone'" | tail -1; }
status_of() { cli run get "$1" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4; }
wait_status() { local i s; for i in $(seq 1 "${2:-240}"); do s=$(status_of "$1"); case "$s" in succeeded|failed|refused|cancelled) echo "$s"; return; esac; sleep 3; done; echo timeout; }
journal() { "${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LB:-0}+1)) $C/pii.log'" 2>/dev/null; }
wait_spawn() { local i p; for i in $(seq 1 "${2:-120}"); do p=$(journal | grep -a 'coder.spawned' | sed -n "${1}p" | grep -o '"pid":[0-9]*' | head -1 | cut -d: -f2); [ -n "$p" ] && { echo "$p"; return; }; sleep 0.5; done; }
eval "$(grep -E '^SLOW=' "$REPO/scripts/composed-real-faults.sh")"
[ -n "${SLOW:-}" ] || { say "ABORT: the slow docs brief was not found"; exit 2; }
CHAINQ='Create NOTES.md in the repository root containing exactly one line: chain ok. Do not commit and do not edit anything else.'
AGENT=claude; MODEL=claude-haiku-5-5; PROVIDER=anthropic
[ "$SCEN" = pi ] && { AGENT=pi; MODEL=deepseek-v4.1-flash:cloud; PROVIDER=ollama-cloud; }

head -1 "$REPO/_tmp/autonomy/control" | grep -q '^PAUSED' || { say "ABORT: the loop is not paused"; exit 3; }
pgrep -af 'run-batchC|claude ' | grep -v 'grep\|flock\|composed-chain' | head -2 | grep . && { say "ABORT: a worker is in flight"; exit 3; }
HEAD=$(git -C "$REPO" rev-parse --short=12 HEAD)
DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
say "== composed-chain-limits $SCEN $TS: $AGENT/$MODEL/$PROVIDER; HEAD $HEAD daemon $REV"
case "$REV" in "$HEAD"*) ;; *) say "ABORT: the daemon is not at HEAD"; exit 2;; esac

NEEDS='needs = ["agent", "model", "provider", "spec", "workdir", "docs_brief"]'
cp "$REPO/graphs/docs.toml" "$GD/docs.toml" || exit 2
D0=$(sha256sum "$GD/docs.toml" | cut -d' ' -f1)
printf 'name = "chain1"\nentry = "docs"\nexit = "out"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "docs"\nkind = "call"\ngraph = "docs"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[node.in]\nbrief = "$.input.docs_brief"\n\n[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "docs"\nport = "ok"\nto = "out"\nkind = "data"\n' "$NEEDS" "$D0" > "$GD/chain1.toml"
D1=$(sha256sum "$GD/chain1.toml" | cut -d' ' -f1)
printf 'name = "chain2"\nentry = "via1"\nexit = "out"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "via1"\nkind = "call"\ngraph = "chain1"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "via1"\nport = "ok"\nto = "out"\nkind = "data"\n' "$NEEDS" "$D1" > "$GD/chain2.toml"
D2=$(sha256sum "$GD/chain2.toml" | cut -d' ' -f1)
printf 'name = "chain3"\nentry = "via2"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "via2"\nkind = "call"\ngraph = "chain2"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "fin"\nkind = "transform"\nvalue = "done"\n\n[[edge]]\nfrom = "via2"\nport = "ok"\nto = "fin"\nkind = "data"\n' "$NEEDS" "$D2" > "$GD/chain3.toml"
# the loop host: start -> docs(call, max_entries M) -> again(loop_budget B) -> start ; M and B differ between the run and its control
loophost() { printf 'name = "%s"\nentry = "start"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "start"\nkind = "transform"\nexpr = "$.payload"\n\n[[node]]\nid = "docs"\nkind = "call"\ngraph = "docs"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\nmax_entries = %s\n\n[node.in]\nbrief = "$.input.docs_brief"\n\n[[node]]\nid = "again"\nkind = "transform"\nexpr = "$.payload"\nloop_budget = %s\n\n[[edge]]\nfrom = "start"\nport = "ok"\nto = "docs"\nkind = "data"\n\n[[edge]]\nfrom = "docs"\nport = "ok"\nto = "again"\nkind = "data"\n\n[[edge]]\nfrom = "again"\nport = "ok"\nto = "start"\nkind = "data"\n' "$1" "$NEEDS" "$D0" "$2" "$3" > "$GD/$1.toml"; }
# midfail: fchainN is the chain with a refuse node after the call in the middle graph; okchainN is the same without it
if [ "$SCEN" = midfail ]; then
  mid() { # NAME(prefix) WITHREFUSE(1/0)
    local pre=$1 bad=$2
    if [ "$bad" = 1 ]; then OUTN='[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[node]]\nid = "bad"\nkind = "refuse"\nhook = "judge"\nreason = "the middle graph refused its leaf"\n\n[[edge]]\nfrom = "via1"\nport = "ok"\nto = "out"\nkind = "data"\n\n[[edge]]\nfrom = "via1"\nport = "ok"\nto = "bad"\nkind = "data"\n'; EXITL='exit = "out"\n'; else OUTN='[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "via1"\nport = "ok"\nto = "out"\nkind = "data"\n'; EXITL='exit = "out"\n'; fi
    printf 'name = "%s1"\nentry = "docs"\nexit = "out"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "docs"\nkind = "call"\ngraph = "docs"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[node.in]\nbrief = "$.input.docs_brief"\n\n[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "docs"\nport = "ok"\nto = "out"\nkind = "data"\n' "$pre" "$NEEDS" "$D0" > "$GD/${pre}1.toml"
    local e1; e1=$(sha256sum "$GD/${pre}1.toml" | cut -d' ' -f1)
    printf "name = \"${pre}2\"\nentry = \"via1\"\n${EXITL}%s\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"via1\"\nkind = \"call\"\ngraph = \"${pre}1\"\nsha256 = \"%s\"\ncaps = [\"spawn\", \"proc\"]\n\n${OUTN}" "$NEEDS" "$e1" > "$GD/${pre}2.toml"
    local e2; e2=$(sha256sum "$GD/${pre}2.toml" | cut -d' ' -f1)
    printf 'name = "%s3"\nentry = "via2"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "via2"\nkind = "call"\ngraph = "%s2"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "fin"\nkind = "transform"\nvalue = "done"\n\n[[edge]]\nfrom = "via2"\nport = "ok"\nto = "fin"\nkind = "data"\n' "$pre" "$NEEDS" "$pre" "$e2" > "$GD/${pre}3.toml"
  }
  mid fchain 1; mid okchain 0
fi
LB=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')

make_input() {   # NAME DOCS_BRIEF : a scratch repo and its input; sets W_<NAME>
  local name=$1 W=$REPO/_tmp/devbox-test/lim-$SCEN-$TS-$1
  mkdir -p "$W/repo/specs"; printf '# Spec: chain\n\n## Exit Criteria\n- [ ] `true`\n' > "$W/repo/specs/real.md"
  git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=f@local -c user.name=f commit -qm base
  uv run --no-project python - "$W/input.json" "$name" "$2" "$AGENT" "$MODEL" "$PROVIDER" <<'PY'
import json, os, sys
p, name, dbrief, agent, model, provider = sys.argv[1:7]
open(p, "w").write(json.dumps({"agent": agent, "model": model, "provider": provider, "spec": "specs/real.md",
  "workdir": f"/projects/pi-improved/_tmp/devbox-test/lim-{os.environ['SCEN']}-{os.environ['TS']}-{name}/repo", "docs_brief": dbrief, "max_budget_usd": 1.0}))
PY
  eval "W_$name=$W"
}
export SCEN
start_raw() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start ${2:-chain3} --input \"\$(cat /projects/pi-improved/_tmp/devbox-test/lim-$SCEN-$TS-$1/input.json)\" 2>&1'"; }
rid_of() { grep -o 'r[0-9]\{12\}' | head -1; }

case "$SCEN" in
restart)
  make_input a "$SLOW"; RID=$(start_raw a | rid_of); [ -n "$RID" ] && row ok "started a 3-level chain ($RID)" || { row no "no run id"; exit 1; }
  P1=$(wait_spawn 1 240); [ -n "$P1" ] && row ok "the real leaf agent spawned (pid $P1)" || { row no "no leaf spawn"; P1=0; }
  DP=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1)
  case "$DP" in ''|*[!0-9]*|0|1) row no "could not find the daemon pid [$DP]: refusing to kill anything"; say "COMPOSED-LIMITS $SCEN: $P pass $F fail"; exit 1;; esac
  [ "$(X sh -c "'ps -p $DP -o args= | cut -c1-60'" | tail -1 | grep -c 'bin/pii daemon')" = 1 ] && row ok "pid $DP IS the pii daemon" || row no "pid $DP is not the daemon"
  X sh -c "'kill -9 $DP'" >/dev/null 2>&1; sleep 2
  [ "$(alive "$DP")" = gone ] && row ok "the daemon is dead after SIGKILL" || row no "the daemon survived"
  [ "$(alive "$P1")" = alive ] && row ok "CONTROL: the real leaf agent SURVIVED the kill (an orphan, pid $P1)" || row no "CONTROL: the agent is not alive right after the kill: nothing proven"
  bash "$REPO/modules/devbox/daemon.sh" up >/dev/null 2>&1; sleep 5
  ND=$(X sh -c "'pgrep -f \"^$PIIB daemon\" | head -1'" | tail -1); [ -n "$ND" ] && [ "$ND" != "$DP" ] && row ok "a NEW daemon runs (pid $ND)" || row no "no new daemon [$ND]"
  [ "$(status_of "$RID")" = failed ] && row ok "the run is reconciled to failed" || row no "run status $(status_of "$RID") (want failed)"
  [ -e "$W_a/repo/NOTES.md" ] && [ "$(wc -l < "$W_a/repo/NOTES.md")" -ge 20 ] && row no "the leaf had finished before the kill: nothing proven" || row ok "the leaf had not finished"
  sleep 3; [ "$(alive "$P1")" = gone ] && row ok "the restart REAPED the real leaf agent (pid $P1 gone)" || row no "the real leaf $P1 is still alive after the restart"
  X sh -c "'kill -9 ${P1:-0} 2>/dev/null; true'" >/dev/null 2>&1 ;;
concurrent)
  for n in a b c d; do make_input $n "$CHAINQ"; done
  for n in a b c d; do eval "R_$n=\$(start_raw $n | rid_of)"; done
  say "RUNS $R_a $R_b $R_c $R_d"
  [ "$(printf '%s\n' "$R_a" "$R_b" "$R_c" "$R_d" | grep -c . )" = 4 ] && [ "$(printf '%s\n' "$R_a" "$R_b" "$R_c" "$R_d" | sort -u | wc -l)" = 4 ] && row ok "four distinct run ids" || { row no "run ids: $R_a $R_b $R_c $R_d"; exit 1; }
  ok=0; for r in $R_a $R_b $R_c $R_d; do s=$(wait_status "$r" 200); [ "$s" = succeeded ] && ok=$((ok+1)); done
  [ "$ok" = 4 ] && row ok "all four chains succeeded" || row no "$ok of 4 chains succeeded"
  N=$(journal | grep -a 'coder.spawned' | grep -o '"session":"[^"]*"' | sort -u | wc -l); [ "$N" = 4 ] && row ok "four distinct agent sessions (one leaf per chain)" || row no "distinct sessions: $N (want 4)"
  for n in a b c d; do eval "w=\$W_$n"; [ "$(cat "$w/repo/NOTES.md" 2>/dev/null)" = "chain ok" ] && row ok "repo $n: its own NOTES.md says 'chain ok'" || row no "repo $n: NOTES.md is [$(cat "$w/repo/NOTES.md" 2>/dev/null | head -1)]"; done
  [ "$(for n in a b c d; do eval "w=\$W_$n"; git -C "$w/repo" rev-list --count HEAD; done | tr -d '\n')" = 1111 ] && row ok "nothing committed in any repo" || row no "a worker committed" ;;
pi)
  make_input a "$CHAINQ"; RID=$(start_raw a | rid_of); [ -n "$RID" ] && row ok "started a 3-level chain with pi at the leaf ($RID)" || { row no "no run id"; exit 1; }
  S=$(wait_status "$RID" 240); [ "$S" = succeeded ] && row ok "the chain SUCCEEDED with pi ($S)" || row no "status $S"
  journal | grep -a 'coder.spawned' | grep -aq '"agent":"pi"' && row ok "the leaf agent is pi (journalled)" || row no "no pi spawn in the journal"
  [ "$(journal | grep -a 'coder.spawned' | grep -o '"pid":[0-9]*' | sort -u | wc -l)" = 1 ] && row ok "exactly ONE agent spawned" || row no "not exactly one spawn"
  [ "$(cat "$W_a/repo/NOTES.md" 2>/dev/null)" = "chain ok" ] && row ok "pi did the real work (NOTES.md: chain ok)" || row no "NOTES.md is [$(cat "$W_a/repo/NOTES.md" 2>/dev/null | head -1)]"
  [ "$(git -C "$W_a/repo" rev-list --count HEAD)" = 1 ] && row ok "nothing committed" || row no "a worker committed" ;;
loop)
  loophost loopcap 2 100; loophost loopctl 6 3
  make_input a "$CHAINQ"; RID=$(start_raw a loopcap | rid_of); [ -n "$RID" ] && row ok "started the looped call, max_entries = 2 ($RID)" || { row no "no run id: $(start_raw a loopcap | head -2 | tr '\n' ' ')"; exit 1; }
  S=$(wait_status "$RID" 300); [ "$S" = failed ] && row ok "the run FAILED at the call's limit ($S)" || row no "status $S (want failed)"
  N=$(journal | grep -a 'coder.spawned' | grep -o '"pid":[0-9]*' | sort -u | wc -l); [ "$N" = 2 ] && row ok "exactly 2 real agent spawns, then the counter stopped it" || row no "$N agent spawns (want 2)"
  journal | grep -a "entered more than max_entries (2)" | head -1 | grep -aq "'docs'" && row ok "the failure NAMES the call and the limit (journal)" || { cli run explain "$RID" | grep -q "max_entries" && row ok "the failure names the limit (explain)" || row no "no max_entries reason in the journal or in explain"; }
  LB=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')
  make_input b "$CHAINQ"; RC=$(start_raw b loopctl | rid_of); [ -n "$RC" ] && row ok "CONTROL: the same loop with max_entries = 6 and loop_budget = 3 started ($RC)" || { row no "CONTROL: no run id"; exit 1; }
  S=$(wait_status "$RC" 300); [ "$S" = failed ] && row ok "CONTROL: it failed too, on a different bound ($S)" || row no "CONTROL: status $S"
  journal | grep -a "entered more than max_entries" | grep -aq "$RC" && row no "CONTROL: the call's counter fired although its limit (6) is above the loop budget (3)" || row ok "CONTROL: the call's counter did NOT fire (limit 6 > loop budget 3)"
  N=$(journal | grep -a 'coder.spawned' | grep -o '"pid":[0-9]*' | sort -u | wc -l); [ "$N" -ge 3 ] && row ok "CONTROL: it ran $N real spawns, more than the capped run's 2" || row no "CONTROL: only $N spawns" ;;
midfail)
  make_input a "$CHAINQ"; OUT=$(start_raw a fchain3); RID=$(echo "$OUT" | rid_of)
  [ -n "$RID" ] && row ok "started the chain whose middle graph refuses ($RID)" || { row no "no run id: $(echo "$OUT" | head -2 | tr '\n' ' ' | cut -c1-200)"; say "COMPOSED-LIMITS $SCEN: $P pass $F fail"; exit 1; }
  S=$(wait_status "$RID" 240); [ "$S" = failed ] || [ "$S" = refused ] && row ok "the run ended $S (not succeeded)" || row no "status $S"
  [ -s "$W_a/repo/NOTES.md" ] && row ok "the LEAF finished its work (NOTES.md), so the failure is at the middle level" || row no "the leaf did not finish: the failure is not where the scenario says"
  EX=$(cli run explain "$RID"); say "explain: $(echo "$EX" | head -2 | tr '\n' ' ' | cut -c1-300)"
  echo "$EX" | grep -q 'via2__bad' && row ok "explain NAMES the refusing node by its call path (via2__bad: the refuse node lives in the middle graph, which the top graph calls as via2)" || row no "explain does not name via2__bad"
  echo "$EX" | grep -q 'the middle graph refused its leaf' && row ok "explain gives the refusal reason" || row no "explain does not give the reason"
  cli run trace "$RID" > "$LOGD/composed-limits-midfail-$TS.trace" 2>&1
  grep -q 'via2__bad' "$LOGD/composed-limits-midfail-$TS.trace" && row ok "the trace has a line for the refusing node" || row no "the trace has no line for via2__bad"
  grep -q 'via2__via1__docs__docs' "$LOGD/composed-limits-midfail-$TS.trace" && row ok "the trace still shows the leaf that ran below it" || row no "the leaf is missing from the trace"
  make_input b "$CHAINQ"; RC=$(start_raw b okchain3 | rid_of); [ -n "$RC" ] && row ok "CONTROL: the same graphs without the refuse node started ($RC)" || { row no "CONTROL: no run id"; exit 1; }
  S=$(wait_status "$RC" 240); [ "$S" = succeeded ] && row ok "CONTROL: it SUCCEEDED, so the refusal above is the refuse node and nothing else" || row no "CONTROL: status $S"
  cli run explain "$RC" | grep -q 'via2__bad' && row no "CONTROL: a run without the refuse node names it" || row ok "CONTROL: explain does not name a refuse node that is not there" ;;
esac
say "COMPOSED-LIMITS $SCEN: $P pass $F fail  (log $LOG)"
[ "$F" = 0 ]