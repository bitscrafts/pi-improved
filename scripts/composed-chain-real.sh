#!/usr/bin/env bash
# The one composition run that had never been executed with a REAL agent: a three-level chain  chain3 -> chain2 -> chain1 -> docs,  whose leaf is a real claude, through the production devbox daemon (real transport).
#   scripts/composed-chain-real.sh        (spend: claude-haiku-5-5, a few cents)
# Rows: A) the whole chain succeeds with exactly ONE agent spawn and the leaf's real work on disk;  B) a real mid-chain cancel kills the real leaf (paired with an "agent was ALIVE before" row);  C) a one-byte change to a middle graph is REFUSED by its pin (control: A, with the same files, succeeded).
# The loop must be paused and no worker in flight. Nothing is committed by a worker.
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
REPO=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}; SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii; GD=$REPO/_tmp/devbox-test/graphs
TS=$(date +%s); LOGD=$REPO/_tmp/claude-batchC; mkdir -p "$LOGD"; LOG=$LOGD/composed-chain-$TS.log
X() { "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; }
say() { echo "$*" | tee -a "$LOG"; }
P=0; F=0; row() { if [ "$1" = ok ]; then P=$((P+1)); say "PASS $2"; else F=$((F+1)); say "FAIL $2"; fi; }
cli() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB $*  2>&1'"; }
alive() { X sh -c "'kill -0 $1 2>/dev/null && echo alive || echo gone'" | tail -1; }
status_of() { cli run get "$1" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4; }
wait_status() { local i s; for i in $(seq 1 "${2:-240}"); do s=$(status_of "$1"); case "$s" in succeeded|failed|refused|cancelled) echo "$s"; return; esac; sleep 3; done; echo timeout; }
journal() { "${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LB:-0}+1)) $C/pii.log'" 2>/dev/null; }
wait_spawn() { local i p; for i in $(seq 1 "${2:-120}"); do p=$(journal | grep -a 'coder.spawned' | sed -n "${1}p" | grep -o '"pid":[0-9]*' | head -1 | cut -d: -f2); [ -n "$p" ] && { echo "$p"; return; }; sleep 0.5; done; }
eval "$(grep -E '^(SLOW|QUICK)=' "$REPO/scripts/composed-real-faults.sh")"
[ -n "${SLOW:-}" ] && [ -n "${QUICK:-}" ] || { say "ABORT: the docs briefs were not found"; exit 2; }

head -1 "$REPO/_tmp/autonomy/control" | grep -q '^PAUSED' || { say "ABORT: the loop is not paused"; exit 3; }
pgrep -af 'run-batchC|claude ' | grep -v 'grep\|flock\|composed-chain' | head -2 | grep . && { say "ABORT: a worker is in flight"; exit 3; }
HEAD=$(git -C "$REPO" rev-parse --short=12 HEAD)
DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
say "== composed-chain-real $TS: claude/claude-haiku-5-5; HEAD $HEAD daemon $REV"
case "$REV" in "$HEAD"*) ;; *) say "ABORT: the daemon is not at HEAD"; exit 2;; esac

# ---- the three graphs, each pinning the one below it by digest (a pin is the sha256 of the file)
cp "$REPO/graphs/docs.toml" "$GD/docs.toml" || exit 2
NEEDS='needs = ["agent", "model", "provider", "spec", "workdir", "docs_brief"]'
D0=$(sha256sum "$GD/docs.toml" | cut -d' ' -f1)
printf 'name = "chain1"\nentry = "docs"\nexit = "out"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "docs"\nkind = "call"\ngraph = "docs"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[node.in]\nbrief = "$.input.docs_brief"\n\n[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "docs"\nport = "ok"\nto = "out"\nkind = "data"\n' "$NEEDS" "$D0" > "$GD/chain1.toml"
D1=$(sha256sum "$GD/chain1.toml" | cut -d' ' -f1)
printf 'name = "chain2"\nentry = "via1"\nexit = "out"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "via1"\nkind = "call"\ngraph = "chain1"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "out"\nkind = "transform"\nexpr = "$.payload"\n\n[[edge]]\nfrom = "via1"\nport = "ok"\nto = "out"\nkind = "data"\n' "$NEEDS" "$D1" > "$GD/chain2.toml"
D2=$(sha256sum "$GD/chain2.toml" | cut -d' ' -f1)
printf 'name = "chain3"\nentry = "via2"\n%s\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "via2"\nkind = "call"\ngraph = "chain2"\nsha256 = "%s"\ncaps = ["spawn", "proc"]\n\n[[node]]\nid = "fin"\nkind = "transform"\nvalue = "done"\n\n[[edge]]\nfrom = "via2"\nport = "ok"\nto = "fin"\nkind = "data"\n' "$NEEDS" "$D2" > "$GD/chain3.toml"
cp "$GD/chain1.toml" "$GD/chain1.toml.orig"
say "graphs: docs ${D0:0:8} <- chain1 ${D1:0:8} <- chain2 ${D2:0:8} <- chain3 (top); depth 3 calls"
LB=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')

