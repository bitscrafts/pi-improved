#!/usr/bin/env bash
# composed-quota-check.sh — the account's spend limit hit INSIDE a called graph, through a PRIVATE daemon with stand-in agents (nothing spends).
# Stand-ins print the CLI's real refusal shape (a stream-json `result` line, is_error true, "You've hit your session limit · resets 8:20pm (UTC)"), because a real
# quota refusal cannot be provoked on demand. Rows (all read from the daemon's own journal / CLI):
#   Q1 the docs coder's refusal: the run does NOT succeed; coder.exited of the docs coder carries quota_exhausted=yes and the reset text; the host `code` coder's does not
#   Q2 the host's work survived (hello.txt) and the trace shows the call
#   Q3 CONTROL (spec 281's near miss): a docs agent that merely READ a file quoting the phrase is NOT flagged and the run succeeds
#   Q4 CONTROL: a refusal with is_error false does not count
# Usage: bash scripts/archive/composed-quota-check.sh      Exit 0 all rows | 1 a row failed | 3 refused
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/bin:$PATH"
ROOT=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)
DT=$ROOT/_tmp/devbox-test; PII=$DT/bin/pii
STANDIN_COST=0.10
# the helper functions (pii_cli, start_daemon, stop_daemon, wait_run, run_check, worker_in_flight) come from the real check, sourced not copied
# shellcheck disable=SC1091
. "$ROOT/modules/devbox/composition-check.sh"
# sourcing re-derives ROOT from ITS OWN $0 (which is this script's path, so it lands one level too high): set them back
ROOT=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd); DT=$ROOT/_tmp/devbox-test; PII=$DT/bin/pii; STANDIN_COST=0.10
P=0; F=0; row() { if [ "$1" = ok ]; then P=$((P+1)); echo "PASS $2"; else F=$((F+1)); echo "FAIL $2"; fi; }

procs=$(worker_in_flight); [ -z "$procs" ] || { echo "STOPPED: a worker is in flight"; echo "$procs" | head -3; echo "QUOTA-CHECK: refused (worker in flight)"; exit 3; }
[ -x "$PII" ] || { echo "QUOTA-CHECK: refused (no daemon binary)"; exit 3; }
head=$(git -C "$ROOT" rev-parse --short=12 HEAD)
s=$DT/quota-$(date +%s); mkdir -p "$s/bin" "$s/run" "$s/graphs" "$s/tmp" "$s/repo/specs"
cat > "$s/bin/claude" <<EOF
#!/usr/bin/env bash
# stand-in claude (quota check): records argv; behaviour chosen by the brief text
echo "\$*" >> "$s/spawns.log"
case "\$*" in *HELLO_PLEASE*) echo PONG > "$s/repo/hello.txt" ;; esac
echo '{"type":"system","subtype":"init","model":"stand-in"}'
case "\$*" in
  *REFUSE_NOW*)   echo '{"type":"result","subtype":"success","is_error":true,"result":"You'"'"'ve hit your session limit · resets 8:20pm (UTC)","total_cost_usd":0}'; exit 1 ;;
  *READ_QUOTE*)   echo '{"type":"user","message":{"content":[{"type":"tool_result","content":"echo You have hit your session limit - resets 8:20pm (UTC)"}]}}' ;;
  *SOFT_REFUSE*)  echo '{"type":"result","subtype":"success","is_error":false,"result":"I read: You have hit your session limit - resets 8:20pm (UTC)","total_cost_usd":0}'; exit 0 ;;
esac
echo '{"type":"result","subtype":"success","is_error":false,"total_cost_usd":$STANDIN_COST}'
EOF
chmod +x "$s/bin/claude"; printf '#!/bin/sh\n' > "$s/bin/pi"; chmod +x "$s/bin/pi"
printf '# Check spec\n\n## Exit Criteria\n\n- [ ] `test -f specs/check.md`\n' > "$s/repo/specs/check.md"
git -C "$s/repo" init -q && git -C "$s/repo" add -A && git -C "$s/repo" -c user.email=q@local -c user.name=q commit -qm base
base=$(git -C "$s/repo" rev-parse HEAD)
cp "$ROOT/graphs/spec.toml" "$ROOT/graphs/docs.toml" "$s/graphs/"
start_daemon "$s" || { echo "QUOTA-CHECK: refused (the private daemon did not answer)"; stop_daemon "$s"; exit 3; }
input() { printf '{"agent":"claude","model":"claude-haiku-5-5","provider":"anthropic","spec":"specs/check.md","base":"%s","workdir":"%s/repo","brief":"HELLO_PLEASE implement the check spec","max_budget_usd":1,"docs_brief":"%s"}' "$base" "$s" "$1"; }
OFF=0
# new_exited: the coder.exited lines journalled since the last call (one run's worth, read right after that run ends), advancing the mark
# It sets NEWX and the mark in THIS shell (a $(...) call would lose the mark).
new_exited() { local n; n=$(wc -l < "$s/pii.log"); NEWX=$(tail -n +$((OFF+1)) "$s/pii.log" | grep -a 'coder.exited'); OFF=$n; }
fieldof() { printf '%s' "$1" | grep -o "\"$2\":\"\\?[^,\"}]*" | head -1 | sed -E 's/^"[a-z_]*":"?//'; }

