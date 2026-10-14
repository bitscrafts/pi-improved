#!/usr/bin/env bash
# Spec 154 criterion 2, on the real wire: a `kill -9` in the first seconds of a
# run must leave EVIDENCE a later reader can see.
#
# Before the change the restarted daemon answered "unknown to this daemon" for
# the whole run (the journal buffered up to 16 events before writing, so the
# accepted run never reached disk). After it, the run is terminal `failed` with
# reason `interrupted by daemon restart`.
#
# This drives the REAL surfaces — `pii daemon` over the control socket
# (`pii run start` / `pii run get`) and the run's own journal in the store
# (`pii graph trace`) — because a mock answers whatever it is sent.
#
#   bash scripts/checks/daemon-crash-leaves-evidence.sh            # expect the FIX
#   EXPECT=pre bash scripts/checks/daemon-crash-leaves-evidence.sh # expect the BUG
#
# `EXPECT=pre` is the same harness against the un-fixed tree (drop the
# `jr.flush()` in `crates/pii-graph/src/run/mod.rs`): it must then report
# "unknown". Both directions come from the live path, so neither is a
# hand-written input.
#
# Readiness is a real round trip (`pii run ls`), never the presence of a socket
# file: a SIGKILLed daemon leaves `control.sock` behind, and a probe that trusts
# that file says "ready" while the live path is dead.
#
# Writes only under _tmp/verify154/. A stale scratch is moved aside, never
# deleted.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.." || exit 1
ROOT=$PWD
PII=$ROOT/target/debug/pii
S=$ROOT/_tmp/verify154
EXPECT=${EXPECT:-post}

note() { printf '%s\n' "$*"; }
die() { note "FAIL: $*"; exit 1; }

case "$EXPECT" in pre | post) ;; *) die "EXPECT must be pre or post, got '$EXPECT'" ;; esac
[ -x "$PII" ] || die "no binary at $PII (cargo build -p pii-cli)"
if [ -e "$S" ]; then mv "$S" "$S.old-$(date +%s)"; fi
mkdir -p "$S/run/graphs" "$S/ws"

# A graph whose first (and only) node blocks for 30 s, so the crash lands
# mid-node exactly as 061's C-run did.
cat > "$S/run/graphs/slow.toml" <<'TOML'
name = "slow"
entry = "w"

[[node]]
id = "w"
kind = "wait"
timer_ms = 30000
TOML

export PII_RUNTIME_DIR=$S/run
export PII_MODEL=llamacpp-server:granite-4.2-3b
export PII_CONFIG=$S/pii.toml # absent on purpose: Config::default()

start_daemon() { # $1 = tag
    "$PII" daemon --foreground --sandbox off \
        --store "$S/store.redb" --workspace "$S/ws" \
        --log-file "$S/daemon-$1.log" >>"$S/daemon-$1.out" 2>&1 &
    echo $!
}

# A live round trip, not `[ -S control.sock ]`: the socket file outlives a
# SIGKILL and would make this probe lie.
wait_ready() {
    for _ in $(seq 1 400); do
        if "$PII" run ls >/dev/null 2>&1; then return 0; fi
        sleep 0.25
    done
    return 1
}

D1=$(start_daemon 1)
wait_ready || die "daemon 1 never answered (see $S/daemon-1.out)"

START=$("$PII" run start slow --input '{}' 2>&1) || die "run start failed: $START"
RUN=$(printf '%s' "$START" | sed -n 's/.*"run_id":"\([^"]*\)".*/\1/p')
[ -n "$RUN" ] || die "no run_id in the start answer: $START"
note "accepted: run_id=$RUN"

# 2 s in: the node is mid-wait, nothing has finished.
sleep 2
kill -9 "$D1"
for _ in $(seq 1 100); do kill -0 "$D1" 2>/dev/null || break; sleep 0.1; done
kill -0 "$D1" 2>/dev/null && die "the daemon survived kill -9"

D2=$(start_daemon 2)
wait_ready || die "daemon 2 never answered (see $S/daemon-2.out)"

AFTER=$("$PII" run get "$RUN" 2>&1)
note "after restart: pii run get $RUN -> $AFTER"

# The near-miss control: a never-used id must read "unknown" in BOTH directions,
# so "unknown" keeps meaning "never existed" and not "the fix is missing".
GHOST=$("$PII" run get no-such-run-154 2>&1)
note "control (never-existed id) -> $GHOST"

STATUS=0
case "$GHOST" in
    *unknown*) ;;
    *) note "FAIL: a never-existed id no longer reads unknown: $GHOST"; STATUS=1 ;;
esac

if [ "$EXPECT" = "post" ]; then
    case "$AFTER" in
        *'"status":"failed"'*) ;;
        *) note "FAIL: the restarted daemon does not report the run terminal: $AFTER"; STATUS=1 ;;
    esac
    case "$AFTER" in
        *unknown*) note "FAIL: the run is still unknown - no evidence on disk"; STATUS=1 ;;
    esac
    kill -TERM "$D2" 2>/dev/null
    wait "$D2" 2>/dev/null
    TRACE=$("$PII" graph trace "$RUN" --db "$S/store.redb" 2>&1)
    note "journal after restart:"
    printf '%s\n' "$TRACE" | sed -n '1,12p'
    case "$TRACE" in
        *'interrupted by daemon restart'*) ;;
        *) note "FAIL: the run's own journal has no restart terminal"; STATUS=1 ;;
    esac
    [ "$STATUS" -eq 0 ] || exit 1
    note "CRASH-EVIDENCE OK: run $RUN terminal failed, reason 'interrupted by daemon restart'"
else
    case "$AFTER" in
        *unknown*) ;;
        *) note "FAIL: EXPECT=pre but the run is visible: $AFTER"; STATUS=1 ;;
    esac
    kill -TERM "$D2" 2>/dev/null
    wait "$D2" 2>/dev/null
    case "$("$PII" run ls 2>&1)" in
        *"$RUN"*) note "FAIL: EXPECT=pre but the run reached the store"; STATUS=1 ;;
    esac
    [ "$STATUS" -eq 0 ] || exit 1
    note "PRE-FIX OK: run $RUN is unknown to the restarted daemon - no evidence on disk"
fi