make_input() {   # NAME DOCS_BRIEF : a scratch repo and its input file; sets W_<NAME>
  local name=$1 W=$REPO/_tmp/devbox-test/chain-$TS-$1
  mkdir -p "$W/repo/specs"; printf '# Spec: chain\n\n## Exit Criteria\n- [ ] `true`\n' > "$W/repo/specs/real.md"
  git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=f@local -c user.name=f commit -qm base
  uv run --no-project python - "$W/input.json" "$name" "$2" <<'PY'
import json, sys
p, name, dbrief = sys.argv[1:4]
open(p, "w").write(json.dumps({"agent": "claude", "model": "claude-haiku-5-5", "provider": "anthropic", "spec": "specs/real.md",
  "workdir": f"/projects/pi-improved/_tmp/devbox-test/chain-{__import__('os').environ['TS']}-{name}/repo", "docs_brief": dbrief, "max_budget_usd": 1.0}))
PY
  eval "W_$name=$W"
}
export TS
start_raw() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start chain3 --input \"\$(cat /projects/pi-improved/_tmp/devbox-test/chain-$TS-$1/input.json)\" 2>&1'"; }
rid_of() { grep -o 'r[0-9]\{12\}' | head -1; }

CHAINQ='Create NOTES.md in the repository root containing exactly one line: chain ok. Do not commit and do not edit anything else.'
# ---- A: the whole chain, real leaf
make_input a "$CHAINQ"; OUT=$(start_raw a); RA=$(echo "$OUT" | rid_of); say "A start: $(echo "$OUT" | head -2 | tr '\n' ' ' | cut -c1-160)"
[ -n "$RA" ] && row ok "A: the three-level chain started as one run ($RA)" || { row no "A: no run id"; say "COMPOSED-CHAIN: $P pass $F fail"; exit 1; }
SA=$(wait_status "$RA" 240); [ "$SA" = succeeded ] && row ok "A: the chain SUCCEEDED ($SA)" || row no "A: status $SA"
N=$(journal | grep -a 'coder.spawned' | grep -o '"pid":[0-9]*' | sort -u | wc -l); [ "$N" = 1 ] && row ok "A: exactly ONE agent spawned at the leaf (not one per level)" || row no "A: $N agent spawns (want 1)"
[ -s "$W_a/repo/NOTES.md" ] && row ok "A: the real leaf agent did real work (NOTES.md: $(wc -l < "$W_a/repo/NOTES.md") line(s))" || row no "A: NOTES.md is missing: the leaf did nothing"
[ "$(git -C "$W_a/repo" rev-list --count HEAD)" = 1 ] && row ok "A: nothing was committed" || row no "A: a worker committed"
cli run trace "$RA" > "$LOGD/composed-chain-$TS.A.trace" 2>&1
for k in via2 via1 docs; do grep -q "$k" "$LOGD/composed-chain-$TS.A.trace" && row ok "A: the trace names the call level '$k'" || row no "A: the trace lacks '$k'"; done
grep -q ' 0 orphans' "$LOGD/composed-chain-$TS.A.trace" && row ok "A: run trace: 0 orphans" || row no "A: orphans reported"

# ---- B: a real cancel in the middle of the chain
make_input b "$SLOW"; OUT=$(start_raw b); RB=$(echo "$OUT" | rid_of)
[ -n "$RB" ] && row ok "B: second chain run started ($RB)" || { row no "B: no run id"; say "COMPOSED-CHAIN: $P pass $F fail"; exit 1; }
LB_B=$LB; P1=$(wait_spawn 2 240)
[ -n "$P1" ] && row ok "B: the leaf agent spawned (pid $P1)" || { row no "B: the leaf never spawned"; P1=0; }
A=$(alive "$P1"); [ "$A" = alive ] && row ok "B CONTROL: the real leaf agent is ALIVE before the cancel (pid $P1)" || row no "B CONTROL: the leaf is $A before the cancel"
say "cancel: $(cli run cancel "$RB" --kill --reason chain-test | head -2 | tr '\n' ' ' | cut -c1-160)"
ST=$(wait_status "$RB" 60); say "B status after the cancel: $ST"
journal | grep -a "\"run\":\"$RB\"" | grep -aq '"kind":"run.cancelled"' && row ok "B: run.cancelled is journalled" || row no "B: no run.cancelled in the journal"
sleep 3; A=$(alive "$P1"); [ "$A" = gone ] && row ok "B: the real leaf agent is GONE after the cancel (pid $P1)" || row no "B: the leaf $P1 is still $A"
[ -e "$W_b/repo/NOTES.md" ] && [ "$(wc -l < "$W_b/repo/NOTES.md")" -ge 20 ] && row no "B: the leaf finished its work before the cancel: nothing was proven" || row ok "B: the leaf had not finished (cancelled mid-work)"
cli run trace "$RB" > "$LOGD/composed-chain-$TS.B.trace" 2>&1; grep -q ' 0 orphans' "$LOGD/composed-chain-$TS.B.trace" && row ok "B: run trace: 0 orphans" || row no "B: orphans reported"

# ---- C: tamper with the MIDDLE graph; the pin must refuse it
printf '# one extra line\n' >> "$GD/chain1.toml"
make_input c "$CHAINQ"; OUT=$(start_raw c); RC=$(echo "$OUT" | rid_of); say "C start: $(echo "$OUT" | head -3 | tr '\n' ' ' | cut -c1-260)"
if [ -z "$RC" ]; then echo "$OUT" | grep -qiE 'sha256|pin|digest|graph.call' && row ok "C: a one-line change to the middle graph is REFUSED, naming the pin" || row no "C: no run, but the refusal does not name the pin"
else row no "C: the tampered chain STARTED as $RC"; fi
mv "$GD/chain1.toml.orig" "$GD/chain1.toml"
[ "$(sha256sum "$GD/chain1.toml" | cut -d' ' -f1)" = "$D1" ] && row ok "C: the graph is restored byte for byte (A ran on the same file)" || row no "C: chain1 was not restored"
X sh -c "'kill -9 ${P1:-0} 2>/dev/null; true'" >/dev/null 2>&1
say "COMPOSED-CHAIN: $P pass $F fail  (log $LOG)"
[ "$F" = 0 ]