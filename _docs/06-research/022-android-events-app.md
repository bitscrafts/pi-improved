# Android app — background monitor/actor for Pii events

**Date**: 2026-09-10 · **Author**: research agent · **Status**: RESEARCH ONLY —
feasibility + architecture recommendation, nothing here is implemented or
specced yet.

## 1. Problem

The user wants to monitor Pii runs (and possibly act on them — approve/deny,
maybe start a run) from a phone, in the background, with push-style delivery
of events (approval requests especially). Pii already has the event
vocabulary and a push-subscribe protocol; the question is purely "how does
that reach an Android phone."

No `021-mqtt-broker.md` exists yet in this tree at the time of writing, so
there is no MQTT design to coordinate with — this doc treats MQTT as one of
several transport candidates and flags where it would slot in if/when that
doc is written.

## 2. What Pii already has (read from source)

- **Event vocabulary** (`crates/pii-proto/src/event.rs`): a closed,
  `kind`-tagged enum — `RunStarted`, `NodeStarted`, `NodeFinished`,
  `NodeFailed`, `NodeDead`, `AssistantDelta`, `ToolCall`, `ToolResult`,
  `RunFinished` (with `WireTerminal::{Success,Failure,Suspended}`),
  `RunCancelled`, `RunSuspended`, `RunResumed`, `ApprovalRequested`,
  `ApprovalResolved`, `WorkerDied`, `WorkerRestarted`, `ChannelMessage`,
  `MemoryDegraded`, `RouteDecision`, `RunSpawned`, `Lagged`. Every event that
  matters for a phone notification already exists on the wire; nothing new
  needs to be added to the enum for v1.
- **Subscribe protocol** (`event.subscribe` → `EventSubscribeResult{sub_id}`,
  then unsolicited `EventPush{sub_id, seq, event}` notifications on the same
  connection): filter by `run` or `session`, optional `kinds` allowlist,
  optional `since_seq` replay. `crates/pii-server/src/subscribe.rs` shows the
  server-side fan-out: one `tokio::sync::broadcast` bus, one task per
  subscription, a bounded per-subscription queue (1024) that emits `Lagged`
  rather than silently dropping, dedup by `seq` for journalled events (live-only
  events like `ToolCall`/`AssistantDelta` always forward). Max 32 subscriptions
  per connection.
- **Transport reality**: the protocol is newline-delimited JSON-RPC over *any*
  `AsyncRead + AsyncWrite` (`crates/pii-proto/src/client.rs` — `Client::connect`
  is generic over the byte pair). Today the only concrete transport is a Unix
  domain socket (`crates/pii-server/src/uds.rs`, `crates/pii-uds/src/lib.rs`).
  **A UDS is host-local by construction** — Android cannot reach a UDS on a
  different machine, full stop, regardless of network. This is not a
  permissions problem, it's a kernel-namespace problem: the socket only exists
  in the host's filesystem.
- **Auth model today is entirely local-trust**: `authorize_peer` in
  `crates/pii-server/src/uds.rs` is a *peercred* check — `daemon_uid ==
  peer_uid`, full stop. There is no token, no TLS, no per-client grant. This
  is fine for a same-machine CLI/TUI/web-proxy but has **zero applicability**
  to a phone on a different device — peercred cannot cross a network boundary
  at all, so today's auth story does not extend to remote clients; a new
  authentication layer is mandatory, not optional hardening.
- **`pii-web`** (`crates/pii-web/src/main.rs`, spec 19): an axum server that
  proxies UDS JSON-RPC over loopback HTTP. Explicitly **read-only / viewer-only**
  — "no write RPC is proxied" — and defaults to binding `127.0.0.1` only (a
  test asserts this: `web_defaults_to_loopback`). It exposes `GET
  /api/{status,runs,runs/:id/trace,sessions,sessions/:id,approvals,heals,memory,health}`.
  It does **not** proxy `event.subscribe` (no streaming endpoint), and it does
  not proxy any write method (`approve.resolve`, `run.start`, `run.cancel`,
  `session.prompt`, `session.interrupt`). It is a local-network debugging UI,
  not a remote API.
- **`CapSet`** (`crates/pii-core/src/cap.rs`, recently split into
  Host/Argv/Secret/Path scopes) is a *node execution* sandbox — what a graph
  node running inside a worker is allowed to touch (paths, hosts, argv,
  secrets). It has nothing to do with client/RPC authorization and should not
  be reused for "what can this phone do" — that's a different axis entirely
  (see §5).
