# Spec: 121 — the hardware HAL: a contract, a simulated backend, and one real backend behind a feature

**Project**: /projects/pi-improved
**Phase**: **Stage B** of `057` (vision phase 4's second component). The vision says *"hardware HAL"*; this host
**has no hardware**, so the deliverable is the contract plus a backend that is provable here — and a real backend
that is honest about being unverified.

## What exists (measured 2026-10-10)
- `grep -ril 'gpio\|i2c\|spi' crates/*/src` → **nothing**: no HAL of any kind, no pin abstraction, no device node.
- The capability model already attenuates paths and tools (`CapSet`, `Ctx::tool`, `PathScope`) — the HAL must
  attenuate **pins/buses** the same way, not invent a second policy layer.
- The observability contract exists (`pii-observe`) and spec 118's gate now requires every crate to carry a seam.
- Hardware itself lives in other workspace projects (ESP32/EFR32/RP2040); this container is x86_64 Linux with no
  `/dev/gpiochip*`. `ls /dev/gpiochip* /dev/i2c-*` → nothing.

## Requirements
R1. **A new crate `pii-hal`** (one crate per subsystem) holding: the trait surface (GPIO in/out, I2C read/write,
    SPI transfer), the capability types (a pin/bus scope, in the spirit of `PathScope`), and the backend trait.
    It depends on `pii-observe` and carries a seam; its `Health` reports the claimed pins and the last error.
R2. **A SIMULATED backend, first and fully**: in-memory pins with state, an addressable fake I2C device, a
    scriptable input sequence. Deterministic and offline. The simulated backend is the one the tests and the
    end-to-end proof use — because it is the only one this host can hold to account.
R3. **A graph node kind** (`hal`) so a procedure can drive a device: at minimum read a pin, write a pin, do one I2C
    transaction, with the value flowing on the node's ports like any other node.
R4. **Enforcement, not convention** (the user's standing preference):
    a. a pin/bus must be **claimed** before use; a read/write on an unclaimed pin is refused with a `Denied`;
    b. the claim is bounded by a **capability scope** — an operation outside the declared pin set is refused even if
       the pin exists on the device (paste both refusals);
    c. a double claim is refused (or is explicitly idempotent — say which, and test it);
    d. the HAL is **off unless configured**: no config, no HAL backend, and a graph that asks for one fails loudly.
R5. **Observability**: named events for claim / release / read / write / transaction / error, each carrying the pin
    or bus+address, all registered in `ALL` (the lint's completeness rule covers them) plus counters in
    `pii metrics` (operations by kind, refusals). A refusals counter that only ever reads zero is a finding.
R6. **One REAL backend behind a feature** (`hal-gpiod` or equivalent), **compiled in the gate** (so it cannot rot)
    but **not run** here: state in the report and the guide exactly what would verify it (a board, the
    `/dev/gpiochip*` node, a loopback wire) and that nothing on this host did.
R7. **A worked end-to-end proof on the simulated backend**: a graph or SOP that blinks a simulated LED (write high,
    read back the state, write low) and prints/observes the sequence — the pillar's user path, executed, with the
    pasted journal and the events. Plus the negative controls: an unclaimed-pin write, an out-of-scope pin, a
    missing backend.
R8. **Docs**: `_docs/0X-guides/hardware-hal.md` (the traits, the capability scope, the simulated backend, the real
    backend and its unverified status, how to bring up a board) and `001-pii-vision.md`'s phase-4 line updated to
    what actually exists.

## Exit Criteria
- [ ] `cargo test -p pii-hal` exits 0 with the simulated-backend tests, including one test per refusal in R4
- [ ] a mutation per refusal that turns it red (paste each), all restored
- [ ] the end-to-end blink on the simulated backend: paste the journal lines and the emitted events with the pin
- [ ] the real backend compiles under its feature (`cargo clippy -p pii-hal --features hal-gpiod --all-targets
      -- -D warnings` exits 0) and the report states plainly that it was never run on hardware
- [ ] the new names appear in `ALL` and move counters in `pii metrics` (paste)
- [ ] `pii-hal` has a seam and `pii health` reports its state (paste)
- [ ] the guide exists; `001`'s phase-4 line updated (paste it)
- [ ] `bash quality-gate.sh` exits 0 (13 steps); plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, names, config keys, the asserted values, and what was NOT verified (hardware)

## Guardrails
- **Never claim a hardware result that was not run on hardware.** The simulated backend's results must be labelled
  as such everywhere, in the tests, the guide and the report.
- Do not weaken a refusal to make an end-to-end path simpler; the refusals are the safety surface.
- Do not add a heavy runtime dependency for the simulated path; the real backend's dependency must be optional and
  feature-gated, with the crates and versions stated.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.
