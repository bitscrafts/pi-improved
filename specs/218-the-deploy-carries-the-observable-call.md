# Spec: the deploy that carries the coder node, named criteria and the call events

DEPLOY: yes   <!-- the dispatcher runs scripts/autonomy/deploy-row.sh for this row, never a coder -->

## Overview
A deploy is a queue row, not an act performed mid-work. Measured 2026-10-20: an install was started for a revision whose
tree did not contain 215 (criteria as data) or 217 (an agent call is observable) — the installer builds from a git export,
so those changes were excluded by design, and the resulting daemon could not support two of the five readings the version
bar requires. The install was stopped, which was the wrong remedy: work is never stopped, it is QUEUED.

**User rule (verbatim, 2026-10-20):** "you should never stop nothing! only add to the queue".

## Requirements
1. This row exists so the deploy happens once, after 215 and 217 are committed, with both inside the build.
2. Preconditions, all checked before the build starts: `git status --porcelain` is empty; HEAD contains the commits for
   215 and 217; a deploy card exists for the revision being built.
3. The card must name the five readings it intends to support, not just the revision.
4. After install, re-read and record: `pii --version`, `cron blocks in pii.toml`, `runs by status` (the SUSPENDED count).
5. If the schedule is missing after the install (`cron 0`), restore it BEFORE anything else and record that it was missing.

## Exit Criteria
- [ ] `git status --porcelain` is empty before the build
- [ ] `bash deploy/install-pii.sh` prints BUILT FROM EXPORT with HEAD, not "-dirty"
- [ ] `pii --version` names the new revision
- [ ] `grep -c '\[\[cron\]\]' /root/.config/pii/pii.toml` is 3
- [ ] `pii graph lint graphs/coder-demo.toml` does NOT report `unknown kind 'coder'`
- [ ] `pii run ls` still shows exactly one suspended run (the armed digest survived the restart)

## Guardrails
- Do NOT stop a running worker, install or build to make room for this row: queue it and wait.
- Do NOT install from a dirty tree; a worker's WIP must never ship.
- Do NOT claim the deploy succeeded from the installer's exit code alone: the post-install readings above are the verdict.
