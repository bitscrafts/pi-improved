#!/usr/bin/env bash
# Spec 417: `scripts/mark-ready.sh` works on a repository that never had a `triage` label, creates `ready` when the
# repository lacks it, and prints its success line only after reading the labels back.
#
# A fake `gh`, first on PATH, is the only `gh` this reaches: it records every argv and holds a tiny state (the
# repository's labels, the issue's labels). Nothing here touches the network or a real issue.
#
# Prints one PASS/FAIL line per case and a final `PASS: n FAIL: m`; exits 0 only when every case passed.
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" || exit 2
cd "$root" || exit 2
SCRIPT="${MARK_READY:-$root/scripts/mark-ready.sh}"
[ -f "$SCRIPT" ] || { echo "FAIL: scripts/mark-ready.sh is missing"; exit 1; }

ok=0; fail=0
pass() { ok=$((ok + 1)); echo "PASS: $1"; }
no() { fail=$((fail + 1)); echo "FAIL: $1"; }

# A fake `gh` in $1/bin. State: $1/repo-labels, $1/issue-labels, plus knobs read from the environment.
fake_gh() {
  local d="$1"
  mkdir -p "$d/bin"
  cat > "$d/bin/gh" <<'FAKE'
#!/usr/bin/env bash
# A fake `gh`: records argv, serves the four shapes mark-ready.sh uses, and honours three knobs.
d="$(dirname "$(dirname "$0")")"
printf '%s\n' "$*" >> "$d/calls.log"
add() { grep -qxF "$1" "$d/$2-labels" 2>/dev/null || echo "$1" >> "$d/$2-labels"; }
drop() { grep -vxF "$1" "$d/$2-labels" > "$d/$2-labels.tmp" 2>/dev/null || :; mv "$d/$2-labels.tmp" "$d/$2-labels"; }
case "$1 $2" in
  "repo view") echo "${FAKE_REPO:-owner/repo}" ;;
  "label list") cat "$d/repo-labels" 2>/dev/null || : ;;
  "label create") add "$3" repo ;;
  "issue view")
    case "$*" in
      *--json\ author*) echo "${FAKE_AUTHOR:-owner}" ;;
      *--json\ state*) echo "${FAKE_STATE:-OPEN}" ;;
      *--json\ labels*)
        out="$(cat "$d/issue-labels" 2>/dev/null | paste -sd, -)"
        # The knobs make the read-back disagree with what the edit claimed to do.
        case "${FAKE_IGNORE_ADD:-0}" in 1) out="$(printf '%s' "$out" | sed 's/ready//g; s/,,*/,/g; s/^,//; s/,$//')" ;; esac
        case "${FAKE_IGNORE_REMOVE:-0}" in 1) case ",$out," in *,triage,*) : ;; *) out="${out:+$out,}triage" ;; esac ;; esac
        printf '%s' "$out" ;;
      *) printf '{}' ;;
    esac ;;
  "issue edit")
    # A label the repository does not have fails the WHOLE call, which is what GitHub does and what broke the old
    # one-call form (`'triage' not found`, measured 2026-11-12). Nothing is changed when that happens.
    for a in "$@"; do
      case "$a" in
        *-label) NEXT_LABEL=1 ;;
        *) if [ "${NEXT_LABEL:-0}" = 1 ]; then
             NEXT_LABEL=0
             grep -qxF "$a" "$d/repo-labels" 2>/dev/null || { echo "'$a' not found" >&2; exit 1; }
           fi ;;
      esac
    done
    case "$*" in
      *--add-label*)    case "${FAKE_IGNORE_ADD:-0}" in 1) : ;; *) add ready issue ;; esac ;;
      *--remove-label*) case "${FAKE_IGNORE_REMOVE:-0}" in 1) : ;; *) drop triage issue ;; esac ;;
    esac ;;
  *) printf '{}' ;;
esac
FAKE
  chmod +x "$d/bin/gh"
}

# One case: a scratch dir with a fake, a starting state, and the environment knobs. Runs mark-ready.sh and records
# its exit code and output; the assertions run in the caller.
run() { # run <tag> <repo-labels-csv> <issue-labels-csv>
  local tag="$1" rl="$2" il="$3"
  D="$(mktemp -d "${TMPDIR:-/tmp}/mark-ready-$tag-XXXXXX")" || exit 2
  fake_gh "$D"
  printf '%s\n' "$rl" | tr ',' '\n' | grep -v '^$' > "$D/repo-labels" || :
  printf '%s\n' "$il" | tr ',' '\n' | grep -v '^$' > "$D/issue-labels" || :
  : > "$D/calls.log"
  OUT="$(PATH="$D/bin:$PATH" bash "$SCRIPT" 7 owner/repo 2>&1)"; RC=$?
}

