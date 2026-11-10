# Spec 157: the deployed daemon posts to the operator's channel (M5's second half)

**Project**: <repo> · **Milestone**: M5 — the deployment has exactly one acceptance line left: `pii health` reports the channel up with at least one post recorded, and seven posts inside seven days.
**Why now**: the release binary is installed and running (`revision ebdd4369ba84`, verified 2026-10-15 14:52Z), but `"channels": []` — nothing is wired to the operator's Discord, so the second half of M5's acceptance cannot even begin to accumulate.

## Requirement
- the deploy writes the daemon's environment file with the channel credentials **by reference** — `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID` — taken from the operator's existing environment: no value is ever written into the repo, the unit file, `pii.toml` or the store
- with the credentials absent the daemon still starts and reports `channels: []`: a missing credential is a state to report, never a crash (this is the control, and it is the case that will actually happen on a fresh host)
- with them present the daemon starts the channel and records one post; `pii health --json` reports the channel and `channel_posts >= 1`
- the verification post names the deployed revision, so the message that arrives on the phone is itself the identity check

## TDD contract
| Test | Input | Expected |
|---|---|---|
| no credentials | environment unset | daemon starts, `channels: []`, stays up, no post attempted, no panic |
| credentials present | environment set | `channels: ["discord"]`, `channel_posts == 1` once the verification post is recorded |
| secret leak scan | the whole tree + the generated unit + the store | the token value appears nowhere; only the variable name |

## Exit criteria
- [ ] `cargo test -p pii-channels -p pii-server --test channel_wiring` → rc=0
- [ ] `bash scripts/checks/no-secret-in-tree.sh` → rc=0 (scans the tree, the unit and the store for the token value)
- [ ] `PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health --json | grep -c '"channels":\[\]'` → prints `0` after the wiring
- [ ] the verification post's text contains the deployed revision (asserted from the channel store or the audit log, not from the intent)

## Guardrails
- Do NOT print, commit or copy the token: reference the variable, never the value
- Do NOT make the daemon depend on the channel being reachable: a refused post is recorded and reported, not a crash
- Do NOT touch <agent-host-config> — the operator's own configuration is theirs
- Do NOT delete or rewrite the existing store: the verification post is its first entry, not a migration

## Milestone link
`_docs/05-operations/production-progression.md` M5; acceptance is the same shape the whole lane uses: one command whose output carries the number, and the number visible where the operator already looks.
