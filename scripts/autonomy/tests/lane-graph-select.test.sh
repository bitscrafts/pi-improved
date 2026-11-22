#!/usr/bin/env bash
# requires: dev-tree
# why: it runs run-batchC-pii.sh, which pins the lane graph against git HEAD of /projects/pi-improved; an export has no .git
# Test: run-batchC-pii.sh selects its lane graph (spec 294). The cases run in a scratch git repository of this test's own (spec 441): the
# lane compares its graphs with the git HEAD of the repository it runs in, so a test that starts it must own that HEAD. The three controls
# are REAL starts (no --dry-run, no PII_GRAPH_EDIT_OK) with the devbox's ssh and daemon stubbed, so nothing leaves this machine; the other
# cases dry-run, which prints the lane and writes the graph or the input and exits BEFORE the daemon, ssh or any agent.
# The default one-node graph must stay byte-identical to the one the script wrote
# before the lane existed (the only difference allowed is the run's timestamp in its name). Controls: a changed pin, and a cap over the limit.
set -uo pipefail
# The cycle runs a row's criteria with env.sh sourced, and env.sh exports PII_LANE_GRAPH from the knob file (spec 295). This test expects the DEFAULT lane, so it pins the variable itself.
unset PII_LANE_GRAPH
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
# uv (the input builder) keeps its cache and managed Pythons under the repo's _tmp when the environment gives it no writable home
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
S=$REPO/scripts/run-batchC-pii.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-select-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; for f in "$@"; do [ -e "$f" ] && mv "$f" "$REPO/_tmp/_trash/lane-select-$(date +%s)-$(basename "$f")"; done; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line
norm() { sed -e 's/^name = "q294-[0-9]*"$/name = "q294-TS"/' -e "s|$REPO|<ROOT>|g" -e "s|/projects/pi-improved|<ROOT>|g" "$1"; }   # the lane script writes the root it runs in (spec 411): compare with the root masked on both sides

# the graph the script wrote BEFORE the lane existed (captured from the pre-294 script; brief and spec are fixed text)
cat > "$SCRATCH/golden.toml" <<'GOLDEN'
name = "q294-TS"
entry = "code"
caps = ["spawn", "proc"]

[limits]
max_wall_ms = 3300000

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "claude"
model = "claude-haiku-5-5"
max_budget_usd = 3.00

workdir = "/projects/pi-improved"
spec = "specs/294-the-queue-worker-can-start-the-composed-spec-graph.md"
brief = '''
Implement spec 294-the-queue-worker-can-start-the-composed-spec-graph (the spec file is the CONTRACT; its path is named at the end of this prompt).
Follow it exactly: TDD (see the RED fail before the GREEN), minimal diffs.
Do NOT run the whole workspace gate: the loop runs quality-gate.sh after you and it is the arbiter. Before you finish run the cheap half:
    cargo fmt --all
    cargo clippy -p <each crate you touched> --all-targets -- -D warnings
and run only the tests your spec names (cargo test -p <crate> --test <name>). Keep the turn short.
HARD RULES: never create, edit or rewrite anything under specs/ (read-only for implementers). Do NOT commit and do not run git commit/push.
Destructive commands are banned: never rm -rf, never git checkout/restore ., git reset --hard or a bare git stash in this shared tree,
and never touch /dev, /proc, /sys or /root. Scratch you create goes under /projects/_tmp/ and only that may be removed, with a guarded script.
Work in the foreground; no background processes. If you run out of budget, write a COMPLETE handoff (done/remaining/next-start) to
/projects/pi-improved/_tmp/claude-batchC/HANDOFF-294-the-queue-worker-can-start-the-composed-spec-graph.md and stop honestly; never compress the work to fake completion.
Final line: SPEC-DONE 294 criteria=<passed>/<total>, with every criterion command and its exit code listed.
'''
env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR", "CLAUDE_CONFIG_DIR"]
timeout_ms = 3000000

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "refuse"
hook = "coder"
reason = "the agent failed"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
GOLDEN

echo "== default: no PII_LANE_GRAPH, dry run"
OUT=$(timeout 60 bash "$S" 294 --dry-run 2>&1); RC=$?
G=$(field "$OUT" GRAPH)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: coder$' && [ -f "$G" ]; then
  if diff <(norm "$G") <(norm "$SCRATCH/golden.toml") >/dev/null; then ok "default_is_the_one_node_graph: LANE-GRAPH coder, graph identical to the pre-294 graph"
  else bad "default_is_the_one_node_graph: the graph differs from the pre-294 graph"; diff <(norm "$G") <(norm "$SCRATCH/golden.toml") | head -20; fi
  trash "$G"
else bad "default_is_the_one_node_graph: exit $RC, LANE-GRAPH line or graph missing"; printf '%s\n' "$OUT" | head -20; fi

