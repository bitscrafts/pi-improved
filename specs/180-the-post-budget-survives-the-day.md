# Spec: the post budget survives the day

## Overview
MEASURED 2026-10-17 22:39Z, verbatim from the deployed daemon log:

    {"error":"capability denied: net: byte budget exhausted","channel":"discord"} kind=channel.egress.denied

027895e moved the net byte window to the CONNECTION, and the post is still denied under a daemon that has made hundreds of requests. Whatever accumulates must be bounded per poll or per run rather than over the daemon life, and the refusal must stay loud.

**Project**: /projects/pi-improved

## Requirements
A channel or a cron run must be able to run for a day without exhausting its byte budget, with a scripted test that makes thousands of requests and still posts, AND a single oversized response still refused.

## Exit Criteria
- [ ] cargo test -p pii-net --test budget_survives_a_day
- [ ] cargo test -p pii-channels --test discord

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT simply raise the constant: the defect is the lifetime the window is applied over.
