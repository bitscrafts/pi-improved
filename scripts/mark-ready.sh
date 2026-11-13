#!/usr/bin/env bash
# Mark an issue `ready`. Usage: scripts/mark-ready.sh <issue-number> [--owner-approved] [owner/repo]
# `ready` means "specified well enough to start", so only a maintainer may set it. This script is how the
# maintainer's assistant does it without opening the door to strangers: an issue written by anyone other
# than the repository owner is refused unless the owner approved it explicitly (--owner-approved).
#
# Spec 417: measured 2026-11-12, a repository that never had a `triage` label could not be marked ready at all:
# `gh issue edit --remove-label triage --add-label ready` is one call, and GitHub rejects the whole call when
# any label it names is missing (`'triage' not found`). `ready` itself also had to exist. So: `ready` is created
# when the repository lacks it, a label is removed only when the issue carries it and in its own call, and the
# success line is printed only after the labels are read back and agree.
set -eu
N="${1:?usage: mark-ready.sh <issue-number> [--owner-approved] [owner/repo]}"; shift
APPROVED=0; [ "${1:-}" = "--owner-approved" ] && { APPROVED=1; shift; }
REPO="${1:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"
OWNER="${REPO_OWNER:-${REPO%%/*}}"
READY_DESC="Specified well enough to start; only the owner sets it"
AUTHOR=$(gh issue view "$N" --repo "$REPO" --json author -q .author.login)
STATE=$(gh issue view "$N" --repo "$REPO" --json state -q .state)
[ "$STATE" = "OPEN" ] || { echo "REFUSE: #$N is $STATE"; exit 3; }
if [ "$AUTHOR" != "$OWNER" ] && [ "$APPROVED" != 1 ]; then
  echo "REFUSE: #$N was written by $AUTHOR, not $OWNER. Read it, then re-run with --owner-approved if $OWNER says so."
  exit 4
fi
labels() { gh issue view "$N" --repo "$REPO" --json labels -q '[.labels[].name]|join(",")'; }
has() { case ",$1," in *",$2,"*) return 0 ;; esac; return 1; }

# R1: the repository must have `ready` before an issue can carry it. An existing one is never touched.
if ! gh label list --repo "$REPO" --json name -q '.[].name' | grep -qx ready; then
  gh label create ready --repo "$REPO" --color 0e8a16 --description "$READY_DESC" >/dev/null
fi

# R2: remove only what the issue actually carries, in its own call, so one missing label cannot sink the rest.
WAS_TRIAGE=0
NOW=$(labels)
if has "$NOW" triage; then
  WAS_TRIAGE=1
  gh issue edit "$N" --repo "$REPO" --remove-label triage >/dev/null
fi
gh issue edit "$N" --repo "$REPO" --add-label ready >/dev/null

# R3: the claim is what the read-back says, never what the calls returned.
AFTER=$(labels)
has "$AFTER" ready || { echo "FAIL: #$N does not carry ready after the edit (labels: ${AFTER:-none})"; exit 5; }
if [ "$WAS_TRIAGE" = 1 ] && has "$AFTER" triage; then
  echo "FAIL: #$N still carries triage after the edit (labels: $AFTER)"; exit 5
fi
echo "ready: #$N (author $AUTHOR$([ "$AUTHOR" != "$OWNER" ] && echo ', owner-approved'))"
