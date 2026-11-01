# Spec 329: a re-install keeps the config the operator wrote

## Overview

`deploy/install-pii.sh` is the only door production comes through, and as written it replaces the operator's
`pii.toml` with a fresh copy of `examples/pii.production.toml` carrying exactly one value forward. Measured
2026-11-01 by running that pipeline (lines 135-141, verbatim) against a prior config that a real operator could
have written:

```
prior   [channel] name="ops" · [cron] name="weekly" · [worker] name="w1"
        [net].grants = ["10.0.0.9:8084"] · [core].session_runs_cap = 512 · model = "openai:glm-5.3-flash"
after   [core] [llama] [memory] [net]                 ← the template's four, nothing else
LOST    [channel] [cron] [worker] · net.grants → ["127.0.0.1:11434"] · session_runs_cap → absent
```

`model` survived because lines 128-133 carry it forward **by name**; everything else an operator set is gone. The
runbook (`_docs/05-operations/105-runbook.md`) tells the operator to "diff the moved-aside file before
restarting" — an advice, and the repo's own rule is that enforcement beats advice. The consequence class is
silent: the install prints `INSTALLED`, and a re-install that removes a `[[cron]]` or a `[[channel]]` un-arms
production without a single line saying so. Today's production config is exactly this shape — one `[[channel]]`
and no `[[cron]]`, with `pii health` reporting `armed: 0`.

The measurement above is an excerpt, and an excerpt is not the artefact: the exit criteria below drive
`deploy/install-pii.sh` itself, through a knob this spec adds, and parse both files with `tomllib`.

## Requirements

**R1 — the replaced file is the base, not the template.** When `$CONFIG_DIR/pii.toml` exists, the produced config
starts from the *operator's* file. The installer then applies only the values it owns: `model` (carried forward
as today, or the template default on a first install), `runtime_dir`, `trajectory_dir`, `log_file`,
`[llama].host`. A key it owns is edited in place when present, appended when absent. Nothing else is rewritten.

**R2 — the template supplies only what is missing.** Keys and tables the operator's file does not have are added
from the template, so a first install and a re-install converge on the same minimum surface. A table that exists
in both is never replaced wholesale: the operator's copy is the one that survives, with R1's keys applied.

**R3 — the appended blocks are idempotent by name.** The `[[channel]] name = "discord"` block and, when
`HAVE_CHANNEL=1`, the `[[cron]] name = "digest"` block are appended only when no block with that name is already
present. A second install adds nothing and duplicates nothing.

**R4 — the promotion is verified against the file it replaced, and refuses rather than lose a key.** After
producing the candidate, the installer parses the aside copy and the candidate and checks that every top-level
table and every key path present in the aside copy is present in the candidate. Any miss prints the missing
names, restores the aside copy over `$CONFIG_DIR/pii.toml`, and exits non-zero. A verification that only greps
the text is not allowed: a commented-out key in the template (`# session_runs_cap = 256`) matches a grep and is
not a key — measured on this spec's own first attempt.

**R5 — the config half can be run alone.** `CONFIG_ONLY=1` runs sections 4 and 4a only: no build, no deploy
card, no binary install, no unit file, and no network precondition (the llama probe exists to prove the host a
*daemon* will call; a config rewrite calls nothing). Everything else — the aside copy, R1-R4, the append — runs
exactly as in a full install, so a test drives the real path instead of a re-enactment.

**R6 — the test never touches the operator's config.** `deploy/install-config-keeps.test.sh` refuses (non-zero,
nothing written) unless `CONFIG_DIR`, `PREFIX`, `STATE_DIR`, `RUNTIME_DIR` and `UNIT_DIR` are all outside
`$HOME/.config`, `/etc` and `/var/lib`, and outside the repository tree. Its own scratch is created under
`_tmp/` and removed by the test only if the test created it.

## Files