- **Write RPCs that matter for an app that acts, not just watches**:
  `approve.resolve` (approve/deny a gate), `run.start`, `run.cancel`,
  `run.resume`, `session.prompt`, `session.interrupt` (all in
  `crates/pii-proto/src/method.rs`). None of these are reachable today except
  over the local UDS.
- **013-dsh-context-slice-c-reference.md**: relevant only as a reminder that
  context/token-usage views should be *projections over the existing journal*,
  not a new store — if the Android app later wants to show context pressure
  per run, it should be a read of a `context`-style projection RPC (once
  Slice C exists), not something the app computes itself from raw events.

## 3. Transport options

### (a) Direct UDS — impossible, not just impractical
Android has no path to a Unix socket living in another machine's filesystem.
Ruled out unconditionally; not worth further evaluation.

### (b) MQTT (Pii publishes/bridges to a broker; phone subscribes)
- **Mechanism**: a small bridge process (could be a `pii-mqtt-bridge` binary,
  or a mode of `pii-server` itself) holds a long-lived `event.subscribe`
  connection to the daemon over UDS (same-host, so peercred auth is trivial)
  and republishes each `EventPush` to `pii/<run_id>/<kind>` (or
  `pii/<instance>/events`) on an MQTT broker. The broker is the only thing the
  phone talks to.
- **Reachability**: the broker can be on the LAN (Mosquitto on the same box or
  a home server) or in the cloud (HiveMQ Cloud, EMQX Cloud, AWS IoT Core,
  self-hosted Mosquitto on a VPS with TLS). Cloud placement solves "phone on
  cellular, off the home network" for free — this is MQTT's actual value prop
  here, not the pub/sub semantics.
- **Background/battery**: this is MQTT's strongest case. MQTT keeps one
  persistent TCP connection with a lightweight keepalive ping (typically
  15–60s); mobile MQTT client libraries are purpose-built to survive Android's
  Doze/App-Standby by pairing with a foreground service (see §4). Battery cost
  is dominated by keeping the radio/socket alive, not by MQTT's framing
  overhead, which is minimal (2-byte header + payload).
- **QoS**: QoS 1 (at-least-once) is the right default for approval-relevant
  events — a `Lagged`-style gap on the Pii side already exists; MQTT retained/
  queued delivery on the broker covers the phone being briefly offline
  (asleep, in a tunnel) without needing Pii to replay via `since_seq` itself.
- **Write path**: the phone can *publish* to `pii/<run_id>/cmd` (approve/deny,
  cancel) and the bridge subscribes to that topic and turns it into an
  `approve.resolve`/`run.cancel` UDS call. This is the natural way to get a
  write path without opening a second, differently-shaped API.
- **Cost of building it**: requires standing up and running a broker
  (self-hosted or a paid cloud tier) plus the bridge process — genuinely new
  infrastructure, not just a new client.

### (c) REST/WebSocket bridge (extend `pii-web`, or a new endpoint)
- `pii-web` already proxies read RPCs over loopback HTTP but is explicitly
  scoped read-only and loopback-only (spec 19 R5 test). Two ways to extend it:
  1. **WebSocket streaming endpoint** (`GET /api/events?run=...` upgraded to
     WS) that proxies `event.subscribe` — straightforward addition, same
     pattern as the existing `call_daemon` helper, just held open instead of
     one-shot.
  2. **Long-poll / SSE** fallback if WS is awkward on mobile networks
     (carrier-grade NAT and some proxies kill idle WS connections faster than
     they kill MQTT keepalives).
