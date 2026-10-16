# Operator distance (spec 159)

Taken **2026-10-15** with `pii incidents distance --since 2026-10-01` over `_tmp/autonomy/logs/incidents.jsonl`
(8 incidents). Every number is derived from recorded timestamps; no model.

| Number | Value | Denominator | Target |
|---|---|---|---|
| H_pre | no act before the card in 8/8 | 8/8 computable | H_pre=0 for ≥ half: met |
| L (first evidence → opened) | median 510.5 s (8.5 min); values 0,0,271,452,569,604,938,1211 | 8/8 computable | median ≤ 30 min: met |
| A (operator acts) | median 0; values all 0 | 8/8 computable | ≤ 1: met |

**Caveat, stated plainly:** the store does not yet record operator actions. H_pre and A read the optional
`operator_acts` array (and `card_at`; the card is `opened_at` when absent). No incident carries them, so
H_pre=false and A=0 here mean "none recorded", not "measured as none happened". Until the loop writes
`operator_acts`, only L is a real measurement.

## R and K for the same date (from 063)
- **R = 6.3** hand-made commits per loop closure (57 commits / 9 closures, spec authoring excluded); target 3.4.
- **K = 2** items closed by hand (from `state.json` `last` fields); target 0.
