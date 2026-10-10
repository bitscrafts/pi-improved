# WASM plugins: third-party graph nodes without rebuilding pii (spec 122)

> **What this is, stated plainly.** A plugin is **untrusted code running inside the
> daemon**. The host contains it with limits the *runtime* enforces, and gives it no
> authority at all. That is a security boundary for the *module*. It is **not** a
> guarantee about the host: a bug in wasmtime is a bug in `pii`, and the host has
> **not been fuzzed** or attacked adversarially beyond the cases listed under
> "What was tested". Do not run a module you would not run as a process on
> this machine without understanding that residual risk.

The crate is `pii-wasm`; the only wasm runtime in the tree is **wasmtime 48.0.5**, and
it appears in that crate only (`cargo run -p xtask -- deps-check` fails if another
crate depends on it, and fails on a second version). A graph runs a plugin with the
`wasm` node kind. With no `[wasm]` section the host is **off**, and a `wasm` node fails
the run with `wasm: not configured`.

## Config (`[wasm]`)

| key | default | meaning |
|---|---|---|
| `dir` | (required) | the **only** directory modules load from. Must exist at daemon start |
| `timeout_ms` | `1000` | wall-clock budget per call; a node may only lower it |
| `max_memory_mib` | `16` | memory ceiling per instance |
| `fuel` | `50000000` | fuel a node gets with `fuel = true`; also the most it may ask for |

A relative `dir` is relative to the daemon's working directory. Unknown keys are a
config error.

## The `wasm` node

```toml
[[node]]
id = "wrap"
kind = "wasm"
module = "wrap.wasm"   # path inside [wasm].dir (absolute paths are checked the same way)
entry = "run"          # optional; default "run"
input = "$.payload"    # optional; default: the node's input payload
timeout_ms = 500       # optional; can only lower the host budget
fuel = true            # optional; true = host default ceiling, or a number; absent = no fuel metering
```

The node's `ok` value is the JSON the module returned. There is no `err` port: every
refusal and failure fails the run with a named error. It needs **no capability**
(`caps = []`): a plugin has none to give. Worked example: `examples/wasm-plugin.toml`
with `examples/wasm/wrap.wat`.

## The ABI

A module is a **core wasm module** (not a component) that exports:

| export | type | role |
|---|---|---|
| `memory` | memory | the linear memory the host reads and writes |
| `alloc` | `(len: i32) -> i32` | returns a pointer to `len` bytes the host may write |
| `run` (or `entry`) | `(ptr: i32, len: i32) -> i64` | reads the input JSON at `(ptr, len)`; returns `(out_ptr << 32) \| out_len` |

The host serializes the input to JSON, calls `alloc`, copies the bytes in, calls the
entry, then reads `out_len` bytes at `out_ptr` and parses them as JSON. A pointer or
length outside the module's memory is `wasm: ... lies outside its memory` — never a host
panic. Output that is not valid JSON fails the call (`module returned invalid JSON`)
and writes nothing to the blackboard.

## The four limits (all enforced by the runtime)

| limit | mechanism | default | failure |
|---|---|---|---|
| wall clock | epoch interruption; a host thread ticks the engine every 10 ms and each call's deadline is `timeout / 10 ms + 1` ticks | 1000 ms | `PiiError::Timeout` (`deadline exceeded after Nms`), event `wasm.timeout` |
| fuel | wasmtime fuel metering, **opt-in** per node | 50 000 000 units when `fuel = true` | `BudgetExhausted` (`ran out of fuel`), event `wasm.fuel_exhausted` |
| memory | a `ResourceLimiter`: growth past the cap traps the call; a module *declaring* more initial memory than the cap is refused at instantiation | 16 MiB | `BudgetExhausted` (`exceeded its memory limit`), event `wasm.memory_limit` |
| no ambient authority | the instance is created with **no imports at all**; a module that imports anything is refused before it runs | — | `Denied` (`imports wasi_snapshot_preview1::path_open, but the host provides no imports`), event `wasm.refusal` reason `imports` |

The timeout resolution is the 10 ms tick: a call is interrupted between its budget and
budget + 10 ms. Also bounded: table growth (10 000 elements), one instance, one memory,
the wasm call stack (wasmtime's default), and the module file itself (8 MiB — compilation
cannot be interrupted, so its input is bounded instead).

### Which WASI functions are exposed

