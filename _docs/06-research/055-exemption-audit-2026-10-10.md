# 055 — Audit of the four observability exemptions (2026-10-10)

**Question (owner):** *"research on the exemptions"* — the lint lets four crates sit outside the
observability contract (`xtask/observability-exemptions.txt`: `pii-abi`, `pii-proto`, `pii-uds`,
`pii-observe`). Is "nothing to observe" TRUE for each of them?

**Answer: two are honest, one is permanent, and two are wrong — because the exemption was granted at CRATE
granularity while the honest unit is the MODULE.** The wrong two are the ones a user hits first.

## What was measured (HEAD `555fec1`, `_docs` read + source read)

| crate | rust lines | does it OWN runtime behaviour? (I/O, sockets, timers, processes) | verdict |
|---|---|---|---|
| `pii-observe` | — | it IS the contract crate; it cannot depend on itself | **permanent, correct** |
| `pii-abi` | 1,387 | **no.** 9 modules of types, traits, enums, bitflags, plus PURE helpers (`CapKind::parse_kind`, `classify_provider`, `classify`, `is_retryable`, `http::header`) that return `Result`/`bool` to the caller — no I/O, no state, no async | **honest exemption** |
| `pii-uds` | 120 | **yes.** It is not path helpers alone: `pub async fn client_connect` is documented in its own header as *"the ONE `UnixStream::connect` site outside the daemon"*, returning `PiiError::Internal` on failure. `runtime_dir()` is pure; `client_connect` is I/O | **wrong** (narrow, but real: a refused/missing socket produces no event) |
| `pii-proto` | 1,556 | **yes.** Six modules are wire types/consts, but `client.rs` is a real async transport: `connect::<R,W>` (performs `sys.hello`, fails on protocol skew), `call`, `send_line`, `shutdown`, a tokio broadcast receiver, serde framing, `ClientError::Io(…)`. **This is the module that can HANG FOREVER when a peer closes without answering** (spec 109 D1) | **wrong** |

Evidence commands (paste-ready):
```
ls crates/{pii-abi,pii-proto,pii-uds,pii-observe}/src
grep -rn 'tokio::|std::io|UnixStream|TcpStream|parse|timeout|connect' crates/<crate>/src
grep -n 'pub async fn client_connect' crates/pii-uds/src/lib.rs      # line 91, "the ONE UnixStream::connect site"
grep -n 'pub async fn connect' crates/pii-proto/src/client.rs        # line 87, performs sys.hello
grep -n 'pii-observe' crates/pii-proto/Cargo.toml                    # NO match: the transport cannot emit
grep -rn 'ClientError' crates/pii-cli/src crates/pii-tui/src         # NO match: nobody surfaces it either
```

## The thesis

An exemption means *"this crate has nothing to observe"*. That claim is only true of a crate with **no I/O
and no state**. `pii-proto` is ~90 % wire types and ~10 % transport — and the 10 % is where the connect
timeout, the protocol skew and the hang live. **The transport that cannot connect is exactly the surface a
user meets first**, and today it emits nothing and nobody surfaces its error: `ClientError` has no consumer
that reports it, and the crate is forbidden from emitting because it carries a crate-wide exemption.

This is the same defect class the lane has been closing all along (spec 106: *"the operator surfaces must not
lie"*): the exemption made a live transport invisible, and the lint could not tell the difference between a
crate of consts and a crate holding a socket client.

## What the rule must become (mechanical, not judgement)

An exemption entry is **invalid** if any module of that crate imports `tokio`, `std::net`, `std::io` or
`std::process` — because such a module owns failures an operator must be able to see. The lint must:
1. scan every `crates/<name>/src/**/*.rs` for those imports when a crate is exempt;
2. **fail the gate** when a crate-wide exemption covers a module with I/O, naming the crate and the module
   (`crate pii-proto is exempt but pii-proto/src/client.rs imports tokio — a module with I/O cannot be
   covered by a crate-wide exemption`);
3. accept a **module-scoped** entry (`<crate>:<module>  # reason`) for the dormant part, with the reason
   naming what is dormant and why;
4. keep the sanctioned set rule (a new crate still needs justification) and the dead-exemption rule.

**Near-miss control (mandatory):** a crate with only types/consts and no such imports must still be
accepted (that is `pii-abi`), and re-introducing `use tokio::net::UnixStream;` into an exempt crate must turn
the lint RED. Without this control the rule would either be noise or would forbid every exemption.

## What this changes, concretely

- **The sanctioned set shrinks**: `pii-observe` (permanent) + `pii-abi` (no I/O) stay; `pii-uds` and
  `pii-proto` stop being exempt at crate level and emit their transport failures through the contract —
  endpoint, timeout bound, protocol skew — as NAMED events (registered in `names.rs`/`ALL`, so the lint's
  completeness rule covers them).
- **Spec 109 becomes the payoff**: it already bounds the handshake; with R7/R8 added it also makes the
  failure VISIBLE (the handshake error is emitted and named) and tightens the exemption rule with its
  near-miss control.
- **Usable product, not ceremony**: when the daemon is down, the CLI/TUI will report *which endpoint* and
  *which bound* failed, in the log and in the event stream, instead of hanging or printing an unlabelled
  error.
- **Deployment decision recorded (owner, 2026-10-10):** *"no need to use docker right now (it will run
  directly on the lnx server)"* — the deployment path is the **systemd unit on the lnx host**
  (`deploy/pii.service` + `examples/pii.production.toml` + `_docs/05-operations/105-runbook.md`); the
  untested `Dockerfile` is no longer a readiness caveat, and it stays in the tree as a spare.

## Honest limits of this audit

- It is source-read, not runtime-measured: I did not execute the four crates to see which events fire. The
  claim is about what the code CAN own, which is decidable from the imports and the public API.
- `pii-abi`'s pure classifiers decide failover reasons; an operator may still want that *decision* recorded.
  Because the functions are pure and their callers (the model/provider path) already emit, the honest answer
  is "exempt is fine, the caller emits" — but that is a judgement, and it is stated as one.
- The rule in §"What the rule must become" catches I/O by import. A crate could reach a socket through
  another crate without importing tokio itself; the rule closes the common case, not every case, and the
  report of its first run must say so.
