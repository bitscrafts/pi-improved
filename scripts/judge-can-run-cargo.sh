#!/usr/bin/env bash
# judge-can-run-cargo.sh — ask the REAL daemon judge, one criterion per run, what its child can do. Spends nothing.
#
# `tdd_green` failed in 0.3 s on a criterion that passes in 27 s outside the judge, and `tdd_red` "failed" in 0.5 s.
# If the judge cannot run cargo, every cargo criterion is red for ever and nobody notices, because red is exactly what
# the RED step wants. A single multi-criterion run only reports the first failure, so each sentinel is its own run.
set -uo pipefail
cd /projects/pi-improved || exit 1
HOST=mvcorrea@10.10.10.217
C=/projects/pi-improved/_tmp/devbox-test
X() { ssh -o BatchMode=yes "$HOST" "podman exec -w /projects/pi-improved pii-devbox env PII_RUNTIME_DIR=$C/run $*" 2>&1; }
cp -f graphs/preflight.toml "$C/graphs/" 2>/dev/null

probe() { # <label> <criterion>
  local label=$1 crit=$2 TS D BASE IN RID ST
  TS=$(date +%s%N); D=_tmp/devbox-test/judge-probe-$TS
  mkdir -p "$D/repo/specs"
  printf '# Spec: probe\n\n## Exit Criteria\n- [ ] `%s`\n' "$crit" > "$D/repo/specs/probe.md"
  git -C "$D/repo" init -q . && git -C "$D/repo" add specs && git -C "$D/repo" -c user.name=probe -c user.email=p@l commit -qm base
  BASE=$(git -C "$D/repo" rev-parse HEAD)
  IN="{\"spec\":\"specs/probe.md\",\"base\":\"$BASE\",\"workdir\":\"$C/judge-probe-$TS/repo\"}"
  RID=$(X "$C/bin/pii run start preflight --input '$IN'" | grep -o 'r[0-9]\{12\}' | head -1)
  for _ in $(seq 1 90); do
    ST=$(X "$C/bin/pii run get $RID" | grep -o '"status":"[a-z_]*"' | head -1)
    case "$ST" in *succeeded*|*failed*) break ;; esac; sleep 2
  done
  printf '%-34s %s\n' "$label" "$(printf '%s' "$ST" | sed 's/"status"://')"
  mkdir -p /projects/_trash && mv "$D" "/projects/_trash/judge-probe-$TS"
}

probe "cargo is on the PATH"            'command -v cargo'
probe "cargo runs"                      'cargo --version'
probe "CARGO_TARGET_DIR is writable"      'touch "$CARGO_TARGET_DIR/.judge-probe" && rm -f "$CARGO_TARGET_DIR/.judge-probe"'
probe "HOME is set"                     'test -n "$HOME"'
probe "CARGO_TARGET_DIR is set"         'test -n "$CARGO_TARGET_DIR"'
probe "cargo metadata (offline)"        'cd /projects/pi-improved && cargo metadata --offline --format-version 1 --no-deps >/dev/null'
probe "the lane_plan_parity criterion"  "cd /projects/pi-improved && timeout 100 cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -qE 'test result: ok\\. [1-9][0-9]* passed'"
