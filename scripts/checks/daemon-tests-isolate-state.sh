#!/usr/bin/env bash
# Spec 425: a test that spawns a `pii daemon` must own every path that daemon takes.
#
# MEASURED 2026-11-21: `crates/pii-cli/tests/memory_learn.rs` spawned the daemon with `PII_RUNTIME_DIR` and
# `PII_CONFIG` but not `PII_STATE_DIR`, so the child resolved its durable stores to `$HOME/.local/state/pii` and died on
#   store /root/.local/state/pii/durable/channel.redb is locked: another daemon holds it
# while the operator's daemon was up. The test then reported `the daemon never accepted memory.learn`. The socket was
# never the collision: the runtime dir was already the test's own. The store was.
#
# Usage: bash scripts/checks/daemon-tests-isolate-state.sh            # scan crates/pii-cli/tests
#        bash scripts/checks/daemon-tests-isolate-state.sh --control  # prove this checker can fail, under _tmp/
#        bash scripts/checks/daemon-tests-isolate-state.sh <dir>      # scan <dir> (used by --control and by hand)
# Prints `PASS: <n> FAIL: <m>` and `inspected: <k> daemon spawn site(s) in <f> file(s)`; exits 0 only when m = 0 and
# k > 0. A scan that inspects nothing FAILS: a checker that silently finds no tests is worse than no checker.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 2

