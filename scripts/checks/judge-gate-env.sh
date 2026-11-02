#!/usr/bin/env bash
# The judge runs the spec's criteria INSIDE the devbox daemon, so the gate the judge runs must inherit the gate's
# environment -- or it judges the machine instead of the change.
#
# Measured 2026-11-02: rows 330 and 331 were refused with `run.refused{hook=judge, reason=the spec's criteria
# failed}` while the same criteria passed by hand in this shell. Cause, read from the code: the daemon is started
# with an EXPLICIT `env` list (modules/devbox/daemon.sh), and that list carried no FRESH_GATE_TARGET -- so each
# judgement built a throwaway ~28 GB compiler target inside a volume already 88% full, the volume slowed, and the
# gate's `session.prompt < 50 ms` assertion went red. The check was measuring the disk, and the disk was the
# check's own footprint. Same shape as the M0 gate-heartbeat bug (a check whose verdict depended on how it was
# invoked).
#
# This check drives the ARTEFACT: it extracts the `env …` command that actually starts the daemon and asks whether
# the gate's variables are in it. The control is a copy of that command with the variable removed, which must fail.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
D=modules/devbox/daemon.sh
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }

# the one line that starts the daemon: it is the only one carrying `podman exec -d` and an `env`
LINE=$(grep -n 'podman exec -d' "$D" | head -1)
if [ -z "$LINE" ]; then fail "the daemon start line was not found in $D"; echo "JUDGE-GATE-ENV: 0 ok, 1 FAIL"; exit 1; fi
pass "the daemon start line is at ${LINE%%:*}"

has(){ printf '%s' "$LINE" | grep -q " $1="; }

for v in FRESH_GATE_TARGET CARGO_BUILD_JOBS; do
  if has "$v"; then pass "$v is in the env the daemon starts with"; else fail "$v is MISSING from the daemon's env"; fi
done

# CONTROL: the same line with the variable stripped must be judged as missing. Without this, `has` could be
# matching something else in the line and the check would pass on a daemon that never receives it.
STRIPPED=$(printf '%s' "$LINE" | sed 's/ FRESH_GATE_TARGET=[^ ]*//')
if printf '%s' "$STRIPPED" | grep -q ' FRESH_GATE_TARGET='; then
  fail "control: the stripped line still carries FRESH_GATE_TARGET (the check cannot fail)"
else
  pass "control: a line without FRESH_GATE_TARGET fails this check"
fi

# and the value must be the repo's own warm cache, not an empty placeholder
if printf '%s' "$LINE" | grep -qE 'FRESH_GATE_TARGET=/projects/pi-improved/_tmp/gate-target'; then
  pass "the value is the repo's warm gate cache"
else
  fail "the value is not /projects/pi-improved/_tmp/gate-target"
fi

echo "JUDGE-GATE-ENV: $ok ok, $bad FAIL"
[ "$bad" -eq 0 ]
