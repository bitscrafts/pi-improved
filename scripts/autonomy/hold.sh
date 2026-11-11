#!/usr/bin/env bash
# hold.sh — the TREE hold: who may write this working tree right now.
#
# WHY THIS EXISTS (2026-11-01, after three collisions in one day). Three callers can write /projects/pi-improved:
#   · an interactive turn reading the repo with a person on the other end (the one that must never be surprised),
#   · the */5 drain (no model) which dispatches a cycle, and
#   · the skill-lane agent cron job (every 6 h) which authors specs and can dispatch.
# The cycle's flock protects a CYCLE from itself; it says nothing about a human-facing turn that is mid-edit with
# no lock at all. So a worker was dispatched into a tree an interactive turn was editing (12:21Z), and at 00:15Z and
# 06:15Z an agent authored and committed while the same tree was being read. Nothing was lost by luck.
#
# PRIORITY (2026-11-01, the user's "priority switch"). Not every caller is equal, and a boolean hold cannot say so:
#   · an interactive turn with a person on the other end is 100 and is never preempted,
#   · an interactive agent working this tree on a person's instruction is 80,
#   · a cycle or a worker is 40,
#   · a bare tick/beat is 20.
# A caller whose priority is HIGHER than the holder's may take over ONLY when the holder is automation (below 100):
# a person's turn must never be blocked by a cron job that took the hold first. Nothing may preempt a hold at 100.
# A cron caller always yields on ANY hold whatever the numbers say -- the switch decides who may take over, never
# who may run beside whom.
#
# THE RULE. A hold names ONE owner and an expiry. Every entry point that can write or spend checks it first:
#   · a CRON caller (`--from-cron`, or PII_CRON=1) never waits and never wins a hold — it prints one line and exits
#     0. The flag only ever REDUCES what a caller may do, so claiming "I am cron" cannot gain anything.
#   · an INTERACTIVE caller takes the hold for itself (TTL, refreshable) and releases it when done.
#   · a hold whose pid is dead, or whose TTL has passed, is reaped on the next check and the takeover is said
#     aloud — a crashed holder must never wedge the loop, which is the failure mode a pure lock file has.
#
# usage: hold.sh take <owner> [ttl_secs] | check | release <owner> | reap | status
. "$(dirname "$(readlink -f "$0")")/env.sh"   # PII_ROOT: the repository this queue belongs to (spec 405)
set -uo pipefail
ROOT=$PII_ROOT
HOLD=${PII_HOLD:-$ROOT/_tmp/autonomy/hold}
TTL_DEFAULT=${PII_HOLD_TTL:-2700}          # 45 min: longer than any cycle, shorter than a lost afternoon

now(){ date -u +%s; }
iso(){ [ -n "${1:-}" ] && date -u -d "@$1" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "-"; }

# field <n>: the n-th whitespace field of the hold file (1=owner 2=pid 3=since 4=expires)
field(){ awk -v n="$1" 'NR==1{print $n}' "$HOLD" 2>/dev/null; }

alive(){ case "${1:-}" in ''|-|0) return 1 ;; esac; kill -0 "$1" 2>/dev/null; }

# expired: an empty/garbled hold is expired; a hold past its TTL is expired
expired(){
  local e; e=$(field 4); [ -n "$e" ] || return 0
  [ "$(now)" -ge "$e" ] 2>/dev/null && return 0
  return 1
}

# reap: drop a hold whose holder is gone. Says so, because a silent takeover hides a crash.
reap(){
  [ -f "$HOLD" ] || return 0
  local pid owner; pid=$(field 2); owner=$(field 1)
  # only reap on a pid that was DECLARED and is now dead; an undeclared pid ("-") leaves TTL as the sole clock,
  # because the pid of a shell-based holder is gone by the next breath and reaping on it erased real holds
  case "$pid" in ''|-|0) : ;; *)
    if [ "$pid" != "-" ] && ! alive "$pid"; then
      mv "$HOLD" "$HOLD.reaped.$(now).$$" 2>/dev/null || rm -f "$HOLD"
      echo "hold: reaped a hold left by '$owner' (pid $pid is gone)"; return 0
    fi ;;
  esac
  if expired; then
    mv "$HOLD" "$HOLD.reaped.$(now).$$" 2>/dev/null || rm -f "$HOLD"
    echo "hold: reaped a hold left by '$owner' (ttl passed at $(iso "$(field 4)"))"; return 0
  fi
  return 1
}