# ---- the scratch repository (spec 441): the lane refuses to start when graphs/spec.toml differs from the git HEAD of the repository
# it runs in, so a test that starts it must own that HEAD. Everything below runs in a scratch copy that is a git repository of its
# own: its first commit carries the copied tree, and the graphs are committed in it. "Differs from HEAD" then compares against a HEAD
# this test made, not against the checkout's — which a row that edits a graph cannot commit until its own gate is green.
mkdir -p "$SCRATCH/repo/graphs" "$SCRATCH/repo/specs" "$SCRATCH/bin"
cp "$REPO"/graphs/*.toml "$REPO/graphs/minimal.txt" "$SCRATCH/repo/graphs/"
cp "$REPO"/specs/294-*.md "$SCRATCH/repo/specs/"
cp "$S" "$SCRATCH/repo/run.sh"   # the copy takes its root from PII_LANE_ROOT (spec 411), set on every run below, not from a sed on a line that no longer exists
(
  cd "$SCRATCH/repo" || exit 2
  git init -q .
  git -c user.name="pii-lane-test" -c user.email="lane-test@localhost" add -A
  # --no-verify: a throwaway fixture is not a change to certify, and a global pre-commit hook must not decide whether it commits
  git -c user.name="pii-lane-test" -c user.email="lane-test@localhost" commit -q --no-verify -m "the tree the lane runs in"
)
# the guard refuses an UNTRACKED graph as well, so the graphs are added by name and the index is proven clean before the start
git -C "$SCRATCH/repo" add graphs
[ -z "$(git -C "$SCRATCH/repo" status --porcelain -- graphs/)" ] \
  || { echo "STOPPED: the scratch graphs are not all committed (the lane would refuse them as untracked or modified)"; exit 2; }
L="$SCRATCH/repo/run.sh"

# the devbox stubs for the real starts below: a real start needs a daemon and an ssh, and neither may leave this machine. The stub
# answers from the command it is given; PII_SSH is the lane's own knob (spec 302), so the stub is the ssh the lane really uses.
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/usr/bin/env bash
cmd="${*: -1}"
case "$cmd" in
  *"run start preflight"*) echo "run r000000000418 started" ;;
  *"run get r000000000418"*) echo '{"status":"succeeded"}' ;;
  *"run get"*) echo '{"status":"succeeded"}' ;;
  *"run cancel"*) echo "cancel stub: cancelled" ;;
  *"run trace"*) echo "" ;;
  *"run explain"*) echo "explain stub" ;;
  *"start-"*|*"run start"*) echo "run r000000000419 started" ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"sid-441"}' ;;
  *"coder.exited"*) echo '{"kind":"coder.exited","rc":"0","tree_changed":"yes","quota_exhausted":"no"}' ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/ssh"
cat > "$SCRATCH/daemon.sh" <<'DSTUB'
#!/usr/bin/env bash
echo '"revision":"stub000000000"'
DSTUB
chmod +x "$SCRATCH/daemon.sh"
# lane_real <out-file> [VAR=value ...]: a REAL start (no --dry-run, no PII_GRAPH_EDIT_OK) of the scratch repo's lane. stdout+stderr in
# <out-file>, the exit status in <out-file>.rc. The property under test is what a real lane does with a HEAD the test owns.
lane_real() {
  local out=$1; shift
  env "$@" PII_LANE_ROOT="$SCRATCH/repo" PII_LANE_GRAPH=spec PII_POLL_SECONDS=0 \
      PII_SSH="$SCRATCH/bin/ssh" PII_DAEMON_SH="$SCRATCH/daemon.sh" PATH="$SCRATCH/bin:$PATH" \
      timeout 120 bash "$L" 294 > "$out" 2>&1
  echo $? > "$out.rc"
}

echo "== spec lane: PII_LANE_GRAPH=spec, dry run"
OUT=$(PII_LANE_ROOT="$SCRATCH/repo" PII_LANE_GRAPH=spec timeout 60 bash "$L" 294 --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: spec$' && [ -f "$I" ]; then
  KEYS=$(uv run --no-project python -I -c 'import json,sys; print(" ".join(sorted(json.load(open(sys.argv[1])))))' "$I" 2>&1)
  WANT="agent base brief docs_brief max_budget_usd method model provider sid spec workdir"
  if [ "$KEYS" = "$WANT" ]; then ok "spec_graph_is_selected: LANE-GRAPH spec, input with all eleven keys (spec 385 added 'method')"
  else bad "spec_graph_is_selected: input keys [$KEYS], want [$WANT]"; fi
  trash "$I"
else bad "spec_graph_is_selected: exit $RC, LANE-GRAPH line or input missing"; printf '%s\n' "$OUT" | head -20; fi

echo "== resume with the spec lane stays on the spec lane and hands the session to the graph (spec 303 R1; this case replaces the 294 fallback)"
OUT=$(PII_LANE_ROOT="$SCRATCH/repo" PII_LANE_GRAPH=spec timeout 60 bash "$L" 294 --resume abc --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: spec$' \
   && ! printf '%s\n' "$OUT" | grep -q 'resume is not supported' \
   && [ -f "$I" ] && grep -q '"sid": *"abc"' "$I"; then ok "resume_stays_on_the_spec_lane: LANE-GRAPH spec, sid in the input, no fallback line"
else bad "resume_stays_on_the_spec_lane: exit $RC"; printf '%s\n' "$OUT" | head -20; fi
trash "$I" "$(field "$OUT" GRAPH)"

# the controls below run in the scratch repository built above; its REPO is rewritten, so the real graphs/ and specs/ are never touched.
# They are REAL starts (no --dry-run, no PII_GRAPH_EDIT_OK): the property is what a real lane does with a HEAD this test owns.
echo "== control: a real start on a clean HEAD is accepted (proves the refusal below is not a broken script)"
O="$SCRATCH/clean.out"; lane_real "$O"; RC=$(cat "$O.rc"); OUT=$(cat "$O")
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^RUN      : ' && ! printf '%s\n' "$OUT" | grep -q 'DRY RUN'; then
  ok "control_unchanged_pin_is_accepted: a real start reached a run (exit 0, not a dry run)"
else bad "control_unchanged_pin_is_accepted: exit $RC"; printf '%s\n' "$OUT" | head -10; fi

echo "== control: docs.toml edited after the commit, so the scratch HEAD no longer carries it"
printf ' ' >> "$SCRATCH/repo/graphs/docs.toml"
O="$SCRATCH/edited.out"; lane_real "$O"; RC=$(cat "$O.rc"); OUT=$(cat "$O")
if [ "$RC" = 9 ] && printf '%s\n' "$OUT" | grep -q '^ABORT' && printf '%s\n' "$OUT" | grep -q 'differs from HEAD' \
   && ! printf '%s\n' "$OUT" | grep -qE '^(DAEMON|RUN)'; then ok "CONTROL_a_changed_pin_is_refused: exit 9 (refused before any work), nothing started"
else bad "CONTROL_a_changed_pin_is_refused: exit $RC"; printf '%s\n' "$OUT" | head -10; fi
cp "$REPO/graphs/docs.toml" "$SCRATCH/repo/graphs/docs.toml"   # back to the committed content for the next real start

echo "== control: PII_MAX_BUDGET_USD=9 is capped at the graph's own limit"
O="$SCRATCH/cap.out"; lane_real "$O" PII_MAX_BUDGET_USD=9; RC=$(cat "$O.rc"); OUT=$(cat "$O")
I=$(field "$OUT" INPUT)
CAP=$( [ -f "$I" ] && uv run --no-project python -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["max_budget_usd"])' "$I" 2>/dev/null)
LIMIT=$(sed -n 's/^max_micro_usd *= *\([0-9]*\).*/\1/p' "$REPO/graphs/spec.toml" | head -1 | awk '{printf "%.1f", $1/1000000}')
if [ "$RC" = 0 ] && [ -n "$LIMIT" ] && [ "$CAP" = "$LIMIT" ]; then ok "CONTROL_cap_never_exceeds_the_limit: max_budget_usd $CAP (the tracked limit in graphs/spec.toml, asked for 9)"
else bad "CONTROL_cap_never_exceeds_the_limit: exit $RC, max_budget_usd [$CAP], want the tracked limit [$LIMIT]"; fi

# the docs coder's note must land where git ignores it, or the unlisted-changes guard refuses the row's own commit (measured on the first composed lane run)
OUT=$(PII_LANE_ROOT="$SCRATCH/repo" PII_LANE_GRAPH=spec bash "$L" 294 --dry-run 2>&1); IN=$(field "$OUT" INPUT)
NOTE=$(uv run --no-project python -c 'import json,sys; print(json.load(open(sys.argv[1]))["docs_brief"])' "$IN" 2>/dev/null | grep -o '_tmp/docs-notes/NOTES-294.md' | head -1)
if [ -n "$NOTE" ] && git check-ignore -q "$NOTE"; then ok "the docs note lands in an ignored path ($NOTE)"; else bad "the docs note lands in an ignored path (got [$NOTE])"; fi
if printf '%s' "$(uv run --no-project python -c 'import json,sys; print(json.load(open(sys.argv[1]))["docs_brief"])' "$IN" 2>/dev/null)" | grep -q 'in the repository root'; then bad "CONTROL: the brief still says the repository root"; else ok "CONTROL: the brief no longer says the repository root"; fi
trash "$SCRATCH"
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
