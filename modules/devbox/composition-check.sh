#!/usr/bin/env bash
# composition-check.sh — spec 287: a composed run through the REAL daemon process, from the CLI, with stand-in agents (shell scripts; nothing spends).
# It starts a PRIVATE daemon (its own PII_RUNTIME_DIR, socket, graph dir and PATH, so the stand-in bin dir comes first) and never touches the
# production daemon. It refuses while a worker (run-batchC, claude, pi) is in flight, except the check's own ancestors. The verdict is a list
# of PASS/FAIL rows read from the daemon's own output, ending `COMPOSITION-CHECK: <n> pass <m> fail`; R3's controls print `CONTROL` lines.
# Usage: bash modules/devbox/composition-check.sh     Exit: 0 every row and control passes | 1 a row or control failed | 3 refused (nothing was run)
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DT=$ROOT/_tmp/devbox-test
EXPECT="a b c d e f g"
STANDIN_COST=0.25

# ---- pure functions (sourced and tested by tests/composition-check-test.sh)
# row LETTER TEXT CODE: one verdict row; CODE 0 passes
row() { if [ "$3" = 0 ]; then echo "PASS $1 $2"; else echo "FAIL $1 $2"; fi; }

# verdict_line FILE: the summary line; an expected row that is missing, or that also has a FAIL, counts as a fail
verdict_line() {
  local p=0 f=0 k
  for k in $EXPECT; do
    if grep -q "^PASS $k " "$1" && ! grep -q "^FAIL $k " "$1"; then p=$((p+1)); else f=$((f+1)); fi
  done
  echo "COMPOSITION-CHECK: $p pass $f fail"
}

# verdict_ok FILE: status 0 only when the verdict has no failures
verdict_ok() { case "$(verdict_line "$1")" in *" 0 fail") return 0 ;; *) return 1 ;; esac; }

# budget_shrinks FIRST_CAP FIRST_COST SECOND_CAP: the second cap is the first less the first's reported cost (to the cent)
budget_shrinks() { awk -v a="$1" -v c="$2" -v b="$3" 'BEGIN { d = b - (a - c); exit !(d < 0.005 && d > -0.005) }'; }

# revision_verdict REV HEAD: row (f). REV is the daemon's build revision; a -dirty suffix is allowed and printed; any other difference is STALE-BINARY
revision_verdict() {
  local base=${1%-dirty} dirty=""
  [ "$base" != "$1" ] && dirty=" (dirty tree)"
  if [ "$base" = "$2" ]; then row f "daemon revision $1 is HEAD $2$dirty" 0
  else row f "STALE-BINARY: daemon $1, HEAD $2" 1; fi
}

# ---- the host side: a worker in flight, the private daemon, the stand-ins
# ancestor_chain PID: PID and its ppid chain up to init, read from /proc/<pid>/stat (the field after the command name's closing paren)
ancestor_chain() {
  local p=$1 i
  for i in $(seq 1 64); do
    echo "$p"
    p=$(sed 's/.*) //' "/proc/$p/stat" 2>/dev/null | awk '{print $2}')
    [ -n "$p" ] && [ "$p" -gt 0 ] 2>/dev/null || return 0
  done
}

# busy_workers TABLE ANCESTOR...: the lines of a `ps -eo pid,ppid,args` TABLE (its header skipped) that are a run-batchC, claude or pi worker,
# minus the ANCESTOR pids and the check's own lines. Any line left is a worker in flight.
busy_workers() {
  local table=$1; shift
  printf '%s\n' "$table" | awk -v anc=" $* " '
    NR == 1 { next }
    index(anc, " " $1 " ") { next }
    { args = $0; sub(/^ *[0-9]+ +[0-9]+ +/, "", args) }
    args ~ /composition-check\.sh/ || args ~ /grep -E/ { next }
    args ~ /(^| |\/)(claude|pi)( |$)/ || args ~ /run-batchC/ { print }
  '
}

# worker_in_flight: the workers the check must not run beside; its own ancestry (the agent that spawned it) is not one of them
worker_in_flight() {
  busy_workers "$(ps -eo pid,ppid,args)" $(ancestor_chain $$)
}

# pii_cli S ARGS...: the CLI against the private daemon's runtime dir (S is the scratch dir)
pii_cli() { local s=$1; shift; env PII_RUNTIME_DIR="$s/run" "$PII" "$@" 2>&1; }

