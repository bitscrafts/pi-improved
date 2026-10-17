# Spec: the timer does not re-fire

## Overview
MEASURED 2026-10-17 22:39Z on the DEPLOYED daemon: with the digest cron live and the reactor fix in place, runs climbed 91 to 516 in four minutes - roughly three per second - for a graph scheduled "0 9 * * *". Either the timer node does not suspend after firing, or the on-finish re-arm has no minimum interval.

**Project**: /projects/pi-improved

## Requirements
After a timer fires, the run must suspend until the next cron instant: one run per scheduled occurrence, provable by counting runs over a window far longer than the interval. The re-arm gets a minimum interval and a refusal event if a graph would fire again immediately.

## Exit Criteria
- [ ] cargo test -p pii-server --test timer_no_refire
- [ ] cargo test -p pii-graph --test deadline_flush

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