take(){
  local owner=${1:?owner required} ttl=${2:-$TTL_DEFAULT} prio=${3:-50}
  # The pid must be the CALLER's, not this script's: `bash hold.sh take` exits immediately, so recording $$ made every
  # hold look dead to the very next check and be reaped. PPID is the shell that asked; PII_HOLD_PID overrides it for
  # a holder that is not the immediate parent (a cron job taking the hold for a worker it is about to spawn).
  # no pid unless the caller states one: PII_HOLD_PID for a holder that will outlive this call. Otherwise "-" and
  # the TTL is the whole story -- which is what an interactive turn wants (45 min, refreshed, released when done).
  local pid=${PII_HOLD_PID:--}
  if [ -f "$HOLD" ] && ! reap; then
    local hpid hprio; hpid=$(field 2); hprio=$(field 5)
    if [ "${prio:-50}" -gt "${hprio:-50}" ] 2>/dev/null && [ "${hprio:-50}" -lt 100 ] 2>/dev/null; then
      mv "$HOLD" "$HOLD.preempted.$(now).$$" 2>/dev/null || true
      echo "hold: TAKE-OVER from '$(field 1)' (priority ${hprio}) by '$owner' (priority ${prio}): automation yields to a person"
    else
      echo "hold: TAKEN-BY $(field 1) pid=$hpid since=$(iso "$(field 3)") until=$(iso "$(field 4)") priority=${hprio:-50}"
      return 3
    fi
  fi
  mkdir -p "$(dirname "$HOLD")" || return 1
  printf '%s %s %s %s %s\n' "$owner" "$pid" "$(now)" "$(( $(now) + ttl ))" "$prio" > "$HOLD"
  echo "hold: TAKEN $owner pid=$pid until=$(iso "$(( $(now) + ttl ))") priority=$prio"
}

check(){
  if [ -f "$HOLD" ] && ! reap; then
    echo "hold: HELD-BY $(field 1) pid=$(field 2) since=$(iso "$(field 3)") until=$(iso "$(field 4)")"
    return 3
  fi
  echo "hold: FREE"
  return 0
}

release(){
  local owner=${1:?owner required}
  [ -f "$HOLD" ] || { echo "hold: FREE (nothing to release)"; return 0; }
  if [ "$(field 1)" != "$owner" ]; then
    echo "hold: REFUSED to release '$owner': it is held by '$(field 1)'"; return 3
  fi
  mv "$HOLD" "$HOLD.released.$(now).$$" 2>/dev/null || rm -f "$HOLD"
  echo "hold: RELEASED $owner"
}

status(){
  if [ -f "$HOLD" ]; then
    printf 'owner=%s pid=%s priority=%s since=%s until=%s alive=%s expired=%s\n' \
      "$(field 1)" "$(field 2)" "$(field 5)" "$(iso "$(field 3)")" "$(iso "$(field 4)")" \
      "$(alive "$(field 2)" && echo yes || echo no)" "$(expired && echo yes || echo no)"
  else
    echo "owner=- pid=- priority=- since=- until=- alive=- expired=-"
  fi
}

# yield_if_held <who> — the one call every cron-side entry point makes. Exit 0 always: a background caller that
# finds the tree held has nothing to do and must not look like a failure (drain.sh already exits 0 for its own
# benign refusals, and a cron job that exits non-zero alarms every 5 minutes for no fault).
yield_if_held(){
  local who=${1:-cron}
  if [ -f "$HOLD" ] && ! reap >/dev/null 2>&1; then
    echo "$who: the tree is held by $(field 1) (until $(iso "$(field 4)")) — yielding, nothing started, nothing spent"
    exit 0
  fi
}

case "${1:-}" in
  take)    shift; take "$@" ;;
  check)   check ;;
  release) shift; release "$@" ;;
  reap)    reap && echo "hold: FREE" ;;
  status)  status ;;
  *) echo "usage: hold.sh take <owner> [ttl] [priority] | check | release <owner> | reap | status" >&2; exit 2 ;;
esac
