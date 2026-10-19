# Spec: a stream close is not an error (and a success terminal must not carry one)

## Overview
MEASURED 2026-10-18 on the DEPLOYED daemon (4182856bcc1b) against ollama-cloud glm-5.3-flash:

    {"kind":"assistant_delta","text":"GLM"}
    {"kind":"node_finished","node":"agent","port":"err",
     "value":{"error":"unavailable: net: read: peer closed connection without sending TLS close_notify"}}
    {"kind":"run_finished","terminal":{"kind":"success","output":{"error":"unavailable: net: read: ..."}}}

The answer arrived and was TRUNCATED: asked for PONG-GLM, pii received GLM plus a transport error. Two defects in one
line - a peer closing the stream without TLS close_notify is treated as an error instead of end-of-stream, AND the run
finished as success while carrying that error.

**Project**: /projects/pi-improved

## Requirements
1. A peer close without close_notify is END OF STREAM, not failure, when at least one complete frame was received: the
   answer is kept and the run is success with no error payload.
2. A close BEFORE any complete frame remains an error (net: read) - silence is not an answer.
3. terminal.kind = success must not carry an error payload. Either the run failed, or the error is a warning field. A
   success terminal with output.error is the shape that hides failures.
4. The behaviour is identical for every hosted provider (openai, anthropic, gemini): this is the net layer, not a quirk.

## Exit Criteria
- [ ] cargo test -p pii-net --test stream_close_is_eof
- [ ] cargo test -p pii-model --test success_carries_no_error

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT make a truncated answer look complete: keep the frames received, say the stream ended, never invent tokens.
- Do NOT special-case ollama-cloud; the same rule must hold for any provider.