# write_standins DIR S: a stand-in claude and pi. Each records its argv (the budget and session are read back from it) and spends nothing.
# claude prints the stream-json lines the coder reads, with a fixed cost. A brief that names BREAK removes the spec's criterion (the docs
# step's control: a docs step that breaks the criteria).
write_standins() {
  mkdir -p "$1"
  cat > "$1/claude" <<EOF
#!/usr/bin/env bash
# stand-in claude (spec 287): records its argv, spends nothing
echo "\$*" >> "$2/spawns.log"
case "\$*" in *BREAK*) rm -f "$2/repo/specs/check.md" ;; esac
echo '{"type":"system","subtype":"init","model":"stand-in"}'
echo '{"type":"result","subtype":"success","is_error":false,"total_cost_usd":$STANDIN_COST}'
EOF
  printf '#!/usr/bin/env bash\n# stand-in pi (spec 287): records its argv, spends nothing\necho "$*" >> "%s/spawns.log"\n' "$2" > "$1/pi"
  chmod +x "$1/claude" "$1/pi"
}

# start_daemon S: a private daemon; its pid is recorded in S/daemon.pid and it is stopped by that pid only
start_daemon() {
  env PII_RUNTIME_DIR="$1/run" PATH="$1/bin:$PATH" PI_CODING_AGENT_DIR="$1/agent" IS_SANDBOX=1 \
    PII_MODEL=llamacpp-server:granite-4.2-3b CLAUDE_CODE_TMPDIR="$1/tmp" TMPDIR="$1/tmp" \
    "$PII" daemon --foreground --store "$1/store.redb" --workspace "$1/repo" --log-file "$1/pii.log" > "$1/daemon.out" 2>&1 &
  echo $! > "$1/daemon.pid"
  for _ in $(seq 1 30); do pii_cli "$1" health --json | grep -q '"revision"' && return 0; sleep 1; done
  return 1
}

# stop_daemon S: stops the pid recorded at start, and only that pid
stop_daemon() {
  local pid; pid=$(cat "$1/daemon.pid" 2>/dev/null) || return 0
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

# wait_run S RID: the run's terminal status word, or `timeout` after about 10 minutes
wait_run() {
  local i st
  for i in $(seq 1 120); do
    st=$(pii_cli "$1" run get "$2" | grep -o '"status":"[a-z_]*"' | head -1 | cut -d'"' -f4)
    case "$st" in succeeded|failed|refused|cancelled) echo "$st"; return 0 ;; esac
    sleep 5
  done
  echo timeout
}

# sessions S: how many agent spawns the stand-ins recorded (one `--session-id` per spawn; an argv can span lines, so lines are no count)
sessions() { local n; n=$(grep -c -- '--session-id' "$1/spawns.log" 2>/dev/null); echo "${n:-0}"; }

# run_check S INPUT: starts the spec graph on the private daemon with INPUT. Sets RID (empty when refused at start), ST (the terminal
# status, or `none`) and EVID (the evidence file: the start output, then run get, run trace and run explain).
run_check() {
  local out
  out=$(pii_cli "$1" run start spec --input "$2")
  RID=$(printf '%s\n' "$out" | grep -o 'r[0-9]\{12\}' | head -1)
  ST=none
  [ -n "$RID" ] && ST=$(wait_run "$1" "$RID")
  EVID=$1/evidence-${RID:-refused}.txt
  { printf '%s\n' "$out"; [ -n "$RID" ] && { pii_cli "$1" run get "$RID"; pii_cli "$1" run trace "$RID"; pii_cli "$1" run explain "$RID"; }; } > "$EVID"
}

