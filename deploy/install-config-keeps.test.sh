#!/usr/bin/env bash
# Spec 329: a re-install keeps the config the operator wrote. Drives deploy/install-pii.sh's config half
# (CONFIG_ONLY=1: sections 4 and 4a, the real promotion path) and reads the result with tomllib.
#
#   bash deploy/install-config-keeps.test.sh                            the cases
#   bash deploy/install-config-keeps.test.sh --control                  the template-only base must lose channel, cron, worker
#   bash deploy/install-config-keeps.test.sh --refuses-the-real-config  CONFIG_DIR=$HOME/.config/pii is refused, nothing written
#
# Python 3.11+ (tomllib) is required: deploy/config-python.sh picks it (PII_PYTHON, python3, or uv into _tmp/).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$REPO/deploy/config-python.sh"
PY="$(config_python "$REPO")" || { echo "ABORT: no Python 3.11+ (tomllib) for the config test" >&2; exit 2; }
INSTALLER="$REPO/deploy/install-pii.sh"
MERGE="$REPO/deploy/pii-config-merge.py"
TEMPLATE="$REPO/examples/pii.production.toml"
UNREACHABLE_LLAMA="http://10.9.9.9:8082/v1"   # CONFIG_ONLY must not probe it (R5)
P='import sys,tomllib;d=tomllib.load(open(sys.argv[1],"rb"))'

