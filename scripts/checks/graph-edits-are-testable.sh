#!/usr/bin/env bash
# Spec 441 R3 / spec 442 R2: a test that starts the lane must own the git HEAD it runs against — shell OR Rust.
#
# run-batchC-pii.sh refuses a start when graphs/spec.toml, graphs/docs.toml or graphs/preflight.toml differs from the git HEAD of the
# repository it runs in (spec 302), and it refuses an untracked graph too. A test that builds its own lane root and starts the lane in
# it therefore has to make that root a git repository (`git init`) with the graphs committed: otherwise the lane walks up to the
# checkout's .git and compares against a HEAD the test does not own, and a row that edits a graph — which cannot commit until its own
# gate is green — can never run the test. That is the circle spec 441 breaks for scripts/autonomy/tests, and spec 442 breaks for
# crates/*/tests: lane_run_parity.rs drove the shell against this checkout itself.
#
# It scans two directories, each with its own pattern:
#   * scripts/autonomy/tests/*.test.sh   (--dir DIR)
#   * crates/*/tests/*.rs                (--rust-dir DIR; DIR is crates/-shaped: DIR/*/tests/*.rs)
#
# A file OFFENDS when it starts a lane for real and never initialises a repository of its own.
#   * a lane start, shell test: a command that runs `run-batchC-pii.sh` (the lane), a copy of it invoked as `run.sh`, or `drain.sh`
#     **given a row to dispatch** (`PII_FORCE_ITEM` / `--force-item`). A drain without a row only reads the guards, so it starts
#     nothing. A path used as data is not a start: an assignment (`S=$REPO/scripts/run-batchC-pii.sh`), a `cp`/`sed`/`printf` that
#     writes a copy, and a `grep`/`sed` pattern are all ignored. A start is the word after `bash`/`sh` (also through a variable the
#     same file assigned to one of those scripts, so `bash "$S" 294` counts when `S=$REPO/scripts/run-batchC-pii.sh`).
#   * a lane start, Rust test: a non-comment line naming `run-batchC-pii.sh` or `drain.sh` in a file that also spawns an interpreter
#     (`Command::new("bash")` / `Command::new("sh")`). Rust spreads one call over several lines, so the interpreter and the script
#     name are matched in the file, not on the line. A bare `run.sh` is NOT matched here: `"tools/run.sh"` is a fixture path in a test
#     that starts nothing, and the Rust fixtures that drive the lane keep the script's own name.
#   * real = no `--dry-run` on that command (shell tests only: the flag is matched on the line, and a Rust call site cannot be read
#     that way). A dry run is still compared with HEAD, but it never spends and never reaches a daemon; the standing rule for a
#     dry-run-only test is a clean checkout, and this check is about tests that START a run. A Rust test that only dry-runs but owns
#     its HEAD is counted as ok either way; one that does not own it is still worth failing.
#   * owns its HEAD = the file runs `git init` or `git clone` (a clone has a HEAD of its own too, spec 302's own test uses one). In a
#     Rust test the spawn form counts as well: a `git` invocation together with the `"init"` argument — the fixtures build their
#     repository with `git(&["init", ...])` or `Command::new("git").args(["init", ...])`, which is `git init` even though the two
#     words never share a line.
#   * ANOTHER repository in the file does not cure a shell started on the checkout itself: a Rust start whose `current_dir()` is a
#     root the file took from the checkout (`let live = live_root();` … `current_dir(&live)`) is an offence on its own. That is
#     exactly the pre-fix `lane_run_parity`, which owned a scratch repo for one lane and drove this checkout for the differential.
#
# It MUST fail when it inspects nothing, in EACH directory it scans: a check that scans no test and passes is the false green this
# repository names twice.
#
# Usage: bash scripts/checks/graph-edits-are-testable.sh                  # scan both directories of this checkout
#        bash scripts/checks/graph-edits-are-testable.sh --dir DIR        # scan DIR/*.test.sh instead of scripts/autonomy/tests
#        bash scripts/checks/graph-edits-are-testable.sh --rust-dir DIR   # scan DIR/*/tests/*.rs instead of crates/
# Prints one line per offender (and per known-latent file), a RESULT line, and exits 0 only when there is no offender, each scanned
# directory held at least one test, and the RESULT ran. Exit 2 is a usage error.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)" || exit 2
shell_dir="$root/scripts/autonomy/tests"
rust_dir="$root/crates"
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) [ -n "${2:-}" ] || { echo "usage: graph-edits-are-testable.sh [--dir DIR] [--rust-dir DIR]"; exit 2; }
           shell_dir=$2; shift 2 ;;
    --rust-dir) [ -n "${2:-}" ] || { echo "usage: graph-edits-are-testable.sh [--dir DIR] [--rust-dir DIR]"; exit 2; }
           rust_dir=$2; shift 2 ;;
    *) echo "usage: graph-edits-are-testable.sh [--dir DIR] [--rust-dir DIR]"; exit 2 ;;
  esac
done

# Pre-existing files with exactly this defect. Spec 441 recorded four here — lane-facts, lane-preflight, lane-resume and
# lane-wait-cancels — and spec 442 fixed all four: each now builds a scratch repository and commits the graphs before it starts the
# lane, so the list is EMPTY and the RESULT prints 0. An entry belongs here only for a file that inherits a HEAD it does not own and
# cannot be given one, with the one-line reason on its line. A file that gains its own HEAD must lose its entry, which this check
# then demands: a suppression list that never shrinks is the blindness spec 441 removed.
known=""

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

