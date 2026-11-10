# DEFECT 2026-10-16 — the tick reports the gateway down while it runs, and skips pause + questions

Found by the 18:15Z supervisor tick, while measuring the selftest's two red rows.

## The measured fact

At **18:15:20Z** the loop's own heartbeat wrote the phone line

> `pii autonomy · the gateway is down and would not start — messages cannot be delivered`

into `_tmp/autonomy/last-reported` (that file's content is now exactly `gw-still-down`, mtime 18:15Z), and its
start attempt was refused because the gateway was **already running** — the redirect target that `tick.sh` line 38
writes, `<agent-host-config>`:

```
✗ Another gateway instance is already running (PID 73).
  Use 'hermes gateway restart' to replace it,
  or 'hermes gateway stop' first.
  Or use 'hermes gateway run --replace' to auto-replace.
```

The gateway was not down. Measured in the same window, read-only:

```
$ hermes gateway status | head -2
✓ Gateway is running (PID: 73)
  (Running manually, not as a system service)
$ ps -eo pid,etime,cmd | grep '[g]ateway run'
     73    19:34:18  .../hermes gateway run --accept-hooks        # up ~19.5 h, since 2026-10-15T22:45Z
```

Nothing about the alarm is true: the gateway has been up for nineteen and a half hours, and its restart attempt
was refused *because it was up*.

## The mechanism

`scripts/autonomy/tick.sh` lines 28-47 decide the gateway's health from one string, with no timeout:

```sh
if command -v hermes >/dev/null 2>&1 && ! hermes gateway status 2>/dev/null | grep -q 'Gateway is running'; then
  ... setsid nohup hermes gateway run ><agent-host-config> 2>&1 </dev/null &
  sleep 12
  if hermes gateway status 2>/dev/null | grep -q 'Gateway is running'; then say_once "gw-up" ...
  else                                                              say_once "gw-still-down" ...   # line 43
  fi
  exit 0                                                                                            # line 46
fi
```

`hermes gateway status` is **slow, not fast**: measured five times in a row on a quiet host, every run answered
correctly and every run took **13.6-14.0 s** (`hermes gateway status | grep -c 'Gateway is running'` → `1`,
13781/13628/13976/13681/13625 ms). A recorded outlier in the loop's own notes is **2m23s** inside this same call.
So the check is a race: whenever the status call answers slowly — or answers nothing under contention, which is
what the empty answer at 18:15 must have been — the branch fires. It then starts a *second* gateway, which the
running instance refuses, and the retry twelve seconds later is asked the same question that just failed.

Three consequences, in order of severity:

1. **A false alarm on the delivered path.** The 30-minute cron job (`pii-autonomy-tick.sh`, `deliver=all`) prints
   this line verbatim to the phone. The 5-minute watchdog discards tick output (`tick.sh >/dev/null 2>&1`), so
   whether *this* instance reached the phone depends on which scheduler ran it; the wording it would send is wrong
   either way.
2. **The tick exits before pause and questions** (line 46). Every other branch below — the operator's `PAUSED`
   switch (line 91) and the open-question carriage (line 114) — is skipped for that tick. With fifteen open
   questions in `_tmp/autonomy/questions.md` (the `[ ]` rows from 10-13 and 10-15), the tick that skips them is
   the tick the user needed.
3. **The selftest rows follow.** In the 18:19 run (`_tmp/autonomy/logs/selftest-20261016T181908Z.log`):
   `FAIL an open question is carried to the phone  expected=0 actual=1` /
   `FAIL it is not repeated every tick  expected=1 actual=0` — the *first* `tick.sh` call was swallowed by this
   branch and printed nothing; the *second* answered and printed the question, so the pair failed in the inverted
   direction. The 12:18Z run failed one row, `paused -> the tick says so (once)`, from the same branch. Both
   runs' near-miss controls passed (`the answer is recorded`, `paused -> the cycle spends nothing`), which is what
   makes this a defect in the branch and not a broken switch.

## Why it is worth fixing now

It is the alarm channel that tells the user the loop is alive. An alarm that fires a false "messages cannot be
delivered" and then skips the questions it exists to deliver is worse than silence: it spends the user's
attention on a state the box is not in.

## The minimal fix (not applied by this tick)

- decide "is a gateway running" from the **process** first, and treat the status string as a second opinion:
  `pgrep -f 'hermes gateway run'` answering a pid means up, whatever the status call says;
- give the status call a **bounded wait** (it is a 14 s call in the healthy case — anything past ~60 s is not an
  answer);
- when the start attempt is refused with "Another gateway instance is already running", that is **`gw-up`**, not
  `gw-still-down` — the refusal is proof the gateway exists;
- and do not `exit 0` on a health line alone: fall through to the pause and question branches, which are
  independent of the gateway's state.

Not applied here because `tick.sh` is the heartbeat the watchdog invokes every five minutes: an edit under a live
invocation is the failure this loop already paid for once (DO-NOT-REPEAT 2.15). The change must land as an atomic
`mv` of a tested file, or between beats.

## What would prove the fix

A selftest row with its near-miss control, both hermetic:

- **the defect:** a `hermes` stub early on `PATH` whose `gateway status` prints nothing (or sleeps past the
  bound) while the real gateway process is up → the tick must **not** print "down", must not attempt a start, and
  must still carry an open question;
- **the control:** the same stub, with no gateway process running, whose start *succeeds* → the tick must say
  `gw-up` exactly once and stay silent on the next beat.

Both belong in `scripts/autonomy/selftest.sh` section 5/5b, next to the rows they currently destabilise.
