#!/usr/bin/env bash
# Step runner for quality-gate.sh (spec 146). Sourced, never run directly.
#
# run_step <label> <command string>
#   Runs the command in its own session, streams its output, and while it runs prints
#     HEARTBEAT step=<label> elapsed=<n>s last: <the step's last output line>
#   at least every QG_HEARTBEAT_SECS (default 30). A heartbeat is a progress note only: the
#   step's exit code is the sole verdict (it prints `PASS: <label>` / `FAIL: <label> ...`).
#   A step whose output has not changed for QG_STALL_SECS (default 1800) is killed and reported
#   as HUNG; a step that dies on a signal is reported as killed. Both name the step, the seconds
#   since its last heartbeat and its last output line. Returns 0 on pass, 1 otherwise.
#   A step that prints a line starting `SKIPPED:` and exits 0 gets no PASS line: SKIPPED is the record.

_qg_last_line() {
    tr '\r' '\n' <"$1" | grep -v '^$' | tail -n 1 | cut -c1-200
}

run_step() {
    local label=$1 cmd=$2
    local hb=${QG_HEARTBEAT_SECS:-30} stall=${QG_STALL_SECS:-1800}
    local logdir=target/gate-steps log
    mkdir -p "$logdir"
    log="$logdir/${label//[^A-Za-z0-9_.-]/_}.log"
    : >"$log"
    echo "==> $label"

    local start=$SECONDS last_beat=$SECONDS last_change=$SECONDS size=0 now cursize last hung=0
    setsid bash -c "$cmd" >"$log" 2>&1 </dev/null &
    local pid=$!
    tail -n +1 -f --pid="$pid" "$log" &
    local tailpid=$!

    while kill -0 "$pid" 2>/dev/null; do
        sleep 0.25
        now=$SECONDS
        cursize=$(stat -c %s "$log" 2>/dev/null || echo 0)
        if [ "$cursize" != "$size" ]; then
            size=$cursize
            last_change=$now
        fi
        if [ $((now - last_change)) -ge "$stall" ]; then
            hung=1
            kill -TERM -- "-$pid" 2>/dev/null
            sleep 1
            kill -KILL -- "-$pid" 2>/dev/null
            break
        fi
        if [ $((now - last_beat)) -ge "$hb" ]; then
            last=$(_qg_last_line "$log")
            echo "HEARTBEAT step=$label elapsed=$((now - start))s last: ${last:-(no output yet)}"
            last_beat=$now
        fi
    done
    wait "$pid" 2>/dev/null
    local rc=$?
    wait "$tailpid" 2>/dev/null

    last=$(_qg_last_line "$log")
    now=$SECONDS
    if [ "$hung" -eq 1 ]; then
        echo "FAIL: $label HUNG: no output for $((now - last_change))s (limit ${stall}s), $((now - last_beat))s since last heartbeat, ran $((now - start))s; last line: ${last:-(none)}"
        return 1
    elif [ "$rc" -gt 128 ]; then
        echo "FAIL: $label killed by signal $((rc - 128)) after $((now - start))s, $((now - last_beat))s since last heartbeat; last line: ${last:-(none)}"
        return 1
    elif [ "$rc" -ne 0 ]; then
        echo "FAIL: $label"
        return 1
    elif grep -q '^SKIPPED:' "$log"; then
        return 0
    fi
    echo "PASS: $label"
}
