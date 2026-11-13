#!/usr/bin/env bash
# Spec 419 + 421: `scripts/report-row.sh` reports a landed row on the issue it came from, closes the issue only when the
# row is done, and decides "already reported" from a comment's FIRST LINE — so a person who quotes pii's comment in a
# reply is not mistaken for pii.
#
# A fake `gh` first on PATH is the only GitHub this reaches, and `PII_STATE` points at a fixture state file, so nothing
# here touches the network, the real queue state or a real issue. Comments are stored one file each, and the fake answers
# the same jq expression report-row.sh asks for (the first line of every comment), so the fixture cannot be more
# forgiving than the real API.
#
# Prints one PASS/FAIL line per case and a final `PASS: n FAIL: m`; exits 0 only when every case passed.
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" || exit 2
cd "$root" || exit 2
SCRIPT="$root/scripts/report-row.sh"
[ -f "$SCRIPT" ] || { echo "FAIL: scripts/report-row.sh is missing"; exit 1; }

ok=0; fail=0
pass() { ok=$((ok + 1)); echo "PASS: $1"; }
no() { fail=$((fail + 1)); echo "FAIL: $1"; }

fake_gh() { # fake_gh <dir>
  mkdir -p "$1/bin" "$1/comments"
  cat > "$1/bin/gh" <<'FAKE'
#!/usr/bin/env bash
d="$(dirname "$(dirname "$0")")"
printf '%s\n' "$*" >> "$d/calls.log"
state() { echo "$(cat "$d/state" 2>/dev/null || echo OPEN)"; }
case "$1 $2" in
  "repo view") echo "${FAKE_REPO:-owner/repo}" ;;
  "issue view")
    case "$*" in
      # report-row.sh asks for `.comments[] | .body | split("\n")[0]`: the first line of each comment.
      *--json\ comments*) for f in "$d"/comments/*; do [ -e "$f" ] && sed -n 1p "$f"; done ;;
      *--json\ state*)    state ;;
      *) printf '{}' ;;
    esac ;;
  "issue comment")
    body=""; prev=""
    for a in "$@"; do [ "$prev" = "--body" ] && body="$a"; prev="$a"; done
    case "${FAKE_IGNORE_COMMENT:-0}" in 1) : ;; *) n=$(ls "$d/comments" | wc -l); printf '%s\n' "$body" > "$d/comments/$((n + 1))" ;; esac ;;
  "issue close")
    case "${FAKE_IGNORE_CLOSE:-0}" in 1) : ;; *) echo CLOSED > "$d/state" ;; esac ;;
  *) printf '{}' ;;
esac
FAKE
  chmod +x "$1/bin/gh"
}

fixture() { # fixture <tag> <status> <commit>
  D="$(mktemp -d "${TMPDIR:-/tmp}/report-row-$1-XXXXXX")" || exit 2
  fake_gh "$D"
  echo OPEN > "$D/state"; : > "$D/calls.log"
  STATE="$D/state.json"
  printf '{"7": {"status": "%s", "commit": "%s", "spec": "specs/7-a-thing.md"}}' "$2" "$3" > "$STATE"
}
run() { OUT="$(PATH="$D/bin:$PATH" PII_STATE="$STATE" bash "$SCRIPT" "${@:1}" 2>&1)"; RC=$?; }
count() { ls "$D/comments" | wc -l; }

# A: a done row comments once, names row/status/commit/spec on its first line, marks it on the second, and closes.
fixture a done abc1234
run 7 --issue 42 owner/repo
if [ "$RC" = 0 ] && [ "$(count)" = 1 ] && [ "$(sed -n 1p "$D/comments/1")" = "pii: row 7 done at abc1234 — specs/7-a-thing.md" ] \
   && sed -n 2p "$D/comments/1" | grep -q '^<!-- pii kind=row row=7 commit=abc1234'; then
  [ "$(cat "$D/state")" = CLOSED ] && grep -q '^issue close 42 .*--reason completed' "$D/calls.log" \
    && pass "a_done_row_comments_and_closes" || no "a_done_row_comments_and_closes: not closed (rc=$RC, out=$OUT)"
else no "a_done_row_comments_and_closes (rc=$RC, out=$OUT)"; fi

# B: a pending row comments and stays open.
fixture b pending -
run 7 --issue 42 owner/repo
if [ "$RC" = 0 ] && [ "$(count)" = 1 ] && ! grep -q '^issue close' "$D/calls.log" && [ "$(cat "$D/state")" = OPEN ]; then
  pass "CONTROL_a_pending_row_comments_and_stays_open"
else no "CONTROL_a_pending_row_comments_and_stays_open (rc=$RC, out=$OUT)"; fi

# C: running it twice with the same values posts nothing the second time.
fixture c done abc1234
run 7 --issue 42 owner/repo; FIRST="$RC"
run 7 --issue 42 owner/repo; SECOND="$RC"
if [ "$FIRST" = 0 ] && [ "$SECOND" = 0 ] && [ "$(count)" = 1 ] && printf '%s' "$OUT" | grep -q 'already reported'; then
  pass "CONTROL_a_second_run_with_the_same_values_posts_nothing"
else no "CONTROL_a_second_run_with_the_same_values_posts_nothing (rc=$FIRST/$SECOND, out=$OUT)"; fi

# D: a different commit posts again.
fixture d done abc1234
run 7 --issue 42 owner/repo
printf '{"7": {"status": "done", "commit": "def5678", "spec": "specs/7-a-thing.md"}}' > "$STATE"
run 7 --issue 42 owner/repo
if [ "$RC" = 0 ] && [ "$(count)" = 2 ]; then
  pass "CONTROL_a_different_commit_posts_again"
else no "CONTROL_a_different_commit_posts_again (rc=$RC, comments=$(count))"; fi

# E: the comment is accepted but the read-back omits it: non-zero, and no close.
fixture e done abc1234
OUT="$(PATH="$D/bin:$PATH" FAKE_IGNORE_COMMENT=1 PII_STATE="$STATE" bash "$SCRIPT" 7 --issue 42 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 5 ] && ! grep -q '^issue close' "$D/calls.log"; then
  pass "CONTROL_a_read_back_without_the_comment_fails"
else no "CONTROL_a_read_back_without_the_comment_fails (rc=$RC, out=$OUT)"; fi

# F: an unknown row is refused before anything is posted.
fixture f done abc1234
OUT="$(PATH="$D/bin:$PATH" PII_STATE="$STATE" bash "$SCRIPT" 999 --issue 42 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 3 ] && ! grep -qE '^(issue comment|issue close)' "$D/calls.log"; then
  pass "CONTROL_an_unknown_row_is_refused"
else no "CONTROL_an_unknown_row_is_refused (rc=$RC, out=$OUT)"; fi

# G: close refused by the server: the read-back disagrees and the script fails.
fixture g done abc1234
OUT="$(PATH="$D/bin:$PATH" FAKE_IGNORE_CLOSE=1 PII_STATE="$STATE" bash "$SCRIPT" 7 --issue 42 owner/repo 2>&1)"; RC=$?
if [ "$RC" = 6 ]; then pass "CONTROL_a_close_that_does_not_land_fails"
else no "CONTROL_a_close_that_does_not_land_fails (rc=$RC, out=$OUT)"; fi

# H (spec 421 R6): a PERSON who quotes pii's comment in a reply must not suppress the real report. The quoting comment
# carries the same sentence, but not on its first line.
fixture h done abc1234
printf 'Interesting — as pii said:\n\npii: row 7 done at abc1234 — specs/7-a-thing.md\n<!-- pii kind=row row=7 commit=abc1234 -->\n' > "$D/comments/1"
run 7 --issue 42 owner/repo
if [ "$RC" = 0 ] && [ "$(count)" = 2 ] && printf '%s' "$OUT" | grep -q '^commented:'; then
  pass "CONTROL_a_human_quoting_pii_does_not_suppress_the_report"
else no "CONTROL_a_human_quoting_pii_does_not_suppress_the_report (rc=$RC, comments=$(count), out=$OUT)"; fi

echo "PASS: $ok FAIL: $fail"
[ "$fail" = 0 ] && [ "$ok" -ge 8 ]
