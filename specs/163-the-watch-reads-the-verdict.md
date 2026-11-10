# Spec: heal_detect reads the gate's recorded verdict

## Overview
heal_detect shells `cargo xtask <gate>` inside the watch, which builds a second xtask into an un-warmed target (73 s idle,
103 s under load) and cannot distinguish working from wedged. It should read the verdict the gate already recorded and
spawn nothing.

**Project**: <repo>

## Exit Criteria
- [ ] `cargo test -p pii-server --test boss_watch`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.
