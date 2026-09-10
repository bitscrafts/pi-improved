# MQTT for Pii — broker, bridge, or nothing yet

**Date**: 2026-09-10 · **Author**: orchestrator (research) ·
**Status**: RESEARCH / FEASIBILITY — no code, no spec, no crate exists yet.
This is a design study to inform a future spec, not an implementation plan
to execute as-is.

## 0. Summary (read this first)

- **Recommendation: bridge (client) mode first, hand-rolled, no new
  dependency.** Pii should *publish* its existing wire events to an
  external MQTT broker via a tiny hand-written MQTT 3.1.1 CONNECT/PUBLISH
  client running over `pii-net`'s existing egress chokepoint
  (`NetService::connect`). This adds zero new inbound attack surface, zero
  new auth subsystem, and reuses grants/byte-budgets/deadlines unchanged.
- **Do not embed `rumqttd`.** It is a fine crate on its own terms, but
  embedding it means it binds its own sockets internally — that breaks
  `pii-net`'s "the one socket" invariant (F2.1, enforced by
  `cargo xtask net-check`), not because of `unsafe_code` (workspace lints
  don't apply to dependency crates) but because Pii's architecture has
  exactly one crate that is allowed to touch a raw listener/socket, and
  `rumqttd` insists on being that crate itself.
- **Do not use `rumqttc` either, even for the client-only bridge.** It
  dials its own socket via its own `MqttOptions`/`EventLoop`, bypassing
  `pii-net` entirely. A hand-rolled CONNECT+PUBLISH-only client is smaller
  than the glue code needed to make `rumqttc` cooperate with the
  chokepoint, and it is genuinely tiny (MQTT 3.1.1 fixed header +
  remaining-length varint + four packet types).
- **A from-scratch in-process broker (Pii accepting inbound MQTT
  connections) is a real option but a *later*, opt-in phase**, not part of
  this recommendation. If built, it must go through `pii-net`'s existing
  `NetService::listen`/`ListenSpec`/`Protocol` machinery as a new
  `Protocol::Mqtt`, exactly like `Protocol::Http` does today, with its own
  connection-level auth and topic ACL (this is new security surface Pii
  does not have today: an inbound, LAN/Internet-reachable listener with
  its own authn/authz, as opposed to today's UDS-peercred-only control
  plane).
- Existing prior research
  (`_docs/06-research/002-pii-architecture-plan-pass2.md:1860`) already
  earmarked `rumqttd`/`rumqttc` for a *Phase 4 IoT/hardware* scenario
  (Zigbee2MQTT-style device bridging, `pii-hw`/`pii-mcu` workers). That is
  a different problem — bridging *existing MQTT devices* into Pii's graph
  engine — from this brief's problem, which is exposing *Pii's own
  event bus* to external subscribers. The two may eventually share a
  broker, but they should not be conflated: device bridging needs a real
  SUBSCRIBE-capable client or broker; event fan-out needs only PUBLISH.
  Both are explicitly out of scope for the current phase
  (`specs/05-phase3-assistant-gateway*.md` all list "MQTT ... not in this
  phase").

## 1. Context read

- `crates/pii-proto/src/event.rs` — the closed `Event` enum (F1.7): 20
  variants (`RunStarted`, `NodeStarted`, `NodeFinished`, `NodeFailed`,
  `NodeDead`, `AssistantDelta`, `ToolCall`, `ToolResult`, `RunFinished`,
  `RunCancelled`, `RunSuspended`, `RunResumed`, `ApprovalRequested`,
  `ApprovalResolved`, `WorkerDied`, `WorkerRestarted`, `ChannelMessage`,
  `MemoryDegraded`, `RouteDecision`, `RunSpawned`, `Lagged`), plus
  `EventFilter { run, session, kinds, since_seq }` and `EventPush { sub_id,
  seq, event }`. There is **no `Heal` wire event today** — spec 33/34's
  heal-fix work (routed patch model, structural reproduction/validation,
  ledger evidence — see recent commits) is journalled but not yet
  surfaced on the wire vocabulary. The brief's `pii/heal` topic is
  therefore aspirational: it falls out for free once a `Heal*` variant is
  added to `Event` following the same pattern as everything else here.
- `crates/pii-server/src/subscribe.rs` — `event.subscribe` (F2.5):
  resolves a target run (explicit `run` or latest run of a `session`),
  replays the journal for `seq > since_seq` via `bus::reattach`, then
  attaches a live `broadcast::Receiver<SeqEvent>` for the rest, deduping
  by `seq`. A per-subscription bounded `mpsc` queue (1024) emits `Lagged`
  instead of dropping silently. Filtering is **two independent
  predicates** applied by the server: `event_matches_run` (run-scoped vs.
  process-scoped events) and `event_matches_kinds` (an allowlist of
  `kind` strings, empty = all). This two-axis filter (run × kind) is the
  thing that does not translate cleanly to MQTT's single-string
  topic-wildcard model — see §2.
- `crates/pii-server/src/bus.rs` — `Bus` wraps one
  `tokio::sync::broadcast::channel::<SeqEvent>(4096)`. `BridgeObserver`
  maps journalled `RunEvent`s to wire `Event`s and pushes them in; some
  events (`ApprovalResolved`, `WorkerDied/Restarted`, `ChannelMessage`,
  `MemoryDegraded`, `ToolCall/ToolResult`, `AssistantDelta`) are
  **live-only** — never journalled, `seq` is meaningless, always
  forwarded. `reattach()` is the replay+live merge used by
  `event.subscribe`; nothing else in the daemon currently calls it.
- `crates/pii-net/` — the egress-only chokepoint today:
  - `service.rs`: `NetService::connect(ctx, target, key, timeout_ms,
    scheme) -> Conn` does resolve-once → authorize (grant + address-class
    checks) → dial → (optional TLS) in one call; `Conn::write_all`/`read`
    charge the grant's byte budgets. This is the exact shape a hand-rolled
    MQTT client needs: dial once, write the CONNECT packet, then
    `write_all` each PUBLISH frame.
  - `grant.rs`/`authorize.rs`: `NetGrant { connect: HostScope,
    allow_private, max_conns, max_bytes_out, max_bytes_in, expires_at_ms
    }`, keyed by `(principal, key)`. An MQTT bridge would request a grant
    under a key like `"mqtt:<host>"`, identical in shape to today's
    `"http:<host>"` keys — no new grant machinery needed.
  - `listen.rs`: `NetService::listen(ctx, ListenSpec::Tcp { addr },
    Protocol, handler: Arc<dyn HttpHandler>)` — one accept thread + one
    thread per connection, hard-capped, `ctx.net()?` gated. Currently
    `Protocol` has exactly one variant, `Http`, and the accept loop is
    HTTP-request-shaped (`HttpHandler::handle`/`handle_stream`). A future
    MQTT *listener* would need either a new `Protocol::Mqtt` variant with
    its own frame-shaped accept/serve loop (mirroring `serve_conn` but
    speaking MQTT framing instead of HTTP), or a lower-level "raw byte
    stream" handler trait alongside `HttpHandler`. Nothing here blocks
    that — it is additive — but it is undone work, not a small patch.
  - `lib.rs` header: *"the only crate in the workspace that opens a
    socket or resolves a name — enforced by `cargo xtask net-check`"*.
    This is the load-bearing constraint for the broker-vs-bridge decision
    in §3.
- `crates/pii-server/src/daemon/mod.rs` — `Daemon::build` constructs one
  `pii_net::NetService`, the worker pool, channels, cron, the timer
  wheel, and a `DaemonContext`; `spawn_background` launches the scheduler
  loop, one task per channel (`router::spawn_all`), and
  `spawn_reactive_loop` (a `tokio::spawn` that does `engine.bus.subscribe()`
  and reacts to every event — the same bus `event.subscribe` reads). An
  MQTT bridge is naturally a **sibling** to `spawn_reactive_loop`: another
  background task that subscribes to `engine.bus` and republishes,
  configured from a new `[mqtt]` section next to `[net]`/`[channel]`/
  `[worker]` in `daemon/config.rs`.
- `_docs/06-research/006-pii-graph-engine.md` — confirms the journal is
  the source of truth and replay-by-seq is a first-class operation
  (`reattach`), which is why §2 below treats MQTT as a **transport**, not
  a second source of truth: any durability/replay guarantee still comes
  from the journal + `event.subscribe`, never from the MQTT broker's own
  retained-message store.

## 2. Topic mapping

### 2.1 Scheme

Two independent filter axes in `EventFilter` (`run`, `kinds`) map best
onto topic segments in that order, so a single-level wildcard filter can
select on either axis independently:

```
pii/run/<run_id>/<kind>[/<sub_id>]
pii/approval                              # process-scoped, not run-scoped
pii/worker/<worker_name>
pii/channel/<channel_name>
pii/memory
pii/heal/<run_id>                         # once a Heal wire event exists
```

Concretely, per `Event` variant:

| Event variant | Topic | QoS | Retain |
|---|---|---|---|
| `RunStarted` | `pii/run/<run_id>/status` | 1 | yes (latest status) |
| `RunFinished` | `pii/run/<run_id>/status` | 1 | yes |
| `RunCancelled` | `pii/run/<run_id>/status` | 1 | yes |
| `RunSuspended` | `pii/run/<run_id>/status` | 1 | yes |
| `RunResumed` | `pii/run/<run_id>/status` | 1 | yes |
| `NodeStarted` | `pii/run/<run_id>/node/<node>` | 0 | no |
| `NodeFinished` | `pii/run/<run_id>/node/<node>` | 0 | no |
| `NodeFailed` | `pii/run/<run_id>/node/<node>` | 0 | no |
| `NodeDead` | `pii/run/<run_id>/node/<node>` | 0 | no |
| `AssistantDelta` | `pii/run/<run_id>/assistant/<session_id>` | 0 | no |
| `ToolCall` / `ToolResult` | `pii/run/<run_id>/tool/<tool>` | 0 | no |
| `RouteDecision` | `pii/run/<run_id>/route` | 0 | no |
| `RunSpawned` | `pii/run/<run_id>/spawn` | 1 | no |
| `ApprovalRequested` / `ApprovalResolved` | `pii/approval` | 1 | latest-pending, yes |
| `WorkerDied` / `WorkerRestarted` | `pii/worker/<worker>` | 1 | yes |
| `ChannelMessage` | `pii/channel/<channel>` | 0 | no |
| `MemoryDegraded` | `pii/memory` | 0 | no |
| `Lagged` | not published to MQTT (internal to the JSON-RPC bounded-queue path; MQTT's own broker/transport loss is the analogous signal there) | — | — |

Rationale for `<run_id>` before `<kind>`: `EventFilter.run` is the more
common real-world subscription ("show me this one run") and MQTT's `#`
multi-level wildcard makes `pii/run/<run_id>/#` a clean "everything about
this run" filter. Filtering by kind *across* runs is still possible with
`pii/run/+/node/+` (single-level `+` wildcard), just not as the primary
axis. `EventFilter.session` has no MQTT equivalent — sessions resolve to
"the latest run of a session" server-side in `subscribe.rs`; an MQTT
client that wants session-following would need Pii to publish a
`pii/session/<session_id>/run` pointer topic (retained, updated on each
new run) that tells the client which `run_id` topic to subscribe to next.
This is a small addition, not a blocker, but is not in the v1 sketch
below.

### 2.2 What MQTT cannot do that `event.subscribe` can: `since_seq` replay

`EventFilter.since_seq` plus `bus::reattach` gives JSON-RPC subscribers
exact, gap-free backfill from the journal before going live. Vanilla
MQTT has no concept of "replay everything since sequence N" — retained
messages give you exactly one cached message per topic (good for "what's
the current run status" on reconnect, useless for "give me every
NodeFinished I missed while disconnected"). Three ways to handle this,
in order of preference:

1. **Don't try to match it.** Treat MQTT as a best-effort live tap;
   clients that need gap-free history use the existing JSON-RPC
   `event.subscribe` with `since_seq` (already correct, already tested,
   already has `Lagged` reporting). This is consistent with treating MQTT
   as a *transport*, not a second source of truth.
2. **Retained status topics narrow the gap for the common case.** A
   dashboard that only cares about "is this run still going and how did
   it end" is fully served by the retained `pii/run/<run_id>/status`
   message — it gets the latest state instantly on subscribe, no replay
   needed.
3. **(Only if a from-scratch broker is built, §3.2) replay-on-SUBSCRIBE.**
   Because a hand-rolled broker's SUBSCRIBE handler is Pii's own code, it
   *could* call the same `bus::reattach` used by `event.subscribe` and
   publish a synthetic backlog burst to the new subscriber before
   switching it to live delivery — giving MQTT clients the same
   replay-then-live guarantee JSON-RPC clients get today. This is
   impossible with an off-the-shelf broker (`rumqttd`) or an external
   bridge target (mosquitto/EMQX): neither knows about Pii's journal, so
   neither can replay it. This is the strongest technical argument for a
   from-scratch broker *if and when* MQTT clients need journal-grade
   replay — but it is not needed for v1's "publish, best-effort" bridge.

### 2.3 QoS policy

- **QoS 0** (fire-and-forget) for everything high-frequency or already
  covered by the journal as source of truth: `NodeStarted/Finished/
  Failed/Dead`, `AssistantDelta`, `ToolCall/ToolResult`, `RouteDecision`,
  `ChannelMessage`, `MemoryDegraded`. Loss is acceptable — the journal
  and `event.subscribe` remain the durable, gap-free channel.
- **QoS 1** (at-least-once) for events where a dropped delivery has real
  consequences for a human/dashboard: `RunStarted/Finished/Cancelled/
  Suspended/Resumed` (status), `ApprovalRequested/Resolved` (someone may
  be waiting to approve), `WorkerDied/Restarted` (ops-relevant),
  `RunSpawned`.
- **No QoS 2.** Exactly-once needs a 4-packet handshake and broker-side
  dedup state; every Pii event already carries an idempotent `seq` (or is
  explicitly live-only), so a QoS-1 client that wants dedup can key on
  `seq` itself. Not worth the protocol complexity for v1.

### 2.4 Payload shape

Publish the same JSON the JSON-RPC path already produces — reuse
`serde_json::to_vec(&event)` on the wire `Event` (already `Serialize`,
already `kind`-tagged `snake_case`) plus the `seq` alongside it, e.g.:

```json
{"seq": 42, "event": {"kind": "node_finished", "run_id": "r1", "node": "a", "port": "ok", "value": 1}}
```

No new payload schema to design or version — the wire vocabulary is
already closed (F1.7) and already has a JSON Schema (`schemars`) derive
on it.

## 3. Broker vs. bridge

### 3.1 Why bridge (client) mode wins for v1

Pii's security model today is deliberately narrow on the *inbound* side:
the control plane is a UDS with peercred auth; the only inbound TCP
listener (`Protocol::Http`) is opt-in and request/response shaped, gated
by `ctx.net()`. There is no existing story for "accept an arbitrary
inbound network client, authenticate it, and enforce a per-client topic
ACL" — that is a new authz subsystem, not a small extension of grants
(`NetGrant` is an *egress* concept: which hosts *Pii* may dial out to,
not which topics an inbound peer may read).

Bridging avoids all of that: Pii only ever calls *out*, through the exact
same `NetService::connect` → grant → resolve-once-pin → dial → byte-budget
path every other egress call already uses. The broker (mosquitto, EMQX,
a cloud broker) is the trust boundary and already has mature
authn/authz/TLS/ACL machinery — reusing it is strictly less work and less
new attack surface than building an inbound listener plus ACL logic
inside Pii.

This also answers the Android-app framing directly: the phone does not
need to speak Pii's UDS/peercred protocol at all, and Pii does not need
to expose *any* new inbound port to the LAN or Internet. The phone
subscribes to whatever broker Pii is bridging to (a LAN mosquitto if
both are on the same network, or a cloud broker such as HiveMQ Cloud /
AWS IoT Core if remote access is wanted) using that broker's own
auth — Pii is a publisher, the broker mediates everyone else. This is
also strictly better for the "IoT devices" case in the brief: any
existing Zigbee2MQTT/ESPHome/Tasmota-style deployment already has a
broker running; Pii bridging into it costs nothing extra to deploy.

### 3.2 When broker mode would be worth it

A genuinely broker-less deployment — a single Pii daemon on a home LAN
with no mosquitto anywhere and a user who does not want to run one — is
a legitimate future ask, and is exactly the case prior research already
flagged for a Phase 4 IoT slice
(`002-pii-architecture-plan-pass2.md:1860`). If/when that lands:

- It must be the **from-scratch minimal broker**, not `rumqttd`, to keep
  `pii-net`'s single-socket invariant intact: add `Protocol::Mqtt` next
  to `Protocol::Http` in `listen.rs`, reuse `NetService::listen`'s
  accept-thread/per-connection-thread model, and give it a frame-level
  handler (parse fixed header + remaining length, dispatch CONNECT/
  SUBSCRIBE/PUBLISH/PINGREQ/DISCONNECT) instead of `HttpHandler`'s
  request/response shape.
- CONNECT-time auth should mirror Pii's existing bias toward local,
  explicit trust: a Pii-issued token/PSK (config-supplied, rotatable),
  never anonymous CONNECT accepted by default, and the daemon config must
  make "I am opening a LAN-reachable listener" an explicit, separately
  documented opt-in (the same posture `ctx.net()` already enforces for
  `Protocol::Http`).
- Per-connection topic ACL: v1 can be coarse (a connection either may
  subscribe to `pii/#` read-only, or is rejected) since Pii-as-broker
  only needs to serve *out* (no client should be allowed to PUBLISH
  *into* Pii and have it treated as a real event — that would let an MQTT
  client forge `RunFinished`/`ApprovalResolved` messages indistinguishable
  from real ones unless the broker keeps its own publish path
  server-authoritative and simply rejects/ignores inbound client
  PUBLISHes for `pii/*` topics).
- This unlocks §2.2's replay-on-SUBSCRIBE trick, which off-the-shelf
  brokers and the bridge mode cannot offer.

Do **not** build this speculatively. It is real, scoped, additive work
gated behind an actual need (a user who won't run mosquitto) — the
bridge in §3.3 covers the dashboards/mobile/IoT cases in the brief
without it.

### 3.3 Recommended architecture (v1)

New crate `crates/pii-mqtt` (small; a module inside `pii-net` would also
work but a crate matches the existing one-concern-per-crate layout and
keeps `pii-net` itself free of MQTT-specific framing):

```
crates/pii-mqtt/
  src/
    lib.rs
    codec.rs   # MQTT 3.1.1 fixed header + remaining-length varint;
               # CONNECT/CONNACK/PUBLISH/PINGREQ/PINGRESP/DISCONNECT only
               # (publish-only client — no SUBSCRIBE/SUBACK needed for v1)
    client.rs  # MqttBridge: owns a pii_net::Conn, does the CONNECT
               # handshake once, PUBLISHes framed events, PINGs on
               # keepalive, reconnects with backoff on write/read error
    topics.rs  # pure fn: &Event -> (topic: String, qos: u8, retain: bool)
```

Dependencies: `pii-abi`, `pii-core`, `pii-net`, `pii-proto`, `serde_json`
— nothing new added to the workspace. No `unsafe`, matches the workspace
`forbid(unsafe_code)` lint trivially since it's ~5 small packet builders
over byte buffers.

Wiring in `crates/pii-server/src/daemon/mod.rs`:

- `daemon/config.rs` gains an optional `[mqtt]` section: `broker` (host:
  port), `client_id`, `topic_prefix` (default `pii`), `kinds` (optional
  allowlist, same shape as `EventFilter.kinds`), and TLS/username/password
  if the target broker needs it.
- `Daemon::build` constructs `MqttBridge` from that config (or skips it
  entirely if absent — this is fully opt-in, mirroring how channels are
  only built when configured).
- `Daemon::spawn_background` gets one more sibling task next to
  `spawn_reactive_loop`: subscribe `engine.bus.subscribe()`, map each
  `SeqEvent` through `pii_mqtt::topics::route(&event)`, skip if filtered
  out by the configured `kinds` allowlist, hand off to
  `MqttBridge::publish`. This never touches `subscribe.rs` or the
  JSON-RPC path — it is a second, independent consumer of the same bus,
  exactly like the reactive loop already is.
- Egress grant: the bridge calls `NetService::connect(ctx, target,
  "mqtt:<host>", timeout_ms, Scheme::Http /* or a new Mqtt scheme if TLS
  differs */)`. This requires the daemon's grant store to have a
  `NetGrant` for key `"mqtt:<broker_host>"` — same shape as today's
  `"http:<host>"` grants, no new grant type.

### 3.4 Minimal implementation sketch

CONNECT packet (client id only, no will, clean session) and one PUBLISH,
using the same varint remaining-length encoding MQTT 3.1.1 specifies:

```rust
// codec.rs — remaining-length varint (MQTT 3.1.1 §2.2.3)
fn encode_remaining_len(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = (n % 128) as u8;
        n /= 128;
        if n > 0 { byte |= 0x80; }
        out.push(byte);
        if n == 0 { break; }
    }
}

fn connect_packet(client_id: &str, keep_alive_secs: u16) -> Vec<u8> {
    let mut var_and_payload = Vec::new();
    // variable header: protocol name "MQTT", level 4, connect flags, keep-alive
    var_and_payload.extend_from_slice(&[0x00, 0x04]);
    var_and_payload.extend_from_slice(b"MQTT");
    var_and_payload.push(0x04);        // protocol level 4 = 3.1.1
    var_and_payload.push(0x02);        // clean session, no will/user/pass
    var_and_payload.extend_from_slice(&keep_alive_secs.to_be_bytes());
    // payload: client id (length-prefixed UTF-8)
    var_and_payload.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    var_and_payload.extend_from_slice(client_id.as_bytes());

    let mut packet = vec![0x10]; // CONNECT fixed header type
    encode_remaining_len(var_and_payload.len(), &mut packet);
    packet.extend_from_slice(&var_and_payload);
    packet
}

fn publish_packet(topic: &str, payload: &[u8], qos: u8, retain: bool) -> Vec<u8> {
    let mut var_and_payload = Vec::new();
    var_and_payload.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    var_and_payload.extend_from_slice(topic.as_bytes());
    // QoS 0 in this sketch: no packet identifier. QoS 1 would insert a
    // 2-byte packet id here and require reading back PUBACK.
    var_and_payload.extend_from_slice(payload);

    let flags = (qos << 1) | u8::from(retain);
    let mut packet = vec![0x30 | flags]; // PUBLISH fixed header type
    encode_remaining_len(var_and_payload.len(), &mut packet);
    packet.extend_from_slice(&var_and_payload);
    packet
}
```

`client.rs` then does, roughly:

```rust
pub struct MqttBridge {
    conn: Mutex<pii_net::Conn>,   // dialed once via NetService::connect
}

impl MqttBridge {
    pub fn connect(net: &pii_net::NetService, ctx: &Ctx, target: &Target,
                    client_id: &str) -> Result<Self, PiiError> {
        let mut conn = net.connect(ctx, target, &format!("mqtt:{}", target.host),
                                     5_000, Scheme::Http)?;
        conn.write_all(&codec::connect_packet(client_id, 60))?;
        // read + validate CONNACK (4 bytes: 0x20 0x02 <flags> <return code>)
        let mut ack = [0u8; 4];
        read_exact(&mut conn, &mut ack)?;
        if ack[3] != 0 { return Err(PiiError::Unavailable("mqtt: connect refused".into())); }
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn publish(&self, topic: &str, payload: &[u8], qos: u8, retain: bool) -> Result<(), PiiError> {
        self.conn.lock().write_all(&codec::publish_packet(topic, payload, qos, retain))
    }
}
```

QoS 1 support (needed for the events in §2.3) adds: a packet-identifier
counter, reading back `PUBACK` (4 bytes: `0x40 0x02 <id_hi> <id_lo>`)
before considering the publish complete, and a resend-on-timeout policy —
all still well within "a few hundred lines," no external MQTT crate
needed.

## 4. Open questions for whoever writes the spec

1. Does the bridge need MQTT 5 (user properties, richer reason codes) for
   any concrete downstream consumer, or is 3.1.1 sufficient? (3.1.1 is
   sufficient for every consumer named in the brief — dashboards, mobile
   apps, generic IoT brokers all speak 3.1.1.)
2. Should `RunSpawned`/`RouteDecision` be exposed over MQTT at all in v1,
   or held back as "journal/JSON-RPC only" until a consumer asks for
   them? (Leaning: include them, they're free — same `topics::route`
   function, no extra wire work.)
3. Where does the `Heal*` wire event (needed for `pii/heal`) get added —
   as new `Event` variants following spec 33/34's heal-fix work, or is
   heal visibility meant to ride on existing `NodeFailed`/`RouteDecision`
   events? This needs an answer in `pii-proto` before `pii/heal` can be
   anything but a placeholder.
4. TLS to the broker: `pii-net`'s `dial_tls` already exists for HTTPS
   (spec 17 R3) — confirm it can be reused unmodified for an MQTT-over-TLS
   (port 8883) dial, since the TLS wrapping happens after `dial()`
   regardless of the payload protocol.
