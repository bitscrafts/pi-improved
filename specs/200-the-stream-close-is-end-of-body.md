# Spec: the stream close is end of body

## Overview
071 section 3: the contradiction. A diagnostic compiled into target/release/pii never printed, while the same net: read: peer closed close_notify text kept appearing. A string in a binary proves a literal exists, not that its code path ran. The decisive test is BEHAVIOURAL.

**Project**: <repo>

## Requirements
Patch the arm to emit an unmistakable token, rebuild, run the identical prompt, and see which token appears. Then: an end-of-stream after at least one frame completes the body with the answer kept and an EMPTY output.error; a close before any frame remains an error.

## Exit Criteria
- [ ] cargo test -p pii-model --test stream_close_is_eof
- [ ] cargo test -p pii-net --test stream_close_is_eof

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT keep the diagnostic token in the shipped binary.