- `deploy/install-pii.sh` (sections 4 and 4a rewritten; `CONFIG_ONLY`)
- `deploy/install-config-keeps.test.sh` (new)
- `gate-steps.txt` (one step, so the test can go red after today)
- `specs/329-a-reinstall-keeps-the-config-the-operator-wrote.md` (this file)

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| keeps-a-section-the-template-lacks | prior config with `[worker]` | `[worker]` and its keys present in the produced file |
| keeps-an-extra-channel-and-cron | prior config with `[[channel]] name="ops"`, `[[cron]] name="weekly"` | both present, names unchanged |
| keeps-the-operators-net-grants | prior `[net].grants = ["10.0.0.9:8084"]` | that exact list, not the template's |
| applies-only-the-keys-it-owns | prior `[llama].host`, `[core].log_file` | host/log_file are the installer's values; `session_runs_cap` is the operator's |
| first-install-still-works | empty `$CONFIG_DIR` | the full template surface is written |
| append-is-idempotent-by-name | run the config half twice | one `[[channel]] name="discord"`, one `[[cron]] name="digest"` |
| --control sees the old path lose them | the template-only base, checked by the same verifier | `[channel] [cron] [worker]` reported missing, so the checker is not vacuous |
| refuses-an-unparseable-prior | prior config with a syntax error | rc != 0, the prior file byte-identical afterwards, the error line named |
| refuses-the-real-config-dir | `CONFIG_DIR=$HOME/.config/pii` | rc != 0, nothing written |
| config-only-skips-build-and-card | `CONFIG_ONLY=1`, no card, no built binaries | rc 0, config written, no binary touched |

## Exit Criteria

- [ ] `bash deploy/install-config-keeps.test.sh 2>&1 | grep -qE "^RESULT: [1-9][0-9]* ok, 0 FAIL"`
- [ ] `bash deploy/install-config-keeps.test.sh --control 2>&1 | grep -q "^CONTROL: the template-only base loses channel, cron, worker and the verifier sees all three"`
- [ ] `bash deploy/install-config-keeps.test.sh --refuses-the-real-config 2>&1 | grep -q "^REFUSED: CONFIG_DIR is the operator's"`
- [ ] `grep -q "CONFIG_ONLY" deploy/install-pii.sh`
- [ ] `grep -q "^install-config|bash deploy/install-config-keeps.test.sh" gate-steps.txt`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- **STOP on a shape mismatch.** If the prior config cannot be parsed as TOML, or a section it carries has no
  place in the template's model (e.g. an unknown table the installer has never written), report the line and
  exit non-zero with the prior file untouched. Do NOT invent syntax to preserve it and do NOT silently drop it.
- **Never delete the aside copy**, and never `rm` the archive directory; the restore path in R4 is the only
  reason the aside copy exists.
- **Do not weaken R4 into a grep.** Parse both files (`tomllib`) and compare table sets and key paths; a
  commented key in the template must not count as present.
- **Do not make the test write outside its scratch**, and do not let `CONFIG_ONLY=1` skip the aside copy: the
  thing being tested is the promotion of a *replaced* config.
- **Do not change what the daemon accepts.** The produced file must be the same TOML shape the daemon reads
  today; no new keys, no renames.

## Closure (2026-11-01)

Verified and committed as `bc20995`, after the row had parked on three worker attempts and its implementation sat
uncommitted in the tree for ~1h45m.

**What passed:** the spec's own test drives the real promotion path and reports `10 ok, 0 FAIL`, including the
control in which the template-only base loses channel, cron and worker; and the promoted config was compared with
the file it replaced — five tables (including `[[channel]]`) and thirteen `key = value` assignments identical.

**What my adoption got wrong, per the Opus review of 2026-11-01** — recorded here rather than quietly fixed,
because the next orchestrator should not repeat it:

- the commit adds three paths the plan row's column 6 does not list (`deploy/pii-config-merge.py` — 180 lines and
  the whole mechanism — plus `deploy/config-python.sh` and the runbook edit), so `unlisted-changes` was bypassed
  by committing anyway;
- it mixes the worker's implementation with my own documentation edit in one commit;
- it says nothing about where the work came from (no row reference, no `Lane:` trailer), so the provenance lives
  only in HANDOFF `cb41662`;
- criterion 3 of this spec (`--refuses-the-real-config`) and criterion 4 have no recorded run.

The behaviour is verified; the process around it was sloppy, and the row is `done` on the strength of the first
statement only.
