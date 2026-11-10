# Archived scripts

One-off exercises, proofs and probes that a spec or a handoff once ran. Nothing in the gate, the queue or a spec's exit criteria calls them.
They are kept, not deleted, and they may need the live stack or the author's paths. Each line is the script's own first comment.

- `checks/digest-occurrence-1200Z.sh`: digest-occurrence-1200Z.sh — observe the 2026-10-19 12:00Z digest occurrence and write the evidence.
- `composed-chain-limits.sh`: Real-agent LIMIT scenarios for composed call chains, through the production devbox daemon (real transport). One scenario per call; each prints PASS/FA
- `composed-chain-real.sh`: The one composition run that had never been executed with a REAL agent: a three-level chain  chain3 -> chain2 -> chain1 -> docs,  whose leaf is a real
- `composed-quota-check.sh`: composed-quota-check.sh — the account's spend limit hit INSIDE a called graph, through a PRIVATE daemon with stand-in agents (nothing spends).
- `composed-real-faults.sh`: Real-agent fault scenarios for the composed `spec` graph, through the production devbox daemon (modules/devbox/daemon.sh), real transport.
- `composed-real-run.sh`: One REAL run of the composed `spec` graph through the PRODUCTION devbox daemon with the real claude on haiku, in a throwaway repo.
- `deploy-lnx-test.sh`: deploy-lnx-test.sh — build here, install on the lnx HOST (which has podman), run the coder graph there, print the reading.
- `exercise-193-196-live.sh`: exercise-193-196-live.sh — two specs, one scratch daemon, observed only.
- `exercise-196-past-the-instant.sh`: exercise-196-past-the-instant.sh — the production shape, replayed: a cancelled run's instant arrives and
- `exercise-2026-10-19.sh`: exercise-2026-10-19.sh — E3 (broken cron graph) and E2 (graphs dir missing) on a SCRATCH daemon.
- `exercise-e10-one-firing.sh`: exercise-e10-one-firing.sh (v3) — E10: a cron set two minutes ahead fires EXACTLY ONE run, then silence.
- `exercise-e2-e3-graphs.sh`: exercise-e2-e3-graphs.sh — E3 (a broken cron graph is refused, loudly and by name) and E2 (a missing graphs
- `exercise-e4-provider-refusal.sh`: exercise-e4-provider-refusal.sh — E4: the model provider is unreachable; pii refuses NAMING it, never hangs.
- `exercise-e5-poll-retry-after.sh`: exercise-e5-poll-retry-after.sh — E5: a 429 must pace the inbound poll by what the provider said.
- `exercise-e6-send-pacing.sh`: exercise-e6-send-pacing.sh — E6: a refused post is paced by the retry_after it carries, and a post still lands.
- `exercise-e9-store-lock.sh`: exercise-e9-store-lock.sh — E9: the store cannot be opened; pii must refuse loudly, not hang.
- `exercise-pii.sh`: exercise-pii.sh (v2) — INJECT faults, then ONLY OBSERVE. Nothing in this script heals anything.
- `experiments/measure-proven-fix.sh`: measure-proven-fix.sh <idx> <fingerprint> <closing-commit> <crate> <paths>
- `experiments/prove-fix-in-7-cycles.sh`: prove-fix-in-7-cycles.sh — the pre-registered experiment the Opus 061 judgement asks for, and the only thing
- `experiments/run-experiment-batch.sh`: run-experiment-batch.sh — run the pre-registered experiment's candidates, one per line of the table below.
- `flake_hunt.py`: Load-stability harness (spec 119): the gate's own test command under a stated CPU load.
- `lane-resume-e2e.sh`: END-TO-END resume of a composed row with a real claude, through the shipped `spec` graph on the production devbox daemon.
- `lane-resume-proof.sh`: REGRESSION CHECK (was the proof) for the lane-resume miss: the shipped spec graph must write its code session where the resume lookup searches. Two re
- `leak_bisect.py`: Spec 116 R1: the F2 leak bisect harness. Re-runnable, fresh store per variant.
- `mvp-verify.sh`: Pii MVP verification suite (2026-09-05).
- `orch-225-227-live-skill-pin.sh`: orch-225-227-live-skill-pin.sh — the ONE live reading that A94 (spec 225 criterion 6) and A95 (spec 227
- `retire-stale-and-rearm.sh`: retire-stale-and-rearm.sh — cancel the stale duplicate digests, restore the cron, and prove the pair.
- `reviews/opus-boss-watch-stall.sh`: The narrow review the operator asked for (2026-10-16): the boss watch's stall under load.
- `reviews/resume-proactivity-review.sh`: resume-proactivity-review.sh — the review that the account window cut off at turn 30. Reads the reset the CLI
- `run-batchA-implement.sh`: run-batchA-implement.sh — dispatch the workhorse lane (pi-workhorse harness) for pii specs 70+71+72.
- `run-rereview-opus.sh`: run-rereview-opus.sh — RE-REVIEW pass of the review -> implement -> re-review pipeline.
- `run-rereview-resume.sh`: run-rereview-resume.sh — finish the 044 re-review: resume the reviewer session and make it
- `run-spec-direct.sh`: run-spec-direct.sh — the pii IMPLEMENTATION lane: pi called DIRECTLY (user rule 2026-08-19,
