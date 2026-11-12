#!/usr/bin/env bash
# requires: dev-tree
# why: it clones /projects/pi-improved by absolute path, so it only means anything in the dev tree
# Test: a resumed row keeps the spec lane (spec 303). The lane runs in a scratch copy of the repo (its REPO is rewritten), with a stub for the
# devbox ssh (PII_SSH) and a stub daemon (PII_DAEMON_SH): no real ssh, daemon or agent. The dry runs write the input and exit before the daemon.
# Controls: no --resume (no sid), and a transcript that IS found (the sid is kept, no "not found" line).
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-resume-XXXXXX") || exit 2
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line
SID=11111111-2222-3333-4444-555555555555

# the scratch repo: the real graphs and the spec, so the real tree is never touched
mkdir -p "$SCRATCH/repo/graphs" "$SCRATCH/repo/specs"
cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$REPO/graphs/preflight.toml" "$SCRATCH/repo/graphs/"
cp "$REPO"/specs/303-*.md "$SCRATCH/repo/specs/"
sed "s|^REPO=/projects/pi-improved|REPO=$SCRATCH/repo|" "$REPO/scripts/run-batchC-pii.sh" > "$SCRATCH/repo/run.sh"
S="$SCRATCH/repo/run.sh"; export PII_LANE_ROOT="$SCRATCH/repo"   # the copy sits at <repo>/run.sh, not under scripts/ (spec 411)

# the devbox stubs: the transcript lookup succeeds only where STUB_FOUND=yes AND the lookup is under the captured location; every run answers at once
cat > "$SCRATCH/ssh-stub.sh" <<'STUB'
#!/usr/bin/env bash
CMD="$*"
printf '%s\n' "$CMD" >> "$SCRATCH_CMDS"
case "$CMD" in
  *"find "*)
    # STUB_TREE: a real find over a fake devbox tree, the devbox-test path mapped onto it; otherwise STUB_FOUND answers
    if [ -n "${STUB_TREE:-}" ]; then LOOK=${CMD//${PII_LANE_ROOT:-/projects/pi-improved}\/_tmp\/devbox-test/$STUB_TREE}; bash -c "${LOOK#*podman exec pii-devbox }"
    else [ "${STUB_FOUND:-yes}" = yes ]; fi ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"stub-sid"}' ;;
  *"run start"*|*"/start-"*) echo "run r000000000463 started" ;;
  *"run get"*) echo '{"status":"succeeded"}' ;;
esac
STUB
printf '#!/usr/bin/env bash\necho "daemon ok (stub)"\n' > "$SCRATCH/daemon-stub.sh"
chmod +x "$SCRATCH/ssh-stub.sh"
export SCRATCH_CMDS="$SCRATCH/ssh-commands.txt"

# the sid the lane wrote into the input JSON, or "absent" (python -I: no import from the script's directory)
input_sid() { # <input.json>
  uv run --no-project python -I -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["sid"] if d.get("sid") else "absent")' "$1" 2>/dev/null
}