main() {
  local ts s head base rev rows=() n dig pin calls spawn1 spawn2 gone prod controls_ok=0 main_in
  PII=$DT/bin/pii
  local procs; procs=$(worker_in_flight)
  if [ -n "$procs" ]; then
    echo "STOPPED: a worker is in flight; the check does not run beside it:"; echo "$procs" | head -5
    echo "COMPOSITION-CHECK: refused (worker in flight)"; return 3
  fi
  [ -x "$PII" ] || { echo "COMPOSITION-CHECK: refused (no daemon binary at $PII)"; return 3; }
  pin=$(grep -o 'sha256 = "[0-9a-f]*"' "$ROOT/graphs/spec.toml" | head -1 | cut -d'"' -f2)
  dig=$(sha256sum "$ROOT/graphs/docs.toml" | cut -c1-64)
  [ "$dig" = "$pin" ] || { echo "COMPOSITION-CHECK: refused (docs.toml digest $dig is not the pin $pin)"; return 3; }

  ts=$(date +%s); s=$DT/check-$ts
  mkdir -p "$s/bin" "$s/run" "$s/graphs" "$s/tmp" "$s/repo/specs"
  write_standins "$s/bin" "$s"
  printf '# Check spec\n\n## Exit Criteria\n\n- [ ] `test -f specs/check.md`\n' > "$s/repo/specs/check.md"
  git -C "$s/repo" init -q && git -C "$s/repo" add -A
  git -C "$s/repo" -c user.email=check@local -c user.name=check commit -qm base
  base=$(git -C "$s/repo" rev-parse HEAD)
  cp "$ROOT/graphs/spec.toml" "$ROOT/graphs/docs.toml" "$s/graphs/"
  head=$(git -C "$ROOT" rev-parse --short=12 HEAD)

  start_daemon "$s" || { echo "COMPOSITION-CHECK: refused (the private daemon did not answer in 30 s)"; stop_daemon "$s"; return 3; }
  main_in="{\"agent\":\"claude\",\"model\":\"claude-haiku-5-5\",\"provider\":\"anthropic\",\"spec\":\"specs/check.md\",\"base\":\"$base\",\"workdir\":\"$s/repo\",\"brief\":\"implement the check spec\",\"max_budget_usd\":1,\"docs_brief\":\"document the check\"}"
  run_check "$s" "$main_in"

  # the rows, each read from the daemon's own output
  rows+=("$(row a "run Succeeded (status: $ST)" "$([ "$ST" = succeeded ]; echo $?)")")
  calls=$(grep -c '^call docs -> graph docs' "$EVID")
  rows+=("$(row b "run trace shows one call docs (saw $calls)" "$([ "$calls" = 1 ]; echo $?)")")
  grep -q "${dig:0:12}" "$EVID"; rows+=("$(row c "the docs call digest is sha256sum graphs/docs.toml (${dig:0:12})" "$?")")
  [ "$(grep -o -- '--session-id [^ ]*' "$s/spawns.log" 2>/dev/null | sort -u | wc -l)" -ge 2 ]
  rows+=("$(row d "two coder spawns with different sessions" "$?")")
  spawn1=$(grep -o -- '--max-budget-usd [0-9.]*' "$s/spawns.log" 2>/dev/null | sed -n 1p | awk '{print $2}')
  spawn2=$(grep -o -- '--max-budget-usd [0-9.]*' "$s/spawns.log" 2>/dev/null | sed -n 2p | awk '{print $2}')
  budget_shrinks "${spawn1:-0}" "$STANDIN_COST" "${spawn2:-0}"
  rows+=("$(row e "the second budget ($spawn2) is the first ($spawn1) less the reported cost $STANDIN_COST" "$?")")
  rev=$(pii_cli "$s" health --json | grep -o '"revision":"[^"]*"' | head -1 | cut -d'"' -f4)
  rows+=("$(revision_verdict "$rev" "$head")")

  # R3 controls, each a second run on the same private daemon. They print CONTROL lines and do not count toward the verdict.
  run_check "$s" "${main_in/document the check/BREAK the docs step}"
  if [ "$ST" != succeeded ] && grep -q 'reverify' "$EVID"; then echo "CONTROL break PASS: docs step broke the criterion: $ST, reverify named"
  else echo "CONTROL break FAIL: status $ST"; controls_ok=1; fi
  git -C "$s/repo" show "$base:specs/check.md" > "$s/repo/specs/check.md"
  spawns_before=$(sessions "$s")
  run_check "$s" "${main_in/,\"docs_brief\":\"document the check\"/}"
  spawns_after=$(sessions "$s")
  if [ "$spawns_after" = "$spawns_before" ] && { [ -z "$RID" ] || [ "$ST" = refused ]; }; then echo "CONTROL missing-docs_brief PASS: refused before any spawn ($ST)"
  else echo "CONTROL missing-docs_brief FAIL: status $ST, spawns $spawns_before -> $spawns_after"; controls_ok=1; fi
  { cat "$ROOT/graphs/docs.toml"; printf ' '; } > "$s/graphs/docs.toml"
  run_check "$s" "$main_in"
  if [ "$ST" != succeeded ] && grep -qiE 'sha256|pin|digest' "$EVID"; then echo "CONTROL pin PASS: docs.toml with one byte changed refused ($ST)"
  else echo "CONTROL pin FAIL: status $ST"; controls_ok=1; fi

  stop_daemon "$s"
  kill -0 "$(cat "$s/daemon.pid")" 2>/dev/null; gone=$?
  env PII_RUNTIME_DIR="$DT/run" "$PII" health --json 2>/dev/null | grep -q '"revision"'; prod=$?
  rows+=("$(row g "private daemon gone and production daemon answers health" "$([ "$gone" -ne 0 ] && [ "$prod" = 0 ]; echo $?)")")

  printf '%s\n' "${rows[@]}" > "$s/rows.txt"
  cat "$s/rows.txt"
  verdict_line "$s/rows.txt"
  verdict_ok "$s/rows.txt"; n=$?
  mkdir -p "$ROOT/_tmp/_trash" && mv "$s" "$ROOT/_tmp/_trash/"
  [ "$n" = 0 ] && [ "$controls_ok" = 0 ] && return 0 || return 1
}

[ "${BASH_SOURCE[0]}" = "$0" ] && main "$@"
