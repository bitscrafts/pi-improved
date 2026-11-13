#!/usr/bin/env bash
# Spec 421: the issue template, the local spec template and pii's own comments are one vocabulary, so an issue can be
# drafted into a spec without translation, and a script can tell a comment pii wrote from a person's without a model.
#
# Usage: bash scripts/checks/comment-and-template-vocabulary.sh            # the tree
#        bash scripts/checks/comment-and-template-vocabulary.sh --control # prove the validators can fail
#
# A comment is pii's only when its FIRST line matches the prefix AND its SECOND line is the marker. Testing the first
# line, not "contains", is what stops a human reply that quotes pii from being counted as pii. GitHub reports a bot's
# comment as author.type == "Bot"; a comment posted through a user token is "User" whether an LLM or a person wrote it,
# so this marker is the only mechanical answer available until a separate bot identity exists (spec 425).
#
# Prints one PASS/FAIL line per assertion and a final `PASS: n FAIL: m`; exits 0 only when every assertion passed.
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" || exit 2
cd "$root" || exit 2

PREFIX='^pii(@[a-z0-9][a-z0-9.-]*)?: (received|drafted|refused|row)( |$)'
MARKER='^<!-- pii kind=(received|drafted|refused|row)( [a-z]+=[^ ]+)* -->$'
KINDS='received drafted refused row'

ok=0; fail=0
pass() { ok=$((ok + 1)); echo "PASS: $1"; }
no() { fail=$((fail + 1)); echo "FAIL: $1"; }

# whose <body>: pii when the first line is the prefix AND the second is the marker; human otherwise.
whose() {
  local first second
  first="$(printf '%s\n' "$1" | sed -n 1p)"
  second="$(printf '%s\n' "$1" | sed -n 2p)"
  printf '%s\n' "$first" | grep -qE "$PREFIX" || { echo human; return; }
  printf '%s\n' "$second" | grep -qE "$MARKER" || { echo human; return; }
  echo pii
}

# kinds_of <body>: the kind word from line 1, when the body is pii's.
kinds_of() { printf '%s\n' "$1" | sed -n '1s/^pii\(@[a-z0-9.\-]*\)\{0,1\}: \([a-z]*\).*/\2/p'; }

# --- the issue template: each field says which spec section it becomes -------------------------------
TASK=.github/ISSUE_TEMPLATE/task.yml
if [ -f "$TASK" ]; then
  field_says() { # field_says <id> <section>
    python3 - "$TASK" "$1" "$2" <<'PY'
import sys
path, field, section = sys.argv[1], sys.argv[2], sys.argv[3]
lines = open(path, encoding="utf-8").read().splitlines()
cur, found, hits = None, [], 0
for line in lines:
    s = line.strip()
    if s.startswith("- id:") or s.startswith("id:"):
        cur = s.split(":", 1)[1].strip()
    elif cur == field and s.startswith("description:"):
        hits += 1
        if section in s:
            found.append(True)
sys.exit(0 if hits and found else 1)
PY
  }
  field_says change "## Overview" && pass "the issue template maps What changes to the spec's Overview" \
    || no "the issue template's What changes does not name ## Overview"
  field_says criteria "## Exit Criteria" && pass "the issue template maps Exit criteria to ## Exit Criteria" \
    || no "the issue template's Exit criteria does not name ## Exit Criteria"
  field_says files "## Files" && pass "the issue template maps Files it may touch to ## Files" \
    || no "the issue template's Files it may touch does not name ## Files"
  grep -qE '^[[:space:]]*-?[[:space:]]*id: context$' "$TASK" && pass "the issue template has the optional Context field" \
    || no "the issue template has no Context field"
else
  no "$TASK is missing"
fi

