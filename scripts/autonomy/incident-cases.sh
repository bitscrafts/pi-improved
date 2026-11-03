#!/usr/bin/env bash
# incident-cases.sh — an incident becomes a case without a person (spec 350).
#
# A NEW fingerprint from `pii incidents scan` opens a CASE: one line in the ledger and its evidence beside it, nothing
# else. A case is not a row: cycle.sh's pick reads plan.tsv only, so a case cannot be dispatched. Writing the criteria
# is the work, and a person or the skill lane does it: `--brief` writes the spec stub with a `## TODO criteria` section.
#
# States: new -> briefed (a spec stub exists) -> queued (a row exists) -> closed (a treatment landed).
# The ledger is append-only TSV; the LAST line for a case_id is its state:
#   fingerprint · first_seen · count · source_host · state · case_id
# A fingerprint already open is never appended twice. A closed fingerprint that recurs is a NEW case <fp>#N, because a
# cure that did not hold is its own finding.
#
# Usage:
#   incident-cases.sh [--input FILE] [--max-new N] [--dry-run]   scan (FILE, else the host over ssh) and open cases
#   incident-cases.sh --list                                     cases whose latest state is new
#   incident-cases.sh --brief <case_id>                          write the spec stub; state -> briefed
#
# Silent when nothing is new: one line per new case, nothing otherwise, so a no_agent cron never becomes noise.
# The cap (--max-new, default 5, oldest first) reports the rest as `deferred` and opens them on a later run.
# Env: PII_CASES (ledger, default _tmp/autonomy/incident-cases.tsv), PII_CASE_EVIDENCE (default: beside the ledger),
#      PII_CASE_SPECS (default specs), PII_HOST (default mvcorrea@lnx), PII_SOURCE_HOST (default: PII_HOST).
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
LEDGER=${PII_CASES:-_tmp/autonomy/incident-cases.tsv}
EVD=${PII_CASE_EVIDENCE:-$(dirname "$LEDGER")/incident-case-evidence}
SPECS=${PII_CASE_SPECS:-specs}
HOST=${PII_HOST:-mvcorrea@lnx}
SRC_HOST=${PII_SOURCE_HOST:-$HOST}

usage(){ echo "usage: incident-cases.sh [--input FILE] [--max-new N] [--dry-run] | --list | --brief <case_id>" >&2; exit 2; }
MODE=scan; INPUT=""; MAX_NEW=5; DRY=0; BRIEF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --input)   [ -n "${2:-}" ] || usage; INPUT=$2; shift 2 ;;
    --max-new) [ -n "${2:-}" ] || usage; MAX_NEW=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --list)    MODE=list; shift ;;
    --brief)   [ -n "${2:-}" ] || usage; MODE=brief; BRIEF=$2; shift 2 ;;
    *) usage ;;
  esac
done
case "$MAX_NEW" in ''|*[!0-9]*) echo "incident-cases: --max-new must be a number" >&2; exit 2 ;; esac

# newest_of <fp> -> "<state> <cases>" for the newest case of this fingerprint; empty when it has none
newest_of(){
  awk -F'\t' -v f="$1" '
    NF>=6 && !($6 in st) { order[++n]=$6; fpof[$6]=$1 }
    NF>=6 { st[$6]=$5 }
    END { for (i=1; i<=n; i++) if (fpof[order[i]]==f) { last=st[order[i]]; c++ }; if (c) print last, c }' "$LEDGER" 2>/dev/null
}

# evidence file for a case (the scan line that opened it)
evidence_of(){ printf '%s' "$EVD/${1//\//_}.txt"; }

scan_output(){
  if [ -n "$INPUT" ]; then cat "$INPUT"
  else timeout 90 ssh -o BatchMode=yes "$HOST" "PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii incidents scan --source runtime --log ~/.local/state/pii/pii.log"
  fi
}