# R6: every directory must sit outside $HOME/.config, /etc and /var/lib, and outside the repository tree.
# The one place inside the repository it may use is its _tmp/ scratch.
guard() {
  local d real
  for d in "$@"; do
    real="$(realpath -m "$d")"
    case "$real" in
      "$HOME/.config"|"$HOME/.config"/*|/etc|/etc/*|/var/lib|/var/lib/*) return 1 ;;
      "$REPO/_tmp"|"$REPO/_tmp"/*) ;;
      "$REPO"|"$REPO"/*) return 1 ;;
    esac
  done
  return 0
}

SCRATCH=""
cleanup() {
  # Removes only the scratch this test created (the guarded pattern below), never anything else.
  case "$SCRATCH" in
    "$REPO/_tmp/install-config-keeps."*) find "$SCRATCH" -mindepth 1 -delete; rmdir "$SCRATCH" ;;
  esac
}
trap cleanup EXIT

new_scratch() {
  mkdir -p "$REPO/_tmp"
  SCRATCH="$(mktemp -d "$REPO/_tmp/install-config-keeps.XXXXXX")"
}

# The operator's file, as a real operator could have written it (the measured shape from spec 329).
fixture() {
  case "$1" in
    full) cat <<'TOML'
[core]
model = "openai:glm-5.3-flash"
runtime_dir = "/old/run"
log_file = "/old/pii.log"
session_runs_cap = 512

[llama]
host = "http://10.0.0.5:8082/v1"

[net]
grants = ["10.0.0.9:8084"]

[worker]
name = "w1"

[[channel]]
name = "ops"
kind = "discord"
graph = "discord-triage"

[[cron]]
name = "weekly"
graph = "discord-digest"
schedule = "0 9 * * 1"
input = { to = "1" }
TOML
      ;;
    broken) printf '[core\nmodel = "openai:glm-5.3-flash"\n' ;;
  esac
}

# mkcase <name> [fixture]: a fresh case directory; the operator's file is written only when a fixture is named.
mkcase() {
  CASE="$SCRATCH/$1"
  mkdir -p "$CASE/config" "$CASE/state" "$CASE/run" "$CASE/units" "$CASE/cards"
  if [ -n "${2:-}" ]; then fixture "$2" > "$CASE/config/pii.toml"; fi
}

# cfg_run <case-dir> [NAME=value ...]: the installer's config half, output in <case-dir>/out.txt.
cfg_run() {
  local c="$1"; shift
  LAST_RC=99
  guard "$c/config" "$c/bin" "$c/state" "$c/run" "$c/units" || return 99
  env -i PATH="$PATH" HOME="$HOME" PII_PYTHON="$PY" CONFIG_ONLY=1 SCOPE=user \
    CONFIG_DIR="$c/config" PREFIX="$c/bin" STATE_DIR="$c/state" RUNTIME_DIR="$c/run" UNIT_DIR="$c/units" \
    DEPLOY_CARD_DIR="$c/cards" LLAMA_HOST="$UNREACHABLE_LLAMA" "$@" \
    bash "$INSTALLER" >"$c/out.txt" 2>&1
  LAST_RC=$?
  return "$LAST_RC"
}

# ran <command...>: the assertion holds only when the last cfg_run exited 0 (a refused run must not pass on the old file).
ran() { [ "$LAST_RC" -eq 0 ] && "$@"; }

# tq <python-expression> <file>: exit 0 when the expression (over the parsed TOML `d`) holds.
tq() { "$PY" -I -c "$P; $1" "$2"; }

# The verifier must see the template-only base lose channel, cron and worker (the control for R4's check).
case_control() {
  mkcase control full
  local out
  out="$("$PY" -I "$MERGE" verify "$CASE/config/pii.toml" "$TEMPLATE" 2>&1)" && return 1
  grep -q "MISSING TABLE channel" <<<"$out" && grep -q "MISSING TABLE cron" <<<"$out" && grep -q "MISSING TABLE worker" <<<"$out"
}

case_keeps_worker() {
  mkcase keeps-worker full
  cfg_run "$CASE"
  check "keeps-a-section-the-template-lacks" \
    ran tq 'sys.exit(0 if d["worker"]["name"]=="w1" else 1)' "$CASE/config/pii.toml"
}

case_keeps_channel_and_cron() {
  mkcase keeps-channel full
  cfg_run "$CASE" PII_DISCORD_TOKEN=test-token PII_DISCORD_CHANNEL_ID=42
  check "keeps-an-extra-channel-and-cron" \
    ran tq 'ch={c["name"] for c in d.get("channel",[])}; cr={c["name"] for c in d.get("cron",[])}; sys.exit(0 if {"ops","discord"}<=ch and {"weekly","digest"}<=cr else 1)' \
    "$CASE/config/pii.toml"
}

case_keeps_grants() {
  mkcase keeps-grants full
  cfg_run "$CASE"
  check "keeps-the-operators-net-grants" \
    ran tq 'sys.exit(0 if d["net"]["grants"]==["10.0.0.9:8084"] else 1)' "$CASE/config/pii.toml"
}

case_applies_owned_keys_only() {
  mkcase owns-keys full
  cfg_run "$CASE"
  check "applies-only-the-keys-it-owns" \
    ran tq "sys.exit(0 if d['llama']['host']=='$UNREACHABLE_LLAMA' and d['core']['log_file']=='$CASE/state/pii.log' and d['core']['session_runs_cap']==512 and d['core']['runtime_dir']=='$CASE/run' else 1)" \
    "$CASE/config/pii.toml"
}

case_first_install() {
  mkcase first-install
  cfg_run "$CASE"
  check "first-install-still-works" \
    ran tq 'sys.exit(0 if {"core","llama","memory","net"}<=set(d) and d["core"]["model"]=="llamacpp-server:granite-4.2-3b" and d["net"]["grants"]==["127.0.0.1:11434"] else 1)' \
    "$CASE/config/pii.toml"
}

case_append_is_idempotent() {
  mkcase idempotent full
  cfg_run "$CASE" PII_DISCORD_TOKEN=test-token PII_DISCORD_CHANNEL_ID=42
  cfg_run "$CASE" PII_DISCORD_TOKEN=test-token PII_DISCORD_CHANNEL_ID=42
  check "append-is-idempotent-by-name" \
    ran tq 'sys.exit(0 if [c["name"] for c in d["channel"]].count("discord")==1 and [c["name"] for c in d["cron"]].count("digest")==1 else 1)' \
    "$CASE/config/pii.toml"
}

case_refuses_unparseable() {
  mkcase refuses-broken broken
  cp "$CASE/config/pii.toml" "$CASE/prior.orig"
  local rc=0
  cfg_run "$CASE" || rc=$?
  local ok=0
  [ "$rc" -ne 0 ] && cmp -s "$CASE/config/pii.toml" "$CASE/prior.orig" \
    && grep -q "does not parse" "$CASE/out.txt" && grep -q "line 1" "$CASE/out.txt" && ok=1
  check "refuses-an-unparseable-prior" test "$ok" -eq 1
}

case_refuses_real_config() {
  ! guard "$HOME/.config/pii"
}

# No card, no built binaries, and an unreachable llama host: the config half still runs and writes the config.
case_config_only() {
  mkcase config-only
  cfg_run "$CASE" && [ -s "$CASE/config/pii.toml" ] && [ ! -e "$CASE/bin/pii" ] && [ ! -e "$CASE/units/pii.service" ]
}

check() {
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "ok   $name"; PASS=$((PASS + 1))
  else
    echo "FAIL $name"; FAIL=$((FAIL + 1))
  fi
}

MODE="${1:-}"
PASS=0
FAIL=0
case "$MODE" in
  --refuses-the-real-config)
    if guard "$HOME/.config/pii"; then
      echo "FAIL: the real config dir was not refused"; exit 1
    fi
    echo "REFUSED: CONFIG_DIR is the operator's config ($HOME/.config/pii): the test writes only under _tmp/, nothing written"
    exit 1 ;;
  --control)
    new_scratch
    if case_control; then
      echo "CONTROL: the template-only base loses channel, cron, worker and the verifier sees all three"
      exit 0
    fi
    echo "CONTROL FAILED: the verifier did not report channel, cron and worker missing from the template-only base"
    exit 1 ;;
  "")
    new_scratch
    case_keeps_worker
    case_keeps_channel_and_cron
    case_keeps_grants
    case_applies_owned_keys_only
    case_first_install
    case_append_is_idempotent
    case_refuses_unparseable
    check "refuses-the-real-config-dir" case_refuses_real_config
    check "config-only-skips-build-and-card" case_config_only
    check "control-the-template-only-base-loses-all-three" case_control
    echo "RESULT: $PASS ok, $FAIL FAIL"
    [ "$FAIL" -eq 0 ] ;;
  *) echo "usage: $0 [--control | --refuses-the-real-config]" >&2; exit 2 ;;
esac