**None.** There is no WASI, no preopened directory, no environment, no clock, no random,
no socket, and no host function. A module compiled for `wasm32-wasi` that calls
`fd_write` or `path_open` has an import and is refused. A plugin must be built
freestanding (`wasm32-unknown-unknown`), or have its imports removed.

## What a plugin can and cannot do

- **Can:** compute on its input JSON, within its time, fuel and memory, and return JSON.
- **Cannot:** read or write a file, open a socket, read the environment or the clock, call
  back into `pii`, or persist state between calls (every call is a fresh instance).
- A plugin is loaded only from `[wasm].dir`; `..`, absolute paths and symlinks that
  resolve outside it are refused (`outside the configured dir`).

## Observability

Events (all carry `module` and `entry`): `wasm.load`, `wasm.instantiate`, `wasm.call`,
`wasm.trap` (with `trap` and the wasm `backtrace`), `wasm.timeout`,
`wasm.fuel_exhausted`, `wasm.memory_limit`, `wasm.refusal` (with `reason`: `not_configured`,
`out_of_scope`, `unreadable`, `too_large`, `imports`, `bad_module`, `bad_abi`,
`invalid_output`). `pii metrics` prints `pii_wasm_calls_total{outcome="ok|trap|timeout|fuel|memory|invalid"}`
and `pii_wasm_refusals_total` when `[wasm]` is configured, and nothing otherwise.
`pii health` reports a `wasm` subsystem (runtime version, dir, limits, call and refusal
counts, last error) only when the host is configured. A contained guest failure does not
degrade the subsystem: the host doing its job is not a host fault.

## Building a plugin

Any toolchain that emits a core module with the exports above and no imports:

```sh
# hand-written WAT
wasm-tools parse examples/wasm/wrap.wat -o plugins/wrap.wasm      # or: wat2wasm
# Rust: a cdylib for wasm32-unknown-unknown with #[no_mangle] extern "C" alloc/run
cargo build --release --target wasm32-unknown-unknown
```

Then `[wasm] dir = "plugins"` and a `wasm` node with `module = "wrap.wasm"`. Check the
module has no imports with `wasm-tools print plugins/wrap.wasm | grep '(import'`.

## What it costs

wasmtime 48.0.5 with `runtime`, `cranelift`, `std`, `backtrace` only (no WASI, no
component model, no async, no cache, no pooling allocator). It pulls Cranelift and
the wasmtime support crates into every build of `pii-server` and `pii-cli`: see the
spec-122 report for the measured clean-build time and the release binary size delta.
Modules are compiled on every call (there is no module cache yet), so a hot loop over a
large module pays compilation each time.

## What a WASM sandbox is not

- It protects the **host from the module**. It does not protect the host from a **wasmtime
  bug**: a Cranelift miscompile or a runtime escape would run with the daemon's authority.
  The Landlock/seccomp sandbox that wraps tools does not wrap this host.
- Limits bound **one call**. Many nodes or many concurrent runs each get their own budget;
  there is no global cap on aggregate plugin CPU or memory.
- **Compilation is not interruptible.** A hostile module is bounded by the 8 MiB file cap, not
  by the timeout.
- A timeout stops the guest at its next epoch check; time spent in host code for the call
  (copying in and out) is not metered.

## What was tested — and what was not

Tested, offline, in `crates/pii-wasm/tests/host.rs` and `crates/pii-server/tests/wasm_e2e.rs`
(hand-written WAT compiled at test time): echo round-trip; infinite loop interrupted by the
epoch deadline; fuel exhaustion; unbounded memory growth refused; a module declaring too much
memory refused; a module importing WASI `path_open` refused; a trap reported with module and
backtrace; an out-of-scope path, a `..` path and a symlink escape refused; invalid JSON and a
wild output pointer failing the call; the host serving a good call after hostile ones; a graph
running a plugin end to end; the host off without `[wasm]`.

**Not tested:** fuzzing of any kind; malformed or adversarial module *binaries* beyond one
garbage file; concurrent hostile calls; memory pressure from many instances at once; a
real third-party toolchain (Rust/AssemblyScript/C) producing a plugin; a module that
exhausts the *host's* stack or heap through the JSON path (a 16 MiB output is parsed in
the daemon); behaviour under the release profile's `panic = "abort"`; any wasmtime
version other than 48.0.5.