case "$MODE" in
list)
  [ -f "$LEDGER" ] || exit 0
  awk -F'\t' '
    NF>=6 && !($6 in st) { order[++n]=$6 }
    NF>=6 { st[$6]=$5; fp[$6]=$1; fs[$6]=$2; c[$6]=$3 }
    END { for (i=1; i<=n; i++) { k=order[i]; if (st[k]=="new") printf "%s\t%s\tfirst_seen=%s\tcount=%s\n", k, fp[k], fs[k], c[k] } }' "$LEDGER"
  exit 0 ;;

brief)
  line=$(awk -F'\t' -v c="$BRIEF" 'NF>=6 && $6==c {print; exit}' "$LEDGER" 2>/dev/null)
  [ -n "$line" ] || { echo "incident-cases: no case $BRIEF in $LEDGER" >&2; exit 2; }
  state=$(awk -F'\t' -v c="$BRIEF" 'NF>=6 && $6==c {s=$5} END{print s}' "$LEDGER")
  [ "$state" = new ] || { echo "incident-cases: $BRIEF is $state, not new — nothing to brief" >&2; exit 2; }
  spec="$SPECS/auto-$BRIEF.md"
  [ ! -e "$spec" ] || { echo "incident-cases: $spec exists — refusing to overwrite it" >&2; exit 2; }
  IFS=$'\t' read -r fp first count src _ <<<"$line"
  mkdir -p "$SPECS"
  {
    echo "# Auto-case $BRIEF — TODO: a person or the skill lane writes the criteria"
    echo
    echo "Opened by incident-cases.sh (spec 350). This stub carries the evidence and is NOT dispatchable until its"
    echo "criteria are written: a criterion that can fail cannot be invented from a fingerprint."
    echo
    echo "## Evidence"
    echo
    echo "- fingerprint: $fp"
    echo "- first_seen: $first"
    echo "- count: $count"
    echo "- source_host: $src"
    echo
    echo '```'
    cat "$(evidence_of "$BRIEF")" 2>/dev/null || echo "(no evidence file was kept for this case)"
    echo '```'
    echo
    echo "## TODO criteria"
    echo
    echo "- [ ] TODO: a criterion that can fail"
  } >"$spec"
  printf '%s\t%s\t%s\t%s\tbriefed\t%s\n' "$fp" "$first" "$count" "$src" "$BRIEF" >>"$LEDGER"
  echo "briefed $BRIEF -> $spec"
  exit 0 ;;
esac

# scan: open a case for every NEW fingerprint, oldest first, at most MAX_NEW per run
INC=$(scan_output) || { echo "incident-cases: the scan failed — no cases opened" >&2; exit 2; }
SORTED=$(printf '%s\n' "$INC" | grep '^incident ' \
  | awk '{ fs=""; for (i=1; i<=NF; i++) if ($i ~ /^first_seen=/) fs=substr($i, 12); print fs "\t" $0 }' \
  | sort -s -k1,1 | cut -f2-)

N_NEW=0
while IFS= read -r line; do
  [ -n "$line" ] || continue
  fp=$(awk '{print $2}' <<<"$line")
  first=$(sed -n 's/.* first_seen=\([^ ]*\).*/\1/p' <<<"$line")
  count=$(sed -n 's/.* count=\([^ ]*\).*/\1/p' <<<"$line")

  read -r state ncases <<<"$(newest_of "$fp")"
  if [ -z "${state:-}" ]; then cid=$fp
  elif [ "$state" = closed ]; then cid="$fp#$((ncases + 1))"
  else continue   # already open: a present fingerprint is never appended twice
  fi

  if [ "$N_NEW" -ge "$MAX_NEW" ]; then echo "deferred $fp first_seen=$first count=$count (over --max-new $MAX_NEW)"; continue; fi
  N_NEW=$((N_NEW + 1))

  if [ "$DRY" -eq 1 ]; then echo "would-open $cid first_seen=$first count=$count"; continue; fi
  mkdir -p "$(dirname "$LEDGER")" "$EVD"
  printf '%s\t%s\t%s\t%s\tnew\t%s\n' "$fp" "$first" "$count" "$SRC_HOST" "$cid" >>"$LEDGER"
  printf '%s\n' "$line" >"$(evidence_of "$cid")"
  echo "new $cid first_seen=$first count=$count"
done <<<"$SORTED"
exit 0
