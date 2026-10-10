# Hardware HAL: GPIO, I2C and SPI behind a claim and a scope (spec 121)

> **Verification status, stated plainly.** Everything below that was *run* was run on the
> **simulated backend** (`backend = "sim"`). The **real backend (`hal-gpiod`) is compiled
> and linted by the gate and has never been run**: this host is x86_64 Linux with no
> `/dev/gpiochip*` and no `/dev/i2c-*`, so nothing on it exercised a real line. No result
> in this guide, in the tests or in the spec-121 report is a hardware result.

The crate is `pii-hal`. A graph drives a device with the `hal` node kind; the daemon
holds one `Hal` built from the `[hal]` config section. With no `[hal]` section the HAL
is **off**, and a graph that asks for it fails the run.

## The traits

- `HalBackend` (what a driver implements): `has_pin/has_i2c/has_spi` (what the device
  has), `claim_pin/release_pin/read_pin/write_pin`, `i2c_transaction(bus, addr, write,
  read_len)`, `spi_transfer(bus, tx)`. It holds **no policy**.
- `Hal` (what a caller holds) wraps a backend with the rules, so no backend can bypass
  them: **claim before use**, **bounded by a `HalScope`**, **double claim refused**.
  Refusals are `PiiError::Denied`, raised before the backend is touched.

## The capability scope

`HalScope` is the pin/bus analogue of `PathScope`: empty permits nothing, and the only
combining operation is `intersect`, which never widens. It names GPIO lines, I2C
`bus:addr` pairs and SPI buses. It is checked on **every** operation, so a pin that
exists on the device but is outside the scope is refused (`... is outside the declared
scope`), as is a write to it without a claim.

| refusal | message | when |
|---|---|---|
| unclaimed | `hal: pin 17 is not claimed` | read/write/transaction/release without a claim |
| out of scope | `hal: pin 18 is outside the declared scope` | any operation on a target not in the scope |
| double claim | `hal: pin 17 is already claimed` | claiming a held target (**refused, not idempotent**) |
| wrong direction | `hal: pin 17 is claimed as input and cannot be written` | write to an input |
| off | `hal: not configured (...)` | a `hal` node with no `[hal]` section |

A pin or address the device lacks is `NotFound`, not `Denied`.

## Config (`[hal]`)

| key | default | meaning |
|---|---|---|
| `backend` | required | `sim` (in-memory board) or `gpiod` (needs a build with `hal-gpiod`) |
| `pins` | `[]` | GPIO lines a graph may claim |
| `i2c` | `[]` | I2C addresses a graph may claim, `"bus:addr"` (`"1:0x48"`, 7-bit) |
| `spi` | `[]` | SPI buses a graph may claim |
| `chip` | `/dev/gpiochip0` | `gpiod` only |

Unknown keys are rejected. An unknown backend, a bad `i2c` entry, or `gpiod` in a build
without the feature fails daemon start.

## The simulated backend

`SimBackend::standard()`: GPIO lines `0..32` (all low); I2C bus 1 with a device at `0x48`
(register 0 = `0x5A`) and one at `0x49` (register 0 = `0x5B`), each a sparse register
file with an auto-incrementing pointer (first written byte = register pointer, the rest
are data; then the read follows from the pointer); SPI buses 0 and 1 as loopback.
`script_input(pin, &[levels])` scripts what a claimed input reads (then the last value
holds). It is deterministic and offline.

## The `hal` node

`op` is one of `claim` (`pin`, `dir`), `release`, `read`, `write` (`pin`, `level`),
`claim_i2c`/`release_i2c`, `i2c` (`bus`, `addr`, `write`?, `read_len`?), `claim_spi`/
`release_spi`, `spi` (`bus`, `tx`). Arguments may be `$.` expressions. The node has one
port, `ok`, carrying `{op, pin, level, high}` / `{op, bus, addr, read}` / `{op, bus, rx}`.
A refusal fails the run (the HAL's `Denied` is the node failure). The graph needs
`caps = ["hw"]`. Worked example: `examples/hal-blink.toml` (claim 17 as output, write
high, read back, write low, read back, release) — executed by
`crates/pii-server/tests/hal_e2e.rs` on the simulated backend.

## Observability

Events: `hal.claim`, `hal.release`, `hal.read`, `hal.write`, `hal.transaction`,
`hal.error` (each with `pin`, or `bus`+`addr`, or `bus`; `hal.error` adds `op`, `reason`,
`error`). `pii metrics`: `pii_hal_ops_total{kind="claim|release|read|write|transaction"}`
and `pii_hal_refusals_total` (printed only when a `[hal]` is configured). `pii health`
reports a `hal` subsystem: `backend`, `claimed_pins`, `claimed_i2c`, `claimed_spi`,
`scope_pins`, `last_error` (`Degraded` while the last failure was a backend fault).

## The real backend (`hal-gpiod`) — UNVERIFIED

`pii-hal` feature `hal-gpiod` pulls the optional dependency **`gpiocdev` 0.8** (pure
Rust, Linux GPIO character device); `pii-server` and `pii-cli` forward the feature. It
implements GPIO only; I2C and SPI return `Unavailable`. The gate compiles and lints it
(`cargo clippy -p pii-hal --features hal-gpiod --all-targets -- -D warnings`) so it
cannot rot. **It has never been run.**

What would verify it, and what nothing on this host did:

1. A board with a GPIO chip (a Raspberry Pi, or any Linux SBC) and a readable
   `/dev/gpiochip0` (user in the `gpio` group, or root).
2. A loopback wire between two lines, e.g. 17 (output) and 18 (input).
3. Build `cargo build -p pii-cli --features hal-gpiod`, config
   `[hal] backend = "gpiod"  pins = [17, 18]  chip = "/dev/gpiochip0"`, and a graph that
   claims 17 `out` and 18 `in`, writes 17 high and reads 18 — it must read `high`; then
   write low and read `low`. Compare against a multimeter or LED on 17.
4. Run the refusal graphs (unclaimed, out of scope) on the same build.

## Bringing up a board

1. Confirm the chip: `ls /dev/gpiochip*`; `gpioinfo` (libgpiod tools) lists the lines.
2. Start on `backend = "sim"` with the pins you intend; run your graph; fix the logic
   with the simulation's deterministic results.
3. Switch to `backend = "gpiod"` with the **same** `pins` scope; keep the scope as small
   as the job needs — it is the only thing between a graph and the other pins.
4. Do step 3 of the previous section before trusting it.