# Q1 + Q2: the docs coder is refused
OFF=$(wc -l < "$s/pii.log")
run_check "$s" "$(input REFUSE_NOW)"; R1=$RID; S1=$ST; new_exited; E1=$NEWX
[ -n "$R1" ] && [ "$S1" != succeeded ] && row ok "Q1: the run does not succeed when the called docs coder is refused ($S1)" || row no "Q1: run [$R1] status [$S1]"
E=$E1; N=$(printf '%s\n' "$E" | grep -c .)
[ "$N" = 2 ] && row ok "Q1: two coder.exited events (code, then the refused docs coder)" || row no "Q1: coder.exited events: $N"
CODE_L=$(printf '%s\n' "$E" | sed -n 1p); DOCS_L=$(printf '%s\n' "$E" | sed -n 2p)
[ "$(fieldof "$DOCS_L" quota_exhausted)" = yes ] && row ok "Q1: the docs coder's coder.exited says quota_exhausted=yes" || row no "Q1: docs quota_exhausted=[$(fieldof "$DOCS_L" quota_exhausted)]"
[ "$(fieldof "$CODE_L" quota_exhausted)" = no ] && row ok "Q1 CONTROL: the host code coder (normal) says quota_exhausted=no" || row no "Q1 CONTROL: code quota_exhausted=[$(fieldof "$CODE_L" quota_exhausted)]"
printf '%s' "$DOCS_L" | grep -q '8:20pm' && row ok "Q1: the CLI's own reset text is journalled (8:20pm)" || row no "Q1: no reset text in the docs coder's exit"
[ "$(cat "$s/repo/hello.txt" 2>/dev/null)" = PONG ] && row ok "Q2: the host's work survived the refusal (hello.txt PONG)" || row no "Q2: hello.txt missing"
pii_cli "$s" run trace "$R1" | grep -q '^call docs' && row ok "Q2: the trace still shows call docs" || row no "Q2: no call in the trace"

# Q3: a docs agent that READ the phrase
run_check "$s" "$(input READ_QUOTE)"; R3=$RID; S3=$ST; new_exited; E3=$NEWX
[ "$S3" = succeeded ] && row ok "Q3 CONTROL: a docs agent that merely read the phrase does not fail the run ($S3)" || row no "Q3 CONTROL: status $S3"
[ "$(fieldof "$(printf '%s\n' "$E3" | sed -n 2p)" quota_exhausted)" = no ] && row ok "Q3 CONTROL: its coder.exited says quota_exhausted=no" || row no "Q3 CONTROL: flagged though it only read the phrase"
# Q4: is_error false
run_check "$s" "$(input SOFT_REFUSE)"; R4=$RID; S4=$ST; new_exited; E4=$NEWX
[ "$(fieldof "$(printf '%s\n' "$E4" | sed -n 2p)" quota_exhausted)" = no ] && row ok "Q4 CONTROL: a result line with is_error false is not a refusal" || row no "Q4 CONTROL: flagged on is_error false"

[ "$(printf '%s\n' "$E3" | grep -c .)" = 2 ] && [ "$(printf '%s\n' "$E4" | grep -c .)" = 2 ] && row ok "each run's section holds exactly its own two coder.exited events (the selection works)" || row no "section sizes: $(printf '%s\n' "$E3" | grep -c .) and $(printf '%s\n' "$E4" | grep -c .)"
stop_daemon "$s"
PIDGONE=gone; kill -0 "$(cat "$s/daemon.pid")" 2>/dev/null && PIDGONE=alive
[ "$PIDGONE" = gone ] && row ok "the private daemon is stopped by its own pid" || row no "the private daemon is still alive"
env PII_RUNTIME_DIR="$DT/run" "$PII" health --json 2>/dev/null | grep -q '"revision"' && row ok "the production daemon still answers" || row no "the production daemon does not answer"
mkdir -p "$ROOT/_tmp/_trash" && mv "$s" "$ROOT/_tmp/_trash/" 2>/dev/null
echo "QUOTA-CHECK: $P pass $F fail   (HEAD $head)"
[ "$F" = 0 ]