# --- the local spec template -----------------------------------------------------------------------
T=specs/TEMPLATE.md
if [ -f "$T" ]; then
  sed -n 2p "$T" | grep -qE '^#: depends=[^ ]+ status=[^ ]+ block=[^ ]+$' \
    && pass "the spec template's card line matches the shape the queue reads" \
    || no "the spec template's card line is not the shape the queue reads"
  missing=""
  for s in Overview Requirements Architecture "TDD Contract" "Exit Criteria" Guardrails Files; do
    grep -qxF "## $s" "$T" || missing="$missing $s"
  done
  [ -z "$missing" ] && pass "the spec template carries all seven sections" || no "the spec template lacks:$missing"
  printf '%s' "$(basename "$T")" | grep -qE '^[0-9]+-' \
    && no "specs/TEMPLATE.md is numbered: the dispatcher would resolve it as a row's spec" \
    || pass "the spec template is not numbered, so the dispatcher never resolves it"
else
  no "$T is missing"
fi

# --- pii's own comments in the tree: both lines, per kind --------------------------------------------
G=graphs/github-issue.toml
for k in received drafted refused; do
  if grep -qE "pii: $k.*kind=$k" "$G"; then
    pass "the graph's $k comment carries the prefix and the marker on one line"
  else
    no "the graph's $k comment lacks the prefix or the marker (or they are not on one line)"
  fi
done
# report-row.sh builds the two lines in shell, so the structure is what can be read from the file; that the two lines
# actually reach the issue is proven behaviourally by scripts/checks/report-row-comments-and-closes.sh (case A reads
# line 1 and line 2 back from the posted comment).
if grep -qE 'pii: row .*\$ROW' scripts/report-row.sh && grep -qE 'kind=row' scripts/report-row.sh \
   && grep -qE '^BODY=' scripts/report-row.sh; then
  pass "report-row.sh builds both lines of its comment and joins them into one body"
else
  no "report-row.sh does not build both lines of its comment"
fi

# --- the control: the validators must be able to fail ------------------------------------------------
if [ "${1:-}" = "--control" ]; then
  good=$'pii: row 1 done at abc1234 — specs/1-x.md\n<!-- pii kind=row row=1 commit=abc1234 -->'
  quoted=$'A person writes first.\n\npii: row 1 done at abc1234 — specs/1-x.md\n<!-- pii kind=row row=1 commit=abc1234 -->'
  prefix_only=$'pii: row 1 done at abc1234 — specs/1-x.md'
  plain=$'Received. Nothing runs until the owner has read the spec drafted from this issue.'
  [ "$(whose "$good")" = pii ] && pass "control: a two-line pii comment is classified as pii" \
    || no "control: a two-line pii comment was not classified as pii"
  [ "$(whose "$quoted")" = human ] && pass "control: a human reply that quotes pii further down is a human's" \
    || no "control: a quoting reply was counted as pii"
  [ "$(whose "$prefix_only")" = human ] && pass "control: a prefix with no marker is a human's" \
    || no "control: a prefix with no marker was counted as pii"
  [ "$(whose "$plain")" = human ] && pass "control: a comment without the prefix is a human's" \
    || no "control: an unprefixed comment was counted as pii"
  [ "$(kinds_of "$good")" = row ] && pass "control: the kind is read from the first line" \
    || no "control: the kind was not read from the first line"
  # the control must also be able to fail: a template without a card line must be rejected.
  mkdir -p _tmp/control-421 && printf '# Spec: x\n\n## Files\n- `a`\n' > _tmp/control-421/TEMPLATE.md
  sed -n 2p _tmp/control-421/TEMPLATE.md | grep -qE '^#: depends=[^ ]+ status=[^ ]+ block=[^ ]+$' \
    && no "control: a template without a card line was accepted" \
    || pass "control: a template without a card line is rejected"
  rm -f _tmp/control-421/TEMPLATE.md; rmdir _tmp/control-421 2>/dev/null
fi

echo "PASS: $ok FAIL: $fail"
[ "$fail" = 0 ] && [ "$ok" -ge 8 ]
