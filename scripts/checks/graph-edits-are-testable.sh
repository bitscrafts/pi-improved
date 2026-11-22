#!/usr/bin/env bash
# Spec 441 R3: a test that starts the lane must own the git HEAD it runs against.
#
# run-batchC-pii.sh refuses a start when graphs/spec.toml, graphs/docs.toml or graphs/preflight.toml differs from the git HEAD of the
# repository it runs in (spec 302), and it refuses an untracked graph too. A test that builds its own lane root and starts the lane in
# it therefore has to make that root a git repository (`git init`) with the graphs committed: otherwise the lane walks up to the
# checkout's .git and compares against a HEAD the test does not own, and a row that edits a graph — which cannot commit until its own
# gate is green — can never run the test. That is the circle spec 441 breaks.
#
# A file OFFENDS when it starts a lane for real and never initialises a repository of its own:
#   * a lane start is a command that runs `run-batchC-pii.sh` (the lane), a copy of it invoked as `run.sh`, or `drain.sh` **given a row
#     to dispatch** (`PII_FORCE_ITEM` / `--force-item`). A drain without a row only reads the guards, so it starts nothing.
#     A path used as data is not a start: an assignment (`S=$REPO/scripts/run-batchC-pii.sh`), a `cp`/`sed`/`printf` that writes a copy,
#     and a `grep`/`sed` pattern are all ignored. A start is the word after `bash`/`sh` (also through a variable the same file assigned
#     to one of those scripts, so `bash "$S" 294` counts when `S=$REPO/scripts/run-batchC-pii.sh`).
#   * real = no `--dry-run` on that command. A dry run is still compared with HEAD, but it never spends and never reaches a daemon; the
#     standing rule for a dry-run-only test is a clean checkout, and this check is about tests that start a run.
#   * owns its HEAD = the file runs `git init` or `git clone` (a clone has a HEAD of its own too, spec 302's own test uses one).
#
# It MUST fail when it inspects nothing: a check that scans no test and passes is the false green this repository names twice.
#
# Usage: bash scripts/checks/graph-edits-are-testable.sh              # scan scripts/autonomy/tests
#        bash scripts/checks/graph-edits-are-testable.sh --dir DIR    # scan DIR (the controls use this)
# Prints one line per offender (and per known-latent file), a RESULT line, and exits 0 only when there is no offender and at least
# one test was inspected. Exit 2 is a usage error or an unreadable directory.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)" || exit 2
dir="$root/scripts/autonomy/tests"
if [ "${1:-}" = "--dir" ]; then
  [ -n "${2:-}" ] || { echo "usage: graph-edits-are-testable.sh [--dir DIR]"; exit 2; }
  dir=$2
  shift 2
fi
[ -d "$dir" ] || { echo "FAIL: $dir is not a directory: nothing was inspected"; exit 1; }

# Pre-existing files with exactly this defect: they start the spec lane for real — in a scratch root of their own, or in the checkout
# itself — and inherit a HEAD they do not own, so they fail for the same reason lane-graph-select.test.sh did. Spec 441's scope is
# lane-graph-select.test.sh only, so they are NOT fixed here — and they are not silently exempt either: every run prints them as
# KNOWN-LATENT. Fixing one means deleting its entry, which this check will then demand.
known="lane-facts.test.sh lane-preflight.test.sh lane-resume.test.sh lane-wait-cancels.test.sh"

# aliases <file>: the variables the file assigns one of the lane scripts to (S="$SCRATCH/repo/run.sh", S=$REPO/scripts/run-batchC-pii.sh).
aliases() {
  sed -e ':a' -e '/\\$/{N;s/\\\n//;ba}' "$1" \
    | sed -nE 's/^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)=[^;|&]*(run-batchC-pii\.sh|run\.sh|drain\.sh)"?([[:space:]]|;|$).*/\1/p' \
    | sort -u
}

# starts <file>: prints one "<line>: <text>" per real lane start, and nothing when the file never starts a lane for real.
# Line continuations are joined first, so a command split over two lines is still one command; comment lines are not commands.
starts() {
  local f=$1 joined a
  joined=$(sed -e ':a' -e '/\\$/{N;s/\\\n//;ba}' "$f")
  # 1: an explicit interpreter, then a literal path to the lane script (this also covers `cd X && bash Y`, `env VAR=1 bash Y`)
  printf '%s\n' "$joined" | grep -nE '(^|[[:space:];&|(])(bash|sh)[[:space:]]+[^;&|]*(run-batchC-pii\.sh|run\.sh|drain\.sh)("|[[:space:]]|;|$)'
  # 2: the same interpreter, then a variable this file bound to one of them (`bash "$S" 294`)
  for a in $(aliases "$f"); do
    printf '%s\n' "$joined" | grep -nE '(^|[[:space:];&|(])(bash|sh)[[:space:]]+"?\$'"$a"'"?([[:space:]]|;|$)'
  done | sort -u -t: -k1,1n
}

ok=0; offenders=0; latent=0; inspected=0
for f in "$dir"/*.test.sh; do
  [ -e "$f" ] || continue
  inspected=$((inspected + 1))
  name=$(basename "$f")
  hits=$(starts "$f" | grep -v -- '--dry-run' | grep -vE '^[0-9]+:[[:space:]]*#')
  [ -n "$hits" ] || continue
  # a drain that is given a row is a lane start; a drain that only reads the guards in a test is not. Keep only the former.
  hits=$( { printf '%s\n' "$hits" | grep -vE 'drain\.sh' ; printf '%s\n' "$hits" | grep -E 'drain\.sh' | grep -E 'PII_FORCE_ITEM|--force-item' ; } | grep -E '.' | sort -u )
  [ -n "$hits" ] || continue
  first=$(printf '%s\n' "$hits" | head -1 | cut -c1-160)
  if grep -qE '\bgit (init|clone)\b' "$f"; then
    ok=$((ok + 1)); continue
  fi
  case " $known " in
    *" $name "*) latent=$((latent + 1)); echo "KNOWN-LATENT $name: starts the lane for real and inherits the checkout's HEAD ($first)"; continue ;;
  esac
  offenders=$((offenders + 1))
  echo "FAIL $name: starts the lane for real without a git repository of its own, so it compares against the checkout's HEAD ($first)"
done

if [ "$inspected" -eq 0 ]; then
  echo "FAIL: no *.test.sh in $dir — a check that inspects nothing must not pass"
  exit 1
fi
echo "RESULT: $inspected tests inspected, $ok of the lane starters own their HEAD, $latent known-latent, $offenders FAIL"
[ "$offenders" -eq 0 ]