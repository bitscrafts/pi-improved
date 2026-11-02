#!/usr/bin/env bash
# Spec 331 R4: a test file that spawns a live `pii daemon` must own the child through `DaemonGuard` and must not kill it by hand.
# A panic between the spawn and a hand-written `.kill()` leaves the daemon running, so the gate inherits it.
# Usage: bash scripts/checks/daemon-tests-are-guarded.sh            # scan crates/pii-cli/tests/*.rs
#        bash scripts/checks/daemon-tests-are-guarded.sh --control  # prove the checker can fail, in a scratch tree under _tmp/
# Prints `RESULT: <n> ok, <m> FAIL` and exits 0 only when m = 0, or one `FAIL <file>: <why>` line per offending file and exit 1.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 2

# A file spawns a daemon when it runs the built binary, calls `.spawn()` and passes "daemon" as an argument.
spawns_daemon() {
  grep -q 'CARGO_BIN_EXE_pii' "$1" && grep -q '\.spawn()' "$1" && grep -q '"daemon"' "$1"
}

# Scans the top-level *.rs files of one directory (the test targets; daemon_guard/mod.rs is not one). Prints FAIL lines,
# sets the global ok/fail counters.
check_dir() {
  local dir="$1" f
  ok=0
  fail=0
  for f in "$dir"/*.rs; do
    [ -e "$f" ] || continue
    spawns_daemon "$f" || continue
    local why=""
    grep -q 'DaemonGuard' "$f" || why="no DaemonGuard owns the child"
    if grep -q '\.kill()' "$f"; then
      why="${why:+$why; }calls .kill() by hand"
    fi
    if [ -n "$why" ]; then
      echo "FAIL $f: $why"
      fail=$((fail + 1))
    else
      ok=$((ok + 1))
    fi
  done
}

if [ "${1:-}" = "--control" ]; then
  scratch="$(mktemp -d "$root/_tmp/daemon-guard-control.XXXXXX")" || {
    echo "CONTROL: FAIL no scratch directory"
    exit 1
  }
  # The two copies are fixed fixtures, not the live file: the live daemon_log_wiring.rs is guarded now, so a copy of it
  # would no longer be the unguarded shape the control needs. Both spawn `pii daemon` as daemon_log_wiring.rs does.
  mkdir -p "$scratch/unguarded" "$scratch/guarded/daemon_guard"
  cat > "$scratch/unguarded/daemon_log_wiring.rs" <<'EOF'
use std::process::Command;

#[test]
fn a_worker_state_change_reaches_the_daemon_log_file() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("daemon")
        .args(["--sandbox", "off", "--workspace", "_tmp"])
        .spawn()
        .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
}
EOF
  cat > "$scratch/guarded/daemon_log_wiring.rs" <<'EOF'
// DaemonGuard owns the child: the guard reaps it on every exit path.
use std::process::Command;

#[test]
fn a_worker_state_change_reaches_the_daemon_log_file() {
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace", "_tmp"])
            .spawn()
            .unwrap(),
    );
    child.kill_now();
}
EOF
  # The guard module may call `.kill()`: it is not a test target, so the checker must not scan it.
  printf 'pub(crate) fn kill_it(c: &mut std::process::Child) { let _ = c.kill(); }\n' \
    > "$scratch/guarded/daemon_guard/mod.rs"

  check_dir "$scratch/unguarded"
  unguarded_fails=$fail
  check_dir "$scratch/guarded"
  guarded_fails=$fail
  if [ "$unguarded_fails" -eq 1 ] && [ "$guarded_fails" -eq 0 ]; then
    echo "REFUSED: the unguarded copy is refused (see the FAIL line above)"
    echo "CONTROL: the unguarded copy is refused and the guarded copy passes"
    rc=0
  else
    echo "CONTROL: FAIL unguarded refused=$unguarded_fails guarded fails=$guarded_fails"
    rc=1
  fi
  # Only this script removes its own scratch, and only from under _tmp/.
  case "$scratch" in
    "$root/_tmp/daemon-guard-control."*) rm -rf -- "$scratch" ;;
  esac
  exit $rc
fi

check_dir "crates/pii-cli/tests"
echo "RESULT: $ok ok, $fail FAIL"
[ "$fail" -eq 0 ]