- **Reachability**: `pii-web` binds loopback by default *on purpose*
  (untrusted-network safety). Making it reachable from a phone means either
  (i) binding a non-loopback interface and putting a reverse proxy + TLS +
  real auth in front of it (turns a "debug viewer" into a production-grade
  exposed service — a meaningfully bigger security posture change than
  today's code implies), or (ii) tunneling — the phone connects over
  Tailscale/WireGuard into the home LAN and then talks to loopback-bound
  `pii-web` as if local. (ii) is far closer to the existing security model
  (still same-network trust, no new exposed surface) and should be the
  default recommendation over (i).
- **Background/battery**: a WS/SSE connection needs the same Android
  foreground-service treatment as MQTT to survive Doze; no inherent advantage
  over MQTT here, and Android's networking stack tends to kill idle plain
  HTTP/WS connections more aggressively than dedicated MQTT client libraries
  (which implement their own reconnect/backoff and are hardened against Doze
  by convention). REST/WS is however less infrastructure than standing up a
  broker, since it reuses `pii-web`.
- **Write path**: `pii-web` would need genuinely new write endpoints (it
  currently proxies none) — `POST /api/approvals/:id/resolve`, `POST
  /api/runs/:id/cancel`, etc. — plus real per-request auth, since the "viewer,
  loopback-only" security argument no longer holds once writes are exposed.

### (d) Cloud relay (Pii → cloud MQTT/HTTP → phone, reachable from anywhere)
This is transport (b) with the broker deliberately placed off-LAN, so it's
not a fourth option so much as the "phone on cellular" instantiation of (b).
Same trade-offs, plus: the daemon's own outbound connection to the cloud
broker becomes a new dependency on external infrastructure and a new secret
(broker credentials) that must live on the host. Evaluate this only after
(b)/(c) are validated on LAN — do not build cloud-first.

### Transport verdict
For a first version, **local network is fine** (a developer's phone on the
same Wi-Fi as the box running Pii, or via a WireGuard/Tailscale tunnel for
"away from home"). That makes **(c) extend `pii-web` with a WebSocket
event-stream endpoint, tunneled rather than exposed** the lowest-new-infra
path. MQTT (b) is the better long-term choice **if/when** Pii wants
always-reachable, no-tunnel, multi-subscriber delivery (e.g. also notifying a
desktop tray app, a Slack bridge, etc. from the same broker) — at that point
build the bridge once and let the Android app, `pii-web`, and anything else
all subscribe from the broker instead of each re-implementing UDS access.
Revisit this verdict once `021-mqtt-broker.md` exists — if that doc concludes
Pii is standing up MQTT for other reasons (multi-agent fan-out, channels),
the Android app should simply be another subscriber rather than justifying
its own transport.

## 4. Android background execution

Android 8 (API 26)+ background-execution limits are the actual hard part,
independent of transport choice.

| Approach | Background reliability | Battery | Rust code shared with Pii |
|---|---|---|---|
| **Native Kotlin + foreground service + MQTT client (Paho or HiveMQ Android client)** | Best — foreground service with a persistent notification is Android's sanctioned "stay alive" mechanism; mature MQTT libraries handle reconnect/backoff/Doze correctly | Good — MQTT keepalive is cheap; foreground service itself has a small fixed cost (notification, wakelock discipline) | None — Kotlin, no shared code |
| **Flutter (Dart) + `mqtt_client`/`flutter_background_service`** | Good, with caveats — needs a plugin (`flutter_foreground_task` or similar) to run the service reliably across OEM battery-optimization variants (Samsung/Xiaomi/Huawei are notoriously aggressive killers); more moving parts than native | Comparable to native once configured correctly | None — Dart, no shared code |
| **React Native + a JS MQTT client + a foreground-service bridge module** | Weakest of the three for *background* specifically — RN's JS runtime is not designed to run headless reliably; needs a native module for the actual foreground service and MQTT connection, at which point most of the value RN provides (cross-platform UI) doesn't help the hard part | Similar to native once the native module does the real work | None |
| **Rust via `cargo-ndk` + `android-activity` + `rumqttc`** | Unproven for *this specific pattern* (headless foreground service) — `android-activity`/`winit`-style crates target a foreground activity/game-loop lifecycle, not Android's `Service`/`JobIntentService` background model; getting a Rust foreground service running under Android's lifecycle requires hand-rolling JNI glue that the current Rust-Android ecosystem does not provide off the shelf | Should be fine once running — `rumqttc` is a lean async MQTT client | **Highest in theory** (could reuse `pii-proto`'s `Event`/`EventFilter` types directly via a shared crate compiled for `aarch64-linux-android`) — but the *service/notification/lifecycle* layer still has to be native Kotlin regardless, since that's OS API surface, not portable Rust |

**Key point independent of framework**: no framework choice avoids needing a
**foreground service with a persistent notification** — that's the Android
API contract, not a library limitation. Every option above ends up with a
small Kotlin/Java shim owning the `Service` lifecycle; the only question is
how much of the actual client logic (parsing, MQTT/WS, event→notification
mapping) sits on top of that shim in Kotlin, Dart, JS, or Rust.

**Recommendation on framework**: native Kotlin. The realistic code-sharing
win from a Rust client (reusing `pii-proto`'s serde types so the wire schema
can't drift between Pii and the app) is real but narrow — it saves
hand-translating maybe a dozen struct shapes, and Kotlin's `kotlinx.serialization`
+ a `data class` per `Event` variant is not hard to keep in sync by hand,
especially since `Event` is `#[serde(tag = "kind", rename_all = "snake_case")]`
(a totally standard sealed-class-with-discriminator shape any JSON library
handles natively). That saving does not offset the cost of pioneering
foreground-service lifecycle handling in a Rust/JNI stack that isn't proven
for this use case. Flutter is a reasonable second choice if the team wants
iOS parity later; React Native is the weakest fit given the JS-runtime/
background tension.

## 5. Event → notification mapping

| Event | Action | Rationale |
|---|---|---|
| `ApprovalRequested` | **High-priority push notification**, actionable (inline Approve/Deny buttons via `NotificationCompat.Action`, or deep-link into the app) | This is the one event with a real deadline — the run is blocked until a human answers. This is the whole reason to build the app. |
| `RunFinished { terminal: Failure }` | Push notification | User likely isn't watching; failure needs attention. |
| `WorkerDied` | Push notification (or a batched "N workers restarted" if `WorkerRestarted` follows quickly — avoid double-buzzing for a routine respawn) | Infra-level signal; only worth surfacing if `WorkerRestarted` does *not* follow within a short window, i.e. actually stuck. |
| `MemoryDegraded` | Low-priority/silent notification (badge only, no sound) | Degraded-but-running; worth knowing, not worth interrupting for. |
| `RunFinished { terminal: Success }` | Silent — update in-app run list, no push | Expected outcome; pushing for every success trains the user to ignore notifications. |
| `NodeStarted`/`NodeFinished`/`NodeFailed`/`NodeDead`/`RouteDecision`/`RunSpawned` | Silent background sync only (update the in-app run-detail timeline if that run is currently open) | Too fine-grained for a notification; this is "watch a live run" material, not "wake me up" material. |
| `AssistantDelta`/`ToolCall`/`ToolResult` | Silent, only relevant if the user has a run's detail screen open (live-tail) | High frequency; would flood notifications if pushed. |
| `RunSuspended`/`RunResumed` | Silent unless `RunSuspended`'s `reason` indicates a gate (overlaps with `ApprovalRequested` in practice) | Avoid double-notifying the same underlying pause. |
| `ChannelMessage`/`RunStarted` | Silent | Informational; shows up in the run list next sync. |
| `Lagged` | Not user-facing — trigger an app-internal resync (`since_seq` replay or fresh `event.subscribe`), never a notification | This is a transport-integrity signal, not a Pii-domain event. |

**Rendering**: use Android notification channels so the user can tune this
per-category (Approvals / Failures / Infra) without the app needing its own
settings UI for it — this is standard Android practice and avoids
reinventing notification-preference plumbing.

## 6. Emitting events (write path) and auth

Yes, the app can and should send commands — approve/deny is the headline
feature — but this requires solving authentication for a genuinely remote,
untrusted-by-default client, which nothing in the current codebase does.

- **`approve.resolve`** is the critical write RPC: the phone is the approver.
  `run.cancel`/`run.resume`/`session.interrupt` are secondary (nice for "stop
  a runaway run from the couch"). `run.start`/`session.prompt` (starting new
  work from the phone) is a v2 concern — higher risk, lower urgency than
  reacting to an approval.
- **Today's auth (peercred, same-uid) cannot extend to a phone** — this needs
  restating because it's tempting to assume "just add a token" is a small
  patch. It's a new trust boundary: previously every writer was implicitly
  "a process running as the daemon's own user on the same kernel," which is
  why peercred was sufficient. A phone is an unprivileged, physically
  separate, sometimes-untrusted-network client. The minimum viable design:
  1. A long-lived **pairing token** minted by an explicit local action (e.g.
     `pii pair --for android` prints a QR code / one-time code), stored only
     on the phone (Android Keystore) and hashed at rest on the daemon side —
     mirroring how SSH/Tailscale-style device pairing works, not a shared
     static password.
  2. The bridge (MQTT bridge or `pii-web` WS endpoint) checks that token on
     every write RPC and maps it to a **capability scope** — e.g. "may
     `approve.resolve` for runs under session X" — rather than blanket write
     access equivalent to the local UDS. This is a new authorization concept;
     `CapSet` (§2) is the wrong tool since it governs node execution
     sandboxing, not client RPC permissions — reusing it would conflate two
     unrelated trust domains.
  3. Revocation: the pairing token must be revocable (lost/stolen phone) —
     a token list keyed by device, not a single shared secret.
  4. Transport-level: TLS is non-negotiable once traffic leaves loopback/UDS
     (MQTT → `mqtts://`, WS → `wss://`) — the token above is worthless over
     plaintext.
- **Do not reuse `pii-web`'s current "viewer-only, loopback-only" security
  argument for the write path** — that argument (spec 19 R5) is exactly why
  today's `pii-web` is safe to run with zero auth. The moment a write RPC and
  non-loopback reachability both exist, that argument no longer holds and a
  real auth layer (above) is mandatory, not optional hardening.

## 7. Recommendation

1. **Transport**: start with **(c) — extend `pii-web` with a WebSocket
   `event.subscribe` proxy**, reached over a WireGuard/Tailscale tunnel into
   the LAN rather than exposed publicly. This reuses existing infrastructure,
   keeps the security posture close to today's (still same-network trust, no
   new exposed service) and needs no new broker. Revisit MQTT (b) as the
   long-term transport if/when `021-mqtt-broker.md` lands and Pii is standing
   up a broker for other reasons (multi-subscriber fan-out to more than just
   this app) — at that point the Android app becomes just another MQTT
   subscriber and the WS proxy can be deprecated or kept as a fallback.
2. **App framework**: **native Kotlin**, foreground service + persistent
   notification (mandatory regardless of transport), OkHttp/Ktor WebSocket
   client (or Paho/HiveMQ if MQTT is chosen instead). Skip Flutter/RN/Rust-NDK
   for v1 — none meaningfully reduce the hard part (Android's background
   lifecycle), and Kotlin keeps the surface small for a single-purpose app.
3. **Notification mapping**: per §5 — `ApprovalRequested` and failure/`WorkerDied`
   get real pushes with notification channels; everything else is silent
   background sync feeding an in-app run list/timeline.
4. **Write path**: build `approve.resolve` first (the actual reason to carry
   a phone around for this), behind a new pairing-token + capability-scope
   auth layer on the bridge — this is new design work, not a checkbox, and
   should get its own spec before implementation (explicit pairing flow,
   token storage, revocation, and exactly which write RPCs the phone may
   call). Do not ship a write path without it — the alternative is a bearer
   token with blanket daemon access reachable from a phone, which is a much
   larger blast radius than today's same-uid-only UDS.
5. **Sequencing**: (1) read-only WS viewer + notifications for
   `ApprovalRequested`/failures (no write path, no new auth — this alone
   delivers most of the "monitor from my phone" value and de-risks the
   Android background-service part first), then (2) the pairing/auth spec,
   then (3) `approve.resolve` from the phone. Do not build the write path and
   the background-service plumbing in the same pass — they have unrelated
   failure modes and should be validated independently.

## 8. Open questions for whoever specs this

- Does the tunnel requirement (WireGuard/Tailscale) conflict with the user's
  actual usage pattern (e.g. wanting zero-setup cellular access without a VPN
  app running)? If yes, that pushes the transport decision toward MQTT-with-
  cloud-broker sooner than §7 assumes.
- Is there already a `021-mqtt-broker.md` in flight elsewhere (this doc found
  none in `_docs/06-research/` as of 2026-09-10) — if Pii's MQTT decision is
  imminent for unrelated reasons (channels, multi-agent fan-out), transport
  choice for this app should wait for it rather than committing to the WS
  path first.
- Multi-instance: does one phone need to watch more than one Pii instance/host
  (e.g. a home box and a work box)? That changes the pairing-token design from
  "one token" to "one token per instance," and affects whether MQTT topics
  need an instance-id prefix (`pii/<instance>/<run_id>/...`).
