# Spec 159: the operator distance is measured (the number that decides whether to keep going)

**Project**: pi-improved · **Milestone**: the closing measurement of the road to production. 061 named it as the missing piece — the operator must be the most distant from the loop, deciding only at the end — and 063 ranked it inside Phase D: *"stop growing the loop" needs a number, not an adjective*.
**Why now**: the lane is closed and the product is deployed, and there is still nothing that says whether it reduces the operator's work. Today's baseline, from 063: **R = 6.3 hand-made commits per loop closure** (57 commits, 9 closures, spec authoring excluded) against a 3.4 target, and **K = 2**. Without this spec those two numbers stay anecdotal.

## Requirement
`pii incidents distance --since <date>` derives, **from recorded timestamps alone and with no model**, three numbers per incident:

- **H_pre** — did the operator act *before* the card existed? True when an operator action carries a timestamp earlier than the incident's card/ask. Target: H_pre = 0 for at least half of incidents.
- **L** — detection latency: from the FIRST evidence event for that incident to the incident being opened. Target: median ≤ 30 minutes.
- **A** — acts per incident: how many separate operator actions the incident required. Target: ≤ 1.

and prints the three distributions, their medians, the counts, and **the denominator** — how many incidents had each field computable at all. An incident missing a timestamp is **named and counted**, never imputed and never silently dropped: a number whose denominator is unrecorded proves nothing.

## TDD contract
| Test | Input | Expected |
|---|---|---|
| a known incident, operator acted late | fixture records with explicit timestamps | H_pre=false, L and A exact to the second |
| an incident nobody touched | fixture records, no operator action | H_pre=true (zero acts before the card), A=0 |
| an incident with no timestamps at all | fixture with those fields absent | reported NOT COMPUTABLE and counted in the denominator, never in the median |
| no incidents in the window | empty store | "no incidents in the window", exit 0, never a division by zero |

## Exit criteria
- [ ] `cargo test -p pii-learn --test incidents_distance` → rc=0
- [ ] `cargo run -q -p pii-cli -- incidents distance --since 2026-10-01` → rc=0 and prints the three distributions plus the denominator
- [ ] the same command run twice prints byte-identical output (deterministic, so a weekly trend line is trustworthy)
- [ ] `_docs/05-operations/operator-distance.md` exists, carries the numbers for the incidents already in the store with the date they were taken, and states R and K for the same date

## Guardrails
- No model, no judge, no inference: every number comes from a recorded timestamp
- Read-only over the store; never rewrite an incident to make a number look better
- A missing field is reported missing and counted; it is NEVER imputed
- Do NOT change the incident-opening rules (150, 156): this spec only reads what they recorded

## Milestone link
`_docs/06-research/061-opus-proactivity-judgement-2026-10-13.md` (the definition) · `_docs/06-research/063-opus-next-phases-2026-10-15.md` Phase D (the threshold) · `_docs/05-operations/production-progression.md`.
