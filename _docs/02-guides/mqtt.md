# MQTT: the reactive front door (spec 117)

A device can now start a graph, and a graph can answer a device. One `[[mqtt]]` section
is one broker connection; the `mqtt` node kind publishes through it.

- **Subscribe → run.** The connection subscribes to `topics` on every (re)connect. Each
  message starts `graph` with input `{topic, payload, text, channel, chat, user}`.
- **Publish.** An `mqtt` node publishes a topic + payload with QoS and retain written out.

## Config (`[[mqtt]]`)

| key | default | meaning |
|---|---|---|
| `name` | required | connection name; health subsystem is `channel.<name>` |
| `url` | required | `mqtt://host[:port]` (port 1883). `mqtts://` is **refused** |
| `graph` | required | graph started per inbound message |
| `topics` | `[]` | filters to subscribe to (`sensors/#`); empty = publish-only |
| `qos` | `0` | subscription QoS, 0..=2 |
| `client_id` | unique | default `pii-<pid>-<time>-<seq>`, distinct per instance |
| `username_env` / `username_file` | – | broker user from an env var / a 0600 file |
| `password_env` / `password_file` | – | broker password from an env var / a 0600 file |
| `allow_plaintext_credentials` | `false` | see "Plaintext rule" |
| `reconnect_min_ms` / `reconnect_max_ms` | `500` / `30000` | backoff base and ceiling |
| `queue_cap` | `1024` | inbound queue bound |
| `poll_interval_ms` | `100` | how often the router drains the queue |

There is no `username =` / `password =` key: credentials are never config values (the
parser rejects unknown keys). They resolve through `pii_channels::secret::resolve`, the
same env-or-0600-file rule as the other channels.

## Payload mapping (inbound)

`topic` = the message topic. `payload` = the bytes parsed as **JSON when valid, else the
text itself** (`{"t":21}` → object; `on` → string `"on"`). `text` always keeps the raw
text. A payload that is not valid UTF-8 is **dropped and counted**
(`mqtt.message.dropped`, reason `payload is not valid UTF-8`); it never panics a run.

## The `mqtt` node (publish)

```toml
[[node]]
id = "pub"
kind = "mqtt"
broker = "plant"        # the [[mqtt]] name
op = "publish"          # the only op; subscribing is the [[mqtt]] section
topic = "out/echo"      # literal or "$." expression
qos = 1                 # REQUIRED, 0..=2 — never implicit
retain = true           # REQUIRED — never implicit
# payload = "$.input.payload"   # optional; default = the predecessor's output
```

The default payload is `$.payload`, i.e. the predecessor's output. A string is sent as its
bytes; any other JSON value as its JSON text. Ports: `ok` `{topic, bytes}`, `err`
`{error}`. The node declares the `net` capability, so the graph must list `caps = ["net"]`.
`Channel::send` (the reply path of a channel run) is deliberately a no-op for MQTT —
replying onto the triggering topic would be a feedback loop; publish with an `mqtt` node.

## Failure behaviour

- Broker unreachable or connection lost → health `Degraded` with the reason (`broker
  host:port unreachable: … (attempt N, retry in M ms)`), event `mqtt.connection.lost` per
  failure, then retry after `min(reconnect_min_ms * 2^k, reconnect_max_ms)`:
  500, 1000, 2000 … 30000 ms by default. The driver sleeps between attempts; it cannot
  spin. A successful `CONNACK` resets the backoff and clears `Degraded`.
- Inbound queue full → the message is dropped and counted (`reason: inbound queue full`).
- Publish queue full → `Unavailable` on the node's `err` port, counted as a drop.

## Events and metrics

Events (all registered in `pii_observe::names::ALL`): `mqtt.connected`,
`mqtt.connection.lost`, `mqtt.message.received`, `mqtt.message.dropped`, `mqtt.published`,
`mqtt.plaintext_credentials`. Each carries `channel` and `broker`; message events carry
`topic`. Counters in `pii metrics`: `pii_mqtt_received_total`, `pii_mqtt_dropped_total`,
`pii_mqtt_published_total` (from `sys.status` `counts.mqtt_*`). "Published" means accepted
by the client for sending; at QoS ≥ 1 delivery is the broker's acknowledgement.

## Plaintext rule (TLS is absent)

This build has **no TLS**: `rumqttc` is compiled with `default-features = false`, so
`rustls` stays confined to `pii-net`. Therefore a username/password is **refused** (the
daemon does not start) for any broker host that is not `localhost` or a loopback IP,
unless `allow_plaintext_credentials = true`, which sends them in clear text and emits
`mqtt.plaintext_credentials` once at startup. Hostnames are not resolved for this check:
an unknown name counts as remote. Credentials never appear in events, errors, health or
trajectories (scrubbed from connection errors; `MqttConfig`'s `Debug` redacts them).

## Not covered

TLS (`mqtts://`), MQTT 5, persistent sessions (clean session each connect — messages
published while the daemon is down are lost), client certificates, will messages, and
`pii-net` egress grants (the broker address is operator config; the client dials it
itself). The test broker is hand-written (`crates/pii-channels/tests/mqtt_broker`), not
a second independent implementation.
