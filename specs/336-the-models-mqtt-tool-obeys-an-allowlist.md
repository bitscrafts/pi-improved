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

- [ ] `cargo test -q -p pii-tools --test mqtt_tool_scope 2>&1 | grep -qE "test result: ok\. [5-9] passed"`
- [ ] `grep -q 'allow_devices' crates/pii-channels/src/mqtt/mod.rs` (the field exists in the channel config)
- [ ] `grep -q 'fn call(&self, ctx: &Ctx' crates/pii-tools/src/mqtt.rs` (the underscore is gone)
- [ ] `! grep -q 'retain: true' crates/pii-tools/src/mqtt.rs`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` → `fresh-gate: PASS`

## Guardrails

- Do NOT change `MqttChannel::publish` or the `mqtt` graph node: only the **tool** is narrowed.
- Do NOT remove `pub` from the CLI.
- Do NOT default `allow_devices` to every known device: absent means none, because a default that grants is the
  defect this spec exists to remove.
- Do NOT make `ls` require a broker in the tests: the fake is the seam.