# rust_starts <file>: prints one "<line>: <text>" per non-comment line naming a lane script.
rust_starts() {
  grep -nE 'run-batchC-pii\.sh|drain\.sh' "$1" | grep -vE '^[0-9]+:[[:space:]]*//'
}

# has_interpreter <file>: the Rust file spawns a shell, so a lane script it names is driven and not only read.
has_interpreter() {
  grep -qE 'Command::new\("(bash|sh)"\)' "$1"
}

# rust_checkout_vars <file>: the variables the file binds to this checkout's own root (`let live = live_root();`).
rust_checkout_vars() {
  grep -oE '\blet[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[[:space:]]*&?[[:space:]]*[A-Za-z_]*root\(\)' "$1" \
    | sed -E 's/.*let[[:space:]]+([A-Za-z0-9_]+).*/\1/' | sort -u
}

# drives_this_checkout <file>: the shell is started IN this checkout — a current_dir() on a root the file took from the checkout
# (`let live = live_root();` … `current_dir(&live)`). This is the offence a repository elsewhere in the file does not cure: the
# lane compares against the HEAD of the directory it runs in, and that directory is the checkout.
drives_this_checkout() {
  local f=$1 v
  for v in $(rust_checkout_vars "$f"); do
    grep -qE "current_dir\([^)]*\b$v\b" "$f" && return 0
  done
  grep -qE 'current_dir\([^)]*(live_root|agent_root|CARGO_MANIFEST_DIR)' "$f"
}

# owns_head <file> <shell|rust>
owns_head() {
  if [ "$2" = rust ]; then
    grep -qE 'git[[:space:]]+(init|clone)' "$1" && return 0
    # the spawn form: the fixtures build their repository with git(&["init", ...]) or Command::new("git").args(["init", ...])
    grep -qE 'Command::new\("git"\)|\.git\(&\[|fn git\(|"git"' "$1" && grep -qE '"init"' "$1" && return 0
    return 1
  fi
  grep -qE '\bgit (init|clone)\b' "$1"
}

ok=0; offenders=0; latent=0; shell_seen=0; rust_seen=0

# scan_one <shell|rust> <file>: count and report that file.
scan_one() {
  local kind=$1 f=$2 name hits first
  name=$(basename "$f")
  if [ "$kind" = shell ]; then
    hits=$(starts "$f" | grep -v -- '--dry-run' | grep -vE '^[0-9]+:[[:space:]]*#')
    [ -n "$hits" ] || return 0
    # a drain that is given a row is a lane start; a drain that only reads the guards in a test is not. Keep only the former.
    hits=$( { printf '%s\n' "$hits" | grep -vE 'drain\.sh' ; printf '%s\n' "$hits" | grep -E 'drain\.sh' | grep -E 'PII_FORCE_ITEM|--force-item' ; } | grep -E '.' | sort -u )
  else
    hits=$(rust_starts "$f")
    [ -n "$hits" ] || return 0
    has_interpreter "$f" || return 0
  fi
  [ -n "$hits" ] || return 0
  first=$(printf '%s\n' "$hits" | head -1 | cut -c1-160)
  # the specific circle this row closes: the shell is started IN the checkout, so the HEAD it compares against is the checkout's
  # whatever other repository the test built. A scratch copy of the script (lane_plan_parity, and this row's own fix) is not this.
  if [ "$kind" = rust ] && drives_this_checkout "$f"; then
    offenders=$((offenders + 1))
    echo "FAIL $name: starts the lane in this checkout itself, so it compares against the checkout's HEAD ($first)"
    return 0
  fi
  if owns_head "$f" "$kind"; then
    ok=$((ok + 1)); return 0
  fi
  case " $known " in
    *" $name "*) latent=$((latent + 1)); echo "KNOWN-LATENT $name: starts the lane for real and inherits the checkout's HEAD ($first)"; return 0 ;;
  esac
  offenders=$((offenders + 1))
  echo "FAIL $name: starts the lane for real without a git repository of its own, so it compares against the checkout's HEAD ($first)"
}

for f in "$shell_dir"/*.test.sh; do
  [ -e "$f" ] || continue
  shell_seen=$((shell_seen + 1))
  scan_one shell "$f"
done

for f in "$rust_dir"/*/tests/*.rs; do
  [ -e "$f" ] || continue
  rust_seen=$((rust_seen + 1))
  scan_one rust "$f"
done

rc=0
if [ "$shell_seen" -eq 0 ]; then
  echo "FAIL: no *.test.sh in $shell_dir — a check that inspects nothing must not pass"
  rc=1
fi
if [ "$rust_seen" -eq 0 ]; then
  echo "FAIL: no */tests/*.rs under $rust_dir — a check that inspects nothing must not pass"
  rc=1
fi
echo "RESULT: $shell_seen shell tests and $rust_seen Rust tests inspected, $ok of the lane starters own their HEAD, $latent known-latent, $offenders FAIL"
[ "$rc" -eq 0 ] && [ "$offenders" -eq 0 ]