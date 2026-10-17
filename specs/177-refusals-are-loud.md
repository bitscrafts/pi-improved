# Spec: refusals are loud

## Overview
069 item 3: the class that hid fourteen hours of zero runs. A cron entry that cannot start, or a channel run refused, must be an EVENT and not a silence.

**Project**: /projects/pi-improved

## Requirements
start_cron_entries and poll_once emit cron.start.refused / channel.run.refused; the on-finish re-arm gets a minimum interval.

## Exit Criteria
- [ ] cargo test -p pii-learn --test incidents_runtime_production
- [ ] grep -q cron.start.refused crates/pii-observe/src/names.rs

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
