## What this night was for

The audit of 2026-11-02 (`_docs/06-research/opus-status-and-path-20261102.md`) re-scored the milestones and demoted
five of them: **M0** to mechanism-only (85 pauses, 75 resumes in a week, 84 of 85 pauses logged as `human` when they
were the orchestrator's; the loop made 0 of 62 commits since 10-31), **M3 breached** (the 7-cycle experiment did run
on 10-13/14: 9 rows, $3.04, 0 proven, every base run executing 0 tests — and 151 shipped anyway), **M4 stale** (the
installer that actually made production has no deploy card and copies dev stores over production's), **M6 unwired**
(one real incident — 731 poll refusals in under an hour — sits in the production log and nothing watches it), **M5**
identity only (plain `pii health` fails; `pii.toml` carries `/root` paths, so every run since deploy failed its
trajectory write). Its verdict: *what runs on `lnx` is a deployed daemon, not production, and the distance between
the container's gates and the host is the finding.*

It ranked one action first: **a read-only host conformance probe** — five checks over ssh, as a gate row, with a
control. The rows this night seeded were the audit's own recommendations, one per spec:

- `specs/335-the-host-conformance-probe.md` — the five checks a container gate cannot see
- `specs/336-the-models-mqtt-tool-obeys-an-allowlist.md` — the model's tool, scoped; the operator's CLI, unchanged
- `specs/337-the-durable-stores-live-on-disk.md` — off tmpfs, so a restart keeps the cursor
- `specs/338-the-installer-refuses-without-a-card.md` — the installer that made production, made to refuse
- `specs/339-health-answers-from-a-plain-shell.md` — one default socket, computed once
- `specs/340-the-incident-publisher-lives-in-the-repo.md` — a missing record is loud, not silent

Tranche A was 335–338; tranche B was 339–340, held back so the first review could re-rank the queue before the
second half was spent.