echo "== resume with the spec lane: the sid reaches the input and the lane stays spec"
OUT=$(PII_LANE_GRAPH=spec timeout 120 bash "$S" 303 --resume "$SID" --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -qx 'LANE-GRAPH: spec' && [ "$(input_sid "$I")" = "$SID" ]; then
  ok "resume_stays_on_the_spec_lane: LANE-GRAPH spec, input sid $SID"
else bad "resume_stays_on_the_spec_lane: exit $RC, input sid [$(input_sid "$I")]"; printf '%s\n' "$OUT" | head -10; fi
if printf '%s\n' "$OUT" | grep -q 'resume is not supported'; then bad "resume_stays_on_the_spec_lane: the fallback text is still printed"; else ok "the fallback text is gone"; fi

echo "== control: no --resume means no sid"
OUT=$(PII_LANE_GRAPH=spec timeout 120 bash "$S" 303 --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -qx 'LANE-GRAPH: spec' && [ "$(input_sid "$I")" = absent ]; then
  ok "CONTROL_without_resume_has_no_sid: LANE-GRAPH spec, no sid value"
else bad "CONTROL_without_resume_has_no_sid: exit $RC, input sid [$(input_sid "$I")]"; fi

echo "== a transcript that is not in the devbox: the run starts fresh and says so, on stdout AND in the log"
: > "$SCRATCH_CMDS"
OUT=$(STUB_FOUND=no PII_LANE_GRAPH=spec PII_SSH="$SCRATCH/ssh-stub.sh" PII_DAEMON_SH="$SCRATCH/daemon-stub.sh" PII_POLL_SECONDS=0 \
  timeout 300 bash "$S" 303 --resume "$SID" 2>&1); RC=$?
L=$(field "$OUT" LOG)
WANT="RESUME: session $SID not found in the devbox, starting fresh"
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -qxF "$WANT" && [ -f "$L" ] && grep -qxF "$WANT" "$L" && printf '%s\n' "$OUT" | grep -q '^RUN      :'; then
  ok "missing_transcript_starts_fresh_and_says_so: exit 0, the line on stdout and in the log, the run started"
else bad "missing_transcript_starts_fresh_and_says_so: exit $RC"; printf '%s\n' "$OUT" | tail -12; fi

echo "== control: the transcript is found, so the run resumes and the sid is kept"
: > "$SCRATCH_CMDS"
OUT=$(STUB_FOUND=yes PII_LANE_GRAPH=spec PII_SSH="$SCRATCH/ssh-stub.sh" PII_DAEMON_SH="$SCRATCH/daemon-stub.sh" PII_POLL_SECONDS=0 \
  timeout 300 bash "$S" 303 --resume "$SID" 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && ! printf '%s\n' "$OUT" | grep -q 'not found in the devbox' && [ "$(input_sid "$I")" = "$SID" ]; then
  ok "CONTROL_found_transcript_resumes: no not-found line, sid kept"
else bad "CONTROL_found_transcript_resumes: exit $RC, input sid [$(input_sid "$I")]"; fi

echo "== the lookup uses the location the session was captured in, not /root/.claude"
if grep -q 'devbox-test/claude' "$SCRATCH_CMDS" && ! grep -q 'devbox-test/tmp' "$SCRATCH_CMDS" && ! grep -q '/root/.claude' "$SCRATCH_CMDS"; then
  ok "lookup_uses_the_captured_location: searches _tmp/devbox-test/claude (the daemon's config dir), not tmp and not /root/.claude"
else bad "lookup_uses_the_captured_location: commands were:"; cat "$SCRATCH_CMDS"; fi

echo "== a directory named for the session is not a transcript: the run starts fresh (coder lane, the graph is the evidence)"
TREE="$SCRATCH/tree"; mkdir -p "$TREE/tmp/claude-0/p/$SID/tasks"
OUT=$(STUB_TREE="$TREE" PII_LANE_GRAPH=coder PII_SSH="$SCRATCH/ssh-stub.sh" PII_DAEMON_SH="$SCRATCH/daemon-stub.sh" PII_POLL_SECONDS=0 \
  timeout 300 bash "$S" 303 --resume "$SID" 2>&1); RC=$?
G=$(field "$OUT" GRAPH)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q 'not found' && ! { [ -f "$G" ] && grep -q "^sid = " "$G"; }; then
  ok "empty_tasks_directory_is_not_a_transcript: not found, no sid line in the graph"
else bad "empty_tasks_directory_is_not_a_transcript: exit $RC"; printf '%s\n' "$OUT" | tail -8; fi

echo "== a real conversation file under the config dir: the run resumes with the sid"
TREE="$SCRATCH/tree2"; mkdir -p "$TREE/claude/projects/p" && : > "$TREE/claude/projects/p/$SID.jsonl"
OUT=$(STUB_TREE="$TREE" PII_LANE_GRAPH=coder PII_SSH="$SCRATCH/ssh-stub.sh" PII_DAEMON_SH="$SCRATCH/daemon-stub.sh" PII_POLL_SECONDS=0 \
  timeout 300 bash "$S" 303 --resume "$SID" 2>&1); RC=$?
G=$(field "$OUT" GRAPH)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q 'RESUMING' && [ -f "$G" ] && grep -qxF "sid = \"$SID\"" "$G"; then
  ok "real_transcript_resumes: RESUMING, sid = \"$SID\" in the graph"
else bad "real_transcript_resumes: exit $RC"; printf '%s\n' "$OUT" | tail -8; fi

mkdir -p "$REPO/_tmp/_trash" && mv "$SCRATCH" "$REPO/_tmp/_trash/lane-resume-$(date +%s)"
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" = 0 ]
