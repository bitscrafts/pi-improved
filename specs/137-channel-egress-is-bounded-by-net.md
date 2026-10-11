# Spec: a channel's egress stays inside what the configuration sanctioned

## Overview

Spec 132 made the daemon register the host a channel declares (`Channel::egress_hosts`) for that channel's
principal, so a channel can dial the host it names and nothing else — that part holds (re-verified 2026-10-11,
`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 Q5). What does **not** hold is the word
*fail-closed*: the host a channel declares is derived from its configured base URL, and for Discord that base
comes from `PII_DISCORD_API_BASE` — **an environment variable**, read in `crates/pii-channels/src/discord.rs`.
So an operator (or a mistakenly exported variable, or a poisoned environment) can point the channel at any
host and the daemon will grant it, because the channel declares it. The configuration never had a say.

The intent of `[net] grants` is that network reach is a decision recorded in the configuration, not a
consequence of whatever a process happens to have in its environment.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (finding 059 F7)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 (F7), §2.3

## Requirements

### Functional
- **R1** A channel's declared egress host is honoured **only if the configuration sanctions it**. Concretely:
  the base URL a channel resolves (whether from the config, a default, or an environment override) must be
  covered by the `[net]` grants for that channel's principal — or by an explicit per-channel hosts list in
  `[[channel]]` — before the daemon registers it.
- **R2** An unsanctioned host is refused **at startup**, with a message that names the channel, the host and
  the config key that would sanction it. Refusing at startup is the point: a channel that cannot dial its
  host must not accept work and then fail per message.
- **R3** The refusal must not break the existing working deployment paths: a channel whose base URL is the
  documented real default (`https://discord.com/api/v10` for Discord) must be granted without the operator
  having to add anything, since the default is part of the shipped configuration's intent. State this
  explicitly in the report — and the shipped default must be a *named constant*, so the sanction is a fact in
  the code, not a string comparison inside the check.
- **R4** A Telegram channel with a custom `base_url`, a Matrix one, and a webhook whose URL is loopback keep
  working exactly as 132 left them (the near-miss controls: they must still take the normal path).
- **R5** Tests: (a) a channel whose environment override points at a host the config does not sanction is
  refused at startup (RED before the change), (b) a channel at the shipped default is accepted, (c) a
  channel whose config entry names the host explicitly is accepted.

### Non-Functional
- No new dependency. clippy `-D warnings` clean.
- The check must be one decision in one place (the daemon's channel build), not repeated per channel kind.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `an_env_override_outside_the_config_is_refused` (new) | `PII_DISCORD_API_BASE=https://evil.example` and no `[net]` grant | `Daemon::build` refuses, naming the channel, the host and the key |
| `the_shipped_default_is_granted` (new, control) | no override | build succeeds, the channel's principal may dial `discord.com:443` |
| `a_config_named_host_is_granted` (new, control) | `[[channel]]` naming the host | build succeeds |
| the existing 132 tests | unchanged | still green (a channel may dial what it declares, and no other) |

## Exit Criteria

- [ ] `cargo test -p pii-channels 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-server --test post_e2e 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] `cargo run -q -p xtask -- net-check 2>&1 | tail -1 | grep -qv 'FAIL'`

## Guardrails

- Do NOT solve this by granting a wildcard (`*.example` / `host:*`) or by granting every channel the hosts of
  every other channel: the point is that the configuration sanctioned this host for this channel.
- Do NOT remove the environment override: it exists for a reason. Bound it, do not delete it.
- Do NOT change the default API base (132 fixed it and a test pins it).
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

(outcome, the three test results, the refusal message, and the near-miss controls — filled in when the item is
closed.)
