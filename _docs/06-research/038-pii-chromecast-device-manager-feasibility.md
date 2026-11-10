# pii as a Chromecast / Google Home device manager — feasibility (2026-09-19)

**Status**: analysis for opus review. **No code written.** This is the technical
deep-dive behind **Decision 4** of the Opus package (already in
[`029-coding-harness-design-questions-for-opus.md`](029-coding-harness-design-questions-for-opus.md#5-new-idea-2026-09-17-mid-session-chromecast-protocol-as-a-standalone-daemon--pii-companion-module)
§5, recorded 2026-09-17 from the user's own framing).

**The user's ask, in full**: *"using the 'chromecast protocol' how about a feature
in pii! allowing to manage 'chromecast and google speaker devices' from within
pii!"*

**Their original framing (verbatim, 2026-09-17)**: **"'Chromecast-protocol' could
be a great standalone daemon project as well a pii companion module!
Chromecast-protocol could be the link to all google home devices at home!"**

Register row: **A10** in `<workspace>/docs/01-overview/OPEN-REGISTER.md`.

---

## What works: the protocol's *control* plane is a near-perfect fit for pii

Cast device control is **TLS on TCP port 8009** running **TLS-encrypted
protobuf** frames, plus HTTP for fetching media. Checking that against pii's
existing primitives:

| need | pii primitive | verdict |
|---|---|---|
| TLS client | `pii_net::dial::dial_tls` — rustls 0.23 + webpki-roots, already used for `https` | **have it** |
| reach the device by IP | `HostScope` grant + `allow_private: true` (private class) | **have it** |
| byte/connection bounds | `NetGrant` (`max_conns`, `max_bytes_out/in`, `expires_at_ms`) | **have it** |
| fetch media over HTTP | `pii_core::HttpTransport` | **have it** |
| be a graph node | `NodeFactory` in registry + `ports()` + `required_caps()` | **have it** |
| be a tool | `pii-tools` + one shared `pii-proto` method | **have it** |

That is a striking amount of the design already built. The `pii-net` doc comment
even says it is *"the one socket… Synchronous by design"* — a Cast control
connection is exactly a synchronous, bounded, TLS, private-IP socket.

**Media casting is the easy half too**: the receiver plays a **URL**, so pii
never handles media bytes. `dial_tls` to `:8009` to command, and an existing
`pii-http`/`pii-net` listener (the pattern already used for HLS work in the
`chromecast-tv-mirror` project) to serve. **The hard part of "managing
Chromecast devices" is not the casting — it is discovery.**

---

## The decisive blocker: pii cannot do Cast discovery, by construction

Cast discovery is **mDNS/DNS-SD over UDP multicast** — send to `224.0.0.251`
port `5353`. Against pii's network layer, all three required pieces are missing
*and one is forbidden on purpose*:

**1. Multicast is a blocked address class.** `crates/pii-net/src/authorize.rs`:

```rust
AddrClass::CloudMetadata
| AddrClass::Multicast          // <-- unconditional BLOCK
| AddrClass::Unspecified
| AddrClass::Nat64MappedPrivate => {
    return Err(NetDenied::BlockedClass(format!("{c:?}")));
}
```

Note it sits in the arm **with no `if` guard** — unlike `Loopback | Private |
LinkLocal`, which are permitted when `grant.allow_private` is true. There is no
`allow_multicast` field on `NetGrant` (verified: the struct has `connect`,
`allow_private`, `max_conns`, `max_bytes_out/in`, `expires_at_ms` — and nothing
else). **No grant can enable it.** SSRF-hardening blocks multicast, which is the
correct default and exactly what makes discovery impossible.

**2. There is no UDP at all.** `Protocol` has one variant:
`listen.rs:27  pub enum Protocol { Http }`. `net-check` allowlists
`UdpSocket::bind` / `UdpSocket::connect` as *symbols that may only appear in
`pii-net` itself* — but nothing in `pii-net` uses them. So even inside the
sanctioned crate, UDP is unexercised territory.

**3. `net-check` is a hard gate.** `xtask net-check` is *"the load-bearing Phase 3
gate"*; `pii-net` is the only crate allowed to open a socket or resolve a name,
with a short `ALLOWLIST` of sanctioned call sites. A `pii-cast` crate calling
`UdpSocket` would **fail the gate** unless deliberately allowlisted.

### Why this matters more than it looks

This is not "a missing feature." It is a **security boundary doing its job**. The
honest reading:

> Adding multicast means deliberately widening pii's network boundary for one
> capability. That is a real decision with a real cost, not a plumbing detail.

Three ways out, each with a different cost:

| option | how | cost |
|---|---|---|
| **(i) Widen `pii-net`** | add `AddrClass::Multicast` + `allow_multicast` on `NetGrant` (default **false**), UDP in `Protocol` | touches the most security-sensitive crate; needs spec + threat-model review |
| **(ii) Cast-specific mDNS inside `pii-net`** | narrow API: `pii_net::cast::discover()` — multicast confined to 5353/one call, not a general capability | smallest honest change; still a boundary change |
| **(iii) Skip discovery entirely** | **address devices directly** — the user supplies IP or hostname; `dial_tls` to `:8009` already works | **zero boundary change**, loses "find all devices in the house" |

**Option (iii) is strictly the cheapest and it already works today.** Discovery is
a convenience layer over an address the user can state once. That reframes the
whole feature:

> "Manage Chromecast and Google Home devices from within pii" does **not**
> require multicast at all — if devices are named in config. Multicast is only
> needed for the *zero-configuration* version.

This is the sharpest question to put to opus: is auto-discovery worth touching
the SSRF boundary, or is a device registry (`name → address`) sufficient? For a
home network with a handful of stable devices, the registry is probably the
whole feature.

---

## The second blocker: protobuf is a new dependency class

Cast control speaks **protobuf** over TLS. pii has no protobuf dependency
(verified: nothing for `protobuf`/`prost` in the workspace manifest). And pii has
a **strict `deps-check` gate** plus a 450-line file gate and workspace-wide
`#![forbid(unsafe_code)]`.

- `prost` (+ `prost-build`) would be the idiomatic choice and is pure-Rust-safe.
- The message set is **small and fixed** — a handful of Cast channel messages
  (`CONNECT`, `GET_STATUS`, `LAUNCH`, `LOAD`, `MEDIA_*`). This is not a
  general-purpose protobuf integration; it is a closed schema.
- Alternative worth weighing: hand-rolled encode/decode for that closed set
  (no dependency, more code, must stay under the file gate).

Either way this is a **new dependency class** entering a repo that polices
dependencies, which makes it a policy decision, not just an implementation one.

---

## Shape: which of the three containers?

The original ask already framed this (029 §5): standalone daemon vs pii companion
crate vs `pii-worker`-isolated node. Grounded against pii's actual precedents:

- **Standalone daemon** (`<workspace>/<project>`) — matches
  `chromecast-tv-mirror`, which already has a working cast stack (6,135 LOC,
  spec-09 landed) and no reason to be re-implemented in pii. Hermes and pii both
  talk to it. **But**: it duplicates device management outside pii, which is the
  opposite of the ask ("from within pii").
- **`pii-worker`-isolated node** — the precedent is clean: node kinds live in
  `pii-server`'s registry precisely because they *"need a resource the engine
  owns… that `pii-graph` itself must not know about."* A cast node needs sockets
  and a device registry. This is the same shape as the `memory` node kind, which
  is the most recently built precedent.
- **`pii-cast` companion crate** — new crate + `net-check` allowlist entry; the
  cleanest separation, the largest surface change.

**The `memory` precedent is the strongest guide**: a cast capability is a
*resource the engine owns*, exposed as a node kind, a closed wire method, and one
shared tool — *"one code path, one capability gate, one bound."*

### But: does it earn native code at all?

Hold this against the pii veto recorded for the code-graph work — **if it cannot
make a gate measurably stronger, it does not belong in pii** — and the honest
answer is divided:

- **As a graph node that routes on device state** → **yes, it earns native code.**
  "Announce X on the kitchen speaker; if the cast fails, try the bedroom one"
  is a *branchable* decision — and per the failover constraint,
  *"a graph cannot branch on a reason pii never produces."* A cast node must
  therefore return a **successful `Outcome` carrying `{status, reason, device}`**,
  not an `Err` — so the graph can route on failure. That is a real architectural
  contribution, not a wrapper.
- **As "cast this URL to this device"** → **it is just a tool.** One-shot,
  effect-ful, no judgement to route on. That belongs in a tool, not a node kind.

So the design question is not "daemon vs crate" but **"is there anything to route
on?"** If yes, it is native pii. If no, it is a tool (or a standalone daemon).

---

## Recommendation to opus

1. **Treat discovery and control as separate decisions.** Control is nearly free
   (option iii, today); discovery is the one that touches the SSRF boundary.
   Do not let "manage all devices" silently force a multicast capability.
2. **Ask whether casting has anything to route on.** If not, this is a tool or a
   standalone daemon modelled on `chromecast-tv-mirror` — and pii should stay
   out of it. If yes, it is a node kind following the `memory` precedent.
3. **Do not re-implement the cast stack.** `chromecast-tv-mirror` has a working
   implementation; the open question is integration, not reimplementation.
4. **Decide the protobuf policy explicitly** (`prost` vs hand-rolled closed
   schema) before any spec is written — it interacts with `deps-check`.

---

## Ground truth for independent verification

- `crates/pii-net/src/authorize.rs` — `AddrClass::Multicast` in the
  unconditional-block arm; no `allow_multicast` on `NetGrant`.
- `crates/pii-net/src/grant.rs:100-113` — the complete `NetGrant` field list.
- `crates/pii-net/src/listen.rs:27` — `pub enum Protocol { Http }`.
- `crates/pii-net/src/dial.rs:44-71` — `dial_tls` (rustls 0.23 + webpki-roots).
- `xtask/src/net_check.rs` — `SYMBOLS` / `ALLOWLIST`; `"load-bearing Phase 3 gate"`.
- `crates/pii-server` node-kind precedent and `crates/pii-tools/src/memory.rs` —
  the *"one code path, one capability gate, one bound"* pattern.
- `chromecast-tv-mirror/` — working cast implementation: `src/cast/`
  (`discovery.rs`, `sender.rs`, `session.rs`), `src/mcp/config.rs:104`
  (`CAST_DEVICE`, single device).
- Workspace manifest — no `protobuf`/`prost` dependency present.
