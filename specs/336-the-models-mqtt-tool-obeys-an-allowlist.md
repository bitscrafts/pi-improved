# Spec: The model's mqtt tool obeys a device allowlist, and the operator's CLI keeps full power

## Overview

`pii mqtt` was added on 2026-11-02 (commit `c78ae56`) as the operator's door to the house, and the same tool was
given to the model. Opus read it the same night:

> `crates/pii-tools/src/mqtt.rs:209` is `fn call(&self, _ctx: &Ctx, …)`: the capability context is ignored. `op=pub`
> publishes **any topic, with `retain`** (`:249-252`). Whatever agent holds the tool can write retained messages to
> `tasmota/discovery/#` or switch any device, with no topic scope and no approval.

That is true, and it is worse than an omission: the input surface is a Discord forum and the model behind it is a
3B local model, so an injected line in a message could set `retain` on a discovery topic and make a device
unreachable until a human notices.

**The decision this implements** is the operator's pending one, in its conservative direction: **the model's tool
may only `ls`, `sub` and `send` to an allowlisted device, with no raw `pub` and no `retain`.** The CLI
(`pii mqtt pub …`) keeps every verb, because the person running it is the one who owns the house. If the operator
later wants raw publish for the model, that is a new spec with its own gate — not a default.

**Project**: /projects/pi-improved
**Phase**: production hardening (Opus audit, operator decision 3)

## Requirements

**R1 — the tool consults its context.** `MqttTool::call` stops ignoring `_ctx` and asks the capability set for
`mqtt.read` before `ls`/`sub` and `mqtt.write` before `send`. A missing capability is a refusal naming the
capability, in the same shape the other tools use.

**R2 — the device set is an allowlist, and it is configuration.** A device may be `send`-targeted only when its
`base` (for example `tasmota_6DE85C`) is named in the `[[mqtt]]` block's new field `allow_devices = ["…"]`.
Absent or empty means **no device is writable**: the tool answers with the list of known names and the refusal.
`ls` and `sub` remain available (they are read-only), and `sub` is restricted to the configured `topics` when that
list is non-empty.

**R3 — no raw `pub`, no `retain`, from the model.** `op = "pub"` is refused by the tool with
`mqtt: raw publish is not available to the model; use `send` with an allowlisted device`. `send` never sets
`retain`. This must be true of the *tool*, not of the CLI: `pii mqtt pub --retain` keeps working.

**R4 — the refusal is testable without a broker.** `MqttOps` already exists as a seam; the tests use the fake and
assert on the refusal text, so nothing in this spec needs a network.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `pub is refused for the model` | `{"op":"pub","topic":"cmnd/x/Power","payload":"ON"}` | `PiiError::Denied` naming raw publish; the fake recorded **zero** publishes |
| `send to an unlisted device is refused and names the allowlist` | `{"op":"send","device":"zb-gw03","payload":"Status 0"}`, allowlist `["SW-FAN-TV"]` | refused; the message names `SW-FAN-TV` |
| `send to a listed device publishes without retain` | `{"op":"send","device":"SW-FAN-TV","payload":"Power ON"}` | one publish: `cmnd/tasmota_6DE85C/Power`, payload `ON`, **retain=false** |
| `control: an empty allowlist makes every device unwritable` | allowlist `[]`, `send SW-FAN-TV` | refused; `ls` still returns 2 devices |
| `ls needs mqtt.read` | `ls` with an empty capability set | refused naming `mqtt.read` |
| `the CLI keeps full power` | `pii mqtt pub tasmota/discovery/x/config '{}' --retain` | accepted (no capability check in `cmd_mqtt.rs`) |

## Exit Criteria

- [ ] `cargo test -q -p pii-tools --test mqtt_tool_scope 2>&1 | grep -qE 'test result: ok\. [5-9] passed'`
- [ ] `grep -q 'allow_devices' crates/pii-channels/src/mqtt/mod.rs`
- [ ] `grep -q 'fn call(&self, ctx: &Ctx' crates/pii-tools/src/mqtt.rs`
- [ ] `! grep -q 'retain: true' crates/pii-tools/src/mqtt.rs`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q 'fresh-gate: PASS'`

## Guardrails

- Do NOT change `MqttChannel::publish` or the `mqtt` graph node: only the **tool** is narrowed.
- Do NOT remove `pub` from the CLI.
- Do NOT default `allow_devices` to every known device: absent means none, because a default that grants is the
  defect this spec exists to remove.
- Do NOT make `ls` require a broker in the tests: the fake is the seam.

## Files

- `specs/336-the-models-mqtt-tool-obeys-an-allowlist.md`
- `crates/pii-abi/src/cap.rs`
- `crates/pii-channels/src/mqtt/config.rs`
- `crates/pii-channels/src/mqtt/mod.rs`
- `crates/pii-cli/src/cmd_mqtt.rs`
- `crates/pii-tools/src/mqtt.rs`
- `crates/pii-tools/tests/mqtt_live.rs`
- `crates/pii-tools/tests/mqtt_tool_scope.rs`
- `xtask/observability-allowlist.txt`
- `crates/pii-tools/src/mqtt/live.rs`

## Closure

Reviewed by the orchestrator on the committed tree, 2026-11-02:

- `cargo test -q -p pii-tools --test mqtt_tool_scope` — 6 passed, 0 failed on `8c36eff`, and the three greps the
  row names hold (`allow_devices` in `mqtt/mod.rs`, `fn call(&self, ctx: &Ctx` in `mqtt.rs`, no `retain: true`).
- `fresh-gate --worktree` **PASS** over HEAD `3df5112`, which carries `8c36eff` (06:20–06:33Z): all eight gate
  steps green, `quality-gate: ALL PASS`, `fresh-gate: PASS`.
- **Mutation, made by the orchestrator.** `may_write` was changed from `if listed {` to `if listed || true {`, so
  the allowlist stops refusing an unlisted device (`bd2cc33c00f3…`): the scope test went **4 passed, 2 failed**,
  rc 101 — `send_to_an_unlisted_device_is_refused_and_names_the_allowlist` and
  `control_an_empty_allowlist_makes_every_device_unwritable` are the two that catch it.
- **Restore.** Copied back from the pre-mutation file, `touch`ed (the `cp -p` false-red lesson), `sha256`
  `00bbec572af5…` before and after, test green again 6/6 rc 0.

So the defect the spec names is fixed at the place it was measured: the tool consults the context, an unlisted
device is refused, and the operator's CLI keeps every verb. What this review does **not** prove is production
behaviour: the `[[mqtt]]` block is still not wired to the tool's construction outside the CLI, which the worker
flagged on its first pass and no criterion here covers.
