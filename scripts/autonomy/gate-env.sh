#!/usr/bin/env bash
# gate-env.sh — the queue's gate selector (spec 262). Sourced by cycle.sh.
#
# A queue item's commit is gated with the FULL list (gate-steps.txt), whatever GATE_STEPS holds in the environment.
# env.sh exports the quick list for its own callers, and a queue that read GATE_STEPS would commit under three steps
# while the log said the gate passed. So this selector never reads GATE_STEPS.

# gate_pairs: the `key value` line of each key in $PII_ROOT/.pii/gate (spec 407); comments and blank lines are dropped. A repository
# with no file has pi-improved's own gate. A line that is not `key = value`, a key that is not one of the five, an empty value,
# or a non-whole-number min_free_mb/timeout_secs is refused: the message names the file and the line on stderr, and it returns 1.
gate_pairs(){
  local f="$PII_ROOT/.pii/gate" n=0 line k v
  [ -f "$f" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    n=$((n+1))
    [[ $line =~ ^[[:space:]]*(#.*)?$ ]] && continue
    [[ $line =~ ^[[:space:]]*([A-Za-z_]+)[[:space:]]*=(.*)$ ]] || { echo "pii: $f line $n: not 'key = value'" >&2; return 1; }
    k=${BASH_REMATCH[1]}; v=${BASH_REMATCH[2]}
    v=${v#"${v%%[![:space:]]*}"}; v=${v%"${v##*[![:space:]]}"}
    case "$k" in
      command|steps_file|build_check) ;;
      min_free_mb|timeout_secs) [[ $v =~ ^[0-9]+$ ]] || { echo "pii: $f line $n: $k must be a whole number, not '$v'" >&2; return 1; } ;;
      *) echo "pii: $f line $n: unknown key '$k'" >&2; return 1 ;;
    esac
    [ -n "$v" ] || { echo "pii: $f line $n: $k is empty" >&2; return 1; }
    echo "$k $v"
  done <"$f"
}

# gate_check: exit 0 when .pii/gate is understood (or absent); otherwise the refusal is on stderr and it exits 1.
gate_check(){ gate_pairs >/dev/null; }

# gate_value <key> <default>: the value .pii/gate declares for <key> (the last one wins), else the default. Exits 1 on a refused file,
# so a caller never runs an empty command.
gate_value(){
  local P k v found=$2
  P=$(gate_pairs) || return 1
  while read -r k v; do [ "$k" = "$1" ] && found=$v; done <<<"$P"
  printf '%s\n' "$found"
}

# gate_command: the gate a queue item's commit runs, run once by `bash -c` from the repository root (spec 407). Default: pi-improved's.
gate_command(){ gate_value command 'GATE_STEPS=gate-steps.txt QG_SKIP_COVERAGE=0 bash quality-gate.sh --routine'; }

# gate_build_check: the fast "does the tree build" command the build probes run in their scratch export (spec 407).
gate_build_check(){ gate_value build_check 'cargo check -q --workspace'; }

# gate_min_free_mb: the disk floor, in MB, that a gate run needs free (spec 407).
gate_min_free_mb(){ gate_value min_free_mb 12000; }

# gate_timeout: the seconds a gate run may take before it is killed (spec 407).
gate_timeout(){ gate_value timeout_secs 3600; }

# gate_steps_file: the list (relative to the repo root) that a queue item's commit is gated with.
gate_steps_file(){ gate_value steps_file gate-steps.txt; }

# gate_has_coverage: yes when the repository's gate has a coverage step, so a queue commit must measure coverage (spec 266, 407). With no
# .pii/gate this is pi-improved's gate, which has one. A declared gate is held to the coverage step its steps_file names, if any.
gate_has_coverage(){
  [ -f "$PII_ROOT/.pii/gate" ] || return 0
  local s; s=$(gate_steps_file) || return 1
  [ -f "$PII_ROOT/$s" ] && gate_step_labels "$PII_ROOT/$s" | grep -qx coverage-floor
}

# gate_step_labels <file>: the label of each step in a gate list, one per line. Blank and # lines have no label.
gate_step_labels(){
  sed -n 's/^\([^#|][^|]*\)|.*/\1/p' "$1"
}

# gate_skip_coverage: the value of QG_SKIP_COVERAGE on a queue item's gate command. Always 0 (spec 266): the coverage
# floor is measured on every queue commit, so an exported QG_SKIP_COVERAGE=1 (env.sh) cannot skip it here.
gate_skip_coverage(){
  echo 0
}

# coverage_measured <log>: yes when the gate log holds the coverage floor's roll-up line, no otherwise (spec 266).
coverage_measured(){
  if grep -qE '^coverage-floor: total [0-9.]+% >= floor' "$1"; then echo yes; else echo no; fi
}