# One spawn site per line of output from `scan_file`:
#   SITE <line> <verdict> <fn> <why>
# verdict is ok | fail | exempt. A "daemon spawn site" is a line that passes "daemon" as an argument to the built
# binary (`.arg("daemon")` or `.args([..., "daemon", ...])`), which is the form every test in this suite uses and the
# form that finds the helper functions (`start_daemon`, `spawn_daemon`, `daemon`) as well as the inline spawns.
#
# The window is the ENCLOSING FUNCTION, not the statement: the command is assembled across `let mut cmd = ...` and a
# later `daemon_guard::spawn(&mut cmd)`, so a statement-scoped scan would miss env lines in the same function. It is
# deliberately not wider than the function: a helper (`pii(dir)`, `fx.envs(cmd)`) that sets the state dir for the
# CLIENTS must not launder a daemon spawn in another function — that is exactly the memory_learn.rs shape, and the
# --control fixture reproduces it.
scan_file() {
  awk '
    function fn_start(l) { return l ~ /^(pub(\([a-z]+\))? )?(async )?fn [A-Za-z0-9_]+/ }
    function env_value(s, key,   v, k) {
      k = ".env(\"" key "\","
      if (index(s, k) == 0) { return "" }
      v = substr(s, index(s, k) + length(k) + 1)
      sub(/^[[:space:]]+/, "", v)
      sub(/\)[[:space:]]*,?[[:space:]]*$/, "", v)
      sub(/[[:space:]]+$/, "", v)
      return v
    }
    # An operator path is the defect: the daemon then opens the stores a live daemon already holds. A bare string
    # literal is refused too, because no scratch path in this suite is a literal: they are all built from a TempDir.
    function unowned(v,   why) {
      if (v == "") { return "sets no value" }
      if (v ~ /^"/) { return "names a literal path (" v "), not the test scratch" }
      if (v ~ /HOME|HOME\}|\/root|\/home\/|\/run\/user|~/) { return "names the operator path (" v ")" }
      return ""
    }
    { line[NR] = $0 }
    END {
      nfn = 0
      for (i = 1; i <= NR; i++) {
        if (fn_start(line[i])) {
          if (nfn > 0) { fn_end[nfn] = i - 1 }
          nfn++; fn_begin[nfn] = i; fn_name[nfn] = line[i]
          sub(/[[:space:]]*\(.*$/, "", fn_name[nfn])
          sub(/^.*fn /, "", fn_name[nfn])
        }
      }
      if (nfn > 0) { fn_end[nfn] = NR }
      # A top-level closing brace ends the function; anything after it is not this test.
      for (f = 1; f <= nfn; f++) {
        for (j = fn_begin[f]; j <= fn_end[f]; j++) {
          if (j > fn_begin[f] && line[j] == "}") { fn_end[f] = j; break }
        }
      }
      # Every marker LINE is judged exactly once. A marker outside any top-level fn is reported as a FAIL, not
      # skipped: a line this scan silently dropped would be a rule it never looked at.
      for (j = 1; j <= NR; j++) {
        if (line[j] ~ /^[[:space:]]*\/\//) { continue }
        if (line[j] !~ /\.arg\("daemon"\)/ && line[j] !~ /\.args\(.*"daemon"/) { continue }
        f = 0
        for (g = 1; g <= nfn; g++) {
          if (j >= fn_begin[g] && j <= fn_end[g]) { f = g }
        }
        if (f == 0) {
          print "SITE\t" j "\tfail\t(no top-level fn)\tsits outside any top-level fn: this scan cannot judge its env"
          continue
        }
        state = ""; runtime = ""; exempt = ""
        for (k = fn_begin[f]; k <= fn_end[f]; k++) {
          if (state == "" && index(line[k], ".env(\"PII_STATE_DIR\",") > 0) {
            state = env_value(line[k], "PII_STATE_DIR")
          }
          if (runtime == "" && index(line[k], ".env(\"PII_RUNTIME_DIR\",") > 0) {
            runtime = env_value(line[k], "PII_RUNTIME_DIR")
          }
          if (runtime == "" && index(line[k], ".env(\"XDG_RUNTIME_DIR\",") > 0) {
            runtime = env_value(line[k], "XDG_RUNTIME_DIR")
          }
          if (index(line[k], "425-exempt:") > 0) {
            exempt = line[k]
            sub(/^.*425-exempt:/, "", exempt)
            sub(/[[:space:]]*(\/\/)?[[:space:]]*$/, "", exempt)
          }
        }
        why = ""
        if (state == "") { why = "sets no PII_STATE_DIR (the durable stores go to $HOME/.local/state/pii)" }
        else if ((w = unowned(state)) != "") { why = "PII_STATE_DIR " w }
        if (runtime == "") {
          why = why (why == "" ? "" : "; ") "sets no PII_RUNTIME_DIR/XDG_RUNTIME_DIR (the socket goes to the operator runtime dir)"
        } else if ((w = unowned(runtime)) != "") {
          why = why (why == "" ? "" : "; ") "runtime dir " w
        }
        if (why == "") {
          print "SITE\t" j "\tok\t" fn_name[f]
        } else if (exempt != "") {
          print "SITE\t" j "\texempt\t" fn_name[f] "\t" exempt
        } else {
          print "SITE\t" j "\tfail\t" fn_name[f] "\t" why
        }
      }
    }
  ' "$1"
}

# Scans every `.rs` under one test directory. `daemon_guard/mod.rs` is the one owner of the child, not a test target:
# it spawns whatever command it is handed, and its own `.spawn()` is not a `pii daemon`.
check_dir() {
  local dir="$1" file f
  ok=0
  fail=0
  inspected=0
  files=0
  # -print0/read -d '' so a path with a space is scanned as one file, not silently split and skipped.
  while IFS= read -r -d '' file; do
    case "$file" in
      */daemon_guard/*) continue ;;
    esac
    case "$(basename "$file")" in
      mod.rs) continue ;;
    esac
    files=$((files + 1))
    while IFS="$(printf '\t')" read -r _ ln verdict fn rest; do
      [ -n "${ln:-}" ] || continue
      inspected=$((inspected + 1))
      case "$verdict" in
        ok) ok=$((ok + 1)) ;;
        exempt)
          echo "EXEMPT $file:$ln ($fn): $rest"
          ok=$((ok + 1))
          ;;
        fail)
          echo "FAIL $file:$ln ($fn): $rest"
          fail=$((fail + 1))
          ;;
      esac
    done <<EOF
$(scan_file "$file")
EOF
  done < <(find "$dir" -name '*.rs' -type f -print0 2>/dev/null | sort -z)
  echo "RESULT: PASS: $ok FAIL: $fail"
  echo "inspected: $inspected daemon spawn site(s) in $files file(s)"
}

# The verdict for one tree: 0 only when every spawn site is owned AND something was actually inspected. A scan that
# finds no daemon spawn is a broken scan, not a clean tree: it certifies a rule it never looked at.
scan_tree() {
  check_dir "$1"
  [ "$fail" -eq 0 ] || return 1
  if [ "$inspected" -eq 0 ]; then
    echo "FAIL: inspected no daemon spawn at all in $1; a check that finds nothing cannot pass"
    return 1
  fi
  return 0
}

if [ "${1:-}" = "--control" ]; then
  scratch="$(mktemp -d "$root/_tmp/daemon-isolation-control.XXXXXX")" || {
    echo "CONTROL: FAIL no scratch directory"
    exit 1
  }
  mkdir -p "$scratch/unowned" "$scratch/owned"
  # The near-miss: the SHAPE of memory_learn.rs before this row. The client helper owns the state dir, so a naive
  # scan reads PII_STATE_DIR in the file and passes it; the daemon's own command owns only the runtime dir. This
  # must be reported, and it must be reported at the spawn line, not anywhere in the file.
  cat > "$scratch/unowned/memory_learn.rs" <<'EOF'
use std::process::Command;

fn pii(args: &[&str], runtime: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", runtime)
        .env("PII_STATE_DIR", runtime)
        .output()
        .unwrap()
}

#[test]
fn an_outcome_is_recorded_once() {
    let d = scratch();
    let mut child = daemon_guard::spawn(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(d.path())
            .env("PII_RUNTIME_DIR", d.path())
            .env("PII_CONFIG", d.path().join("pii.toml"))
            .stderr(Stdio::null()),
    );
    child.kill_now();
}
EOF
  # The same test, owning both paths at the spawn.
  cat > "$scratch/owned/memory_learn.rs" <<'EOF'
use std::process::Command;

#[test]
fn an_outcome_is_recorded_once() {
    let d = scratch();
    let mut child = daemon_guard::spawn(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .env("PII_RUNTIME_DIR", d.path())
            .env("PII_STATE_DIR", d.path())
            .stderr(Stdio::null()),
    );
    child.kill_now();
}
EOF

  echo "== control: a daemon spawn whose state dir is set only for its clients (the memory_learn shape)"
  scan_tree "$scratch/unowned"
  unowned_rc=$?
  unowned_fail=$fail
  unowned_inspected=$inspected
  echo "== control: the same spawn owning both paths"
  scan_tree "$scratch/owned"
  owned_rc=$?
  owned_fail=$fail
  owned_ok=$ok
  owned_inspected=$inspected
  echo "== control: a directory with no daemon spawn at all (a checker that inspects nothing must not pass)"
  mkdir -p "$scratch/unowned-empty"
  scan_tree "$scratch/unowned-empty"
  empty_rc=$?
  empty_inspected=$inspected

  rc=0
  [ "$unowned_rc" -eq 1 ] && [ "$unowned_inspected" -eq 1 ] && [ "$unowned_fail" -eq 1 ] || {
    echo "CONTROL: FAIL the unowned spawn was not refused (rc=$unowned_rc inspected=$unowned_inspected fail=$unowned_fail)"
    rc=1
  }
  [ "$owned_rc" -eq 0 ] && [ "$owned_inspected" -eq 1 ] && [ "$owned_fail" -eq 0 ] && [ "$owned_ok" -eq 1 ] || {
    echo "CONTROL: FAIL the owned spawn was not accepted (rc=$owned_rc inspected=$owned_inspected ok=$owned_ok fail=$owned_fail)"
    rc=1
  }
  [ "$empty_rc" -eq 1 ] && [ "$empty_inspected" -eq 0 ] || {
    echo "CONTROL: FAIL an empty scan was not refused (rc=$empty_rc inspected=$empty_inspected)"
    rc=1
  }
  [ "$rc" -eq 0 ] && echo "CONTROL: the unowned spawn is refused, the owned one passes, and a scan that inspects nothing is refused"
  # Only this script removes its own scratch, and only from under _tmp/.
  case "$scratch" in
    "$root/_tmp/daemon-isolation-control."*) rm -rf -- "$scratch" ;;
  esac
  exit $rc
fi

scan_tree "${1:-crates/pii-cli/tests}" || exit 1
echo "OK: every daemon spawn site owns its state and runtime dirs"