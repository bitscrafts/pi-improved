#!/usr/bin/env bash
# Spec 286: the composition mutation sweep. Each row of composition_mutations.py is applied, one at a time, to a
# scratch copy of the tree; the row's test file is run there; the scratch file is restored from the live file and its
# sha256 is checked against the live file's. The live tree is never written.
# Usage: composition.sh [--root DIR] [--rows FILE] [--scratch DIR]   (paths relative to the repo root)
# Prints one row per mutation and `MUTATION-SWEEP: required <red>/<total> probes <n>`; exits 0 only when every
# required row is RED and every digest matched. A row whose test is red before any mutation is BASELINE-RED and does
# not count.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
abs() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$here" "$1" ;; esac; }
root="$here"
rows="$here/scripts/mutate/composition_mutations.py"
scratch="$here/_tmp/composition-sweep"
while [ $# -gt 0 ]; do
	case "$1" in
	--root) root="$(abs "$2")"; shift 2 ;;
	--rows) rows="$(abs "$2")"; shift 2 ;;
	--scratch) scratch="$(abs "$2")"; shift 2 ;;
	*) echo "usage: composition.sh [--root DIR] [--rows FILE] [--scratch DIR]" >&2; exit 2 ;;
	esac
done
work="$scratch.work"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"

uvhome="$here/_tmp/uv"
# The linker writes its temporary files under TMPDIR: default it to a gitignored dir when the caller set none.
export TMPDIR="${TMPDIR:-$here/_tmp/tmp}"
mkdir -p "$TMPDIR"
py() {
	UV_CACHE_DIR="${UV_CACHE_DIR:-$uvhome/cache}" UV_PYTHON_INSTALL_DIR="${UV_PYTHON_INSTALL_DIR:-$uvhome/python}" \
		uv run --no-project python "$here/scripts/mutate/composition_mutations.py" "$@"
}

# A leftover scratch tree from an interrupted run is moved aside, never deleted.
if [ -e "$scratch" ] || [ -e "$work" ]; then
	trash="$here/_tmp/_trash/composition-sweep-$(date -u +%Y%m%dT%H%M%SZ)-$$"
	mkdir -p "$trash"
	[ -e "$scratch" ] && mv "$scratch" "$trash/"
	[ -e "$work" ] && mv "$work" "$trash/"
	echo "moved a leftover scratch tree to ${trash#"$here"/}"
fi
mkdir -p "$scratch" "$work"
# `-m` stamps the copies with the time of the copy: cargo must not trust an artifact built from an older mtime.
git -C "$root" ls-files -z -co --exclude-standard | tar -C "$root" --null -T - -cf - | tar -C "$scratch" -xmf -

table="$(py rows --rows "$rows")"

# One cargo test run of a row's test file (`<crate>/<stem>`) in the scratch tree; the root crate's own tests when empty.
run_test() {
	local test="$1" log="$2"
	if [ -z "$test" ]; then
		cargo test -q --manifest-path "$scratch/Cargo.toml" >"$log" 2>&1
	else
		cargo test -q --manifest-path "$scratch/Cargo.toml" -p "${test%%/*}" --test "${test#*/}" >"$log" 2>&1
	fi
}

# The baseline: each distinct test file must pass before any mutation of it can count as red.
declare -A base=()
baseline() {
	local key="[$1]" slug="${1:-crate}"
	if [ -z "${base[$key]:-}" ]; then
		if run_test "$1" "$work/baseline-${slug//\//_}.log"; then base[$key]=ok; else base[$key]=red; fi
	fi
	base_state="${base[$key]}"
}

restore() {
	# The scratch file goes back to the live file's bytes, and its digest must match the live file's.
	cp "$root/$1" "$scratch/$1"
	local got
	got="$(sha256sum "$scratch/$1" | cut -d' ' -f1)"
	if [ "$got" != "$2" ]; then
		echo "MUTATION-SWEEP: ABORTED: $1 restored to sha256 $got, the live file is $2"
		exit 2
	fi
}

echo "| id | kind | file | test | result |"
echo "|---|---|---|---|---|"
red=0
total=0
probes=0
while IFS=$'\t' read -r id kind file test; do
	orig="$(sha256sum "$root/$file" | cut -d' ' -f1)"
	baseline "$test"
	if [ "$base_state" = red ]; then
		result=BASELINE-RED
	else
		set +e
		py apply --rows "$rows" --scratch "$scratch" --id "$id" >"$work/apply-$id.txt" 2>&1
		arc=$?
		set -e
		if [ "$arc" -eq 3 ]; then
			result=NO-APPLY
		elif [ "$arc" -ne 0 ]; then
			restore "$file" "$orig"
			cat "$work/apply-$id.txt"
			echo "MUTATION-SWEEP: ABORTED: applying $id failed"
			exit 2
		else
			set +e
			run_test "$test" "$work/$id.log"
			rc=$?
			set -e
			if [ "$rc" -eq 0 ]; then
				result=SURVIVED
			elif grep -q "could not compile" "$work/$id.log"; then
				result=BROKEN
			else
				result=RED
			fi
		fi
	fi
	restore "$file" "$orig"
	printf '| %s | %s | %s | %s | %s |\n' "$id" "$kind" "$file" "${test:--}" "$result"
	if [ "$kind" = required ]; then
		total=$((total + 1))
		if [ "$result" = RED ]; then red=$((red + 1)); fi
	else
		probes=$((probes + 1))
	fi
done <<<"$table"

echo "MUTATION-SWEEP: required $red/$total probes $probes"
[ "$total" -gt 0 ] && [ "$red" -eq "$total" ]
