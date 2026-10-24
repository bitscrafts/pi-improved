#!/usr/bin/env bash
# brief-path.sh — where this hour's queue brief goes (spec 251). The ONE place that decides the path, so cycle.sh and
# drain.sh cannot drift apart. The brief is a run's artefact: it lives under _tmp/ (git-ignored scratch), never in the
# tracked _docs/ tree, where an untracked file would make every dispatch read the tree as dirty.
#     BRIEF=$(bash scripts/autonomy/brief-path.sh)     # prints a repo-relative path; creates its directory
set -uo pipefail
cd /projects/pi-improved || exit 1
D=_tmp/autonomy/briefs
mkdir -p "$D" || exit 1
echo "$D/QUEUE-BRIEF-$(date -u +%Y-%m-%dT%H).md"