# A: neither triage nor ready on the repository, and no triage on the issue: ready is created and added, nothing is
# removed. This is the measured failure: the old one-call form named `triage`, which the repository does not have.
run a "spec" ""
if [ "$RC" = 0 ] && grep -qx ready "$D/repo-labels" && grep -qx ready "$D/issue-labels"; then
  grep -q -- '--remove-label' "$D/calls.log" && no "a: removed a label on an issue that carried none" || pass "a_repository_without_triage_gets_ready"
else no "a_repository_without_triage_gets_ready (rc=$RC, out=$OUT)"; fi
grep -q '^label create ready' "$D/calls.log" && pass "a: ready was created" || no "a: ready was not created"

# B: the issue carries triage: it is removed, in its own call.
run b "ready,triage" "triage"
if [ "$RC" = 0 ] && ! grep -qx triage "$D/issue-labels" && grep -qx ready "$D/issue-labels"; then
  [ "$(grep -c -- '--remove-label triage' "$D/calls.log")" = 1 ] && pass "triage_is_removed_in_its_own_call" || no "b: the removal was not exactly one call"
else no "triage_is_removed_in_its_own_call (rc=$RC, out=$OUT)"; fi

# C: a stranger's issue, no --owner-approved: refused, and no label is touched.
D="$(mktemp -d "${TMPDIR:-/tmp}/mark-ready-c-XXXXXX")"; fake_gh "$D"; printf 'ready\n' > "$D/repo-labels"; : > "$D/issue-labels"; : > "$D/calls.log"
OUT="$(PATH="$D/bin:$PATH" FAKE_AUTHOR=stranger bash "$SCRIPT" 7 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 4 ] && ! grep -qE '^(issue edit|label create)' "$D/calls.log"; then pass "CONTROL_a_strangers_issue_sets_no_label"
else no "CONTROL_a_strangers_issue_sets_no_label (rc=$RC, out=$OUT)"; fi

# D: a closed issue: refused, and no label is touched.
D="$(mktemp -d "${TMPDIR:-/tmp}/mark-ready-d-XXXXXX")"; fake_gh "$D"; printf 'ready\n' > "$D/repo-labels"; : > "$D/issue-labels"; : > "$D/calls.log"
OUT="$(PATH="$D/bin:$PATH" FAKE_STATE=CLOSED bash "$SCRIPT" 7 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 3 ] && ! grep -qE '^(issue edit|label create)' "$D/calls.log"; then pass "CONTROL_a_closed_issue_sets_no_label"
else no "CONTROL_a_closed_issue_sets_no_label (rc=$RC, out=$OUT)"; fi

# E: the edit claims success but the read-back shows no ready: non-zero, and no success line.
run e "ready" ""
D2="$D"; rm -rf "$D2"; D="$(mktemp -d "${TMPDIR:-/tmp}/mark-ready-e-XXXXXX")"; fake_gh "$D"; printf 'ready\n' > "$D/repo-labels"; : > "$D/issue-labels"
OUT="$(PATH="$D/bin:$PATH" FAKE_IGNORE_ADD=1 bash "$SCRIPT" 7 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 5 ] && ! printf '%s' "$OUT" | grep -q '^ready: #7'; then pass "CONTROL_a_read_back_that_disagrees_fails"
else no "CONTROL_a_read_back_that_disagrees_fails (rc=$RC, out=$OUT)"; fi

# F: the issue carried triage and the read-back still shows it: non-zero, and no success line.
D="$(mktemp -d "${TMPDIR:-/tmp}/mark-ready-f-XXXXXX")"; fake_gh "$D"; printf 'ready\ntriage\n' > "$D/repo-labels"; printf 'triage\n' > "$D/issue-labels"
OUT="$(PATH="$D/bin:$PATH" FAKE_IGNORE_REMOVE=1 bash "$SCRIPT" 7 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 5 ] && ! printf '%s' "$OUT" | grep -q '^ready: #7'; then pass "CONTROL_an_issue_that_keeps_triage_fails"
else no "CONTROL_an_issue_that_keeps_triage_fails (rc=$RC, out=$OUT)"; fi

echo "PASS: $ok FAIL: $fail"
[ "$fail" = 0 ] && [ "$ok" -ge 6 ]
