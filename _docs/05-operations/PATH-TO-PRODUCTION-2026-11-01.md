# Path to production — pii, 2026-11-01

**Where production is, measured:** daemon pid 75729 serving `669f5803d90a`, started 2026-10-31, `pii health` rc 0,
client == daemon == that revision. The tree has moved since (`1e8d530` and later) with **0 Rust files changed** —
every commit since the deploy is shell, docs and specs — so the running code is what the tree builds, and only the
compiled-in revision *stamp* differs. That distinction is the one `pii health` cannot make, and the runbook says so.

**What the milestones actually are** (`production-progression.md`, M0→M7) and their state tonight:

| | milestone | state | evidence |
|---|---|---|---|
| M0 | builder loop trustworthy | DONE | autonomy selftest 42 rows, 36 shell tests |
| M1 | detection from own records (spec 150) | mechanism DONE, **delivery was broken** | 11 incidents from the real logs; 496 refusals = ONE record |
| M2 | evidence on a crash (spec 154) | DONE | spec closure |
| M3 | resolution on a branch (spec 151) | DONE (pre-registered condition) | spec closure |
| M4 | deploy card (spec 152) | DONE, enforced in code | installer refuses without a card |
| M5 | first real deploy | **identity only** | 0 posts, 0 runs since deploy |
| M6 | reads its own runtime logs | mechanism DONE | `incidents scan --source runtime` on the live log |
| M7 | retire the dormant surface | **your six decisions** | see `open-items.md` |

## The four things standing between here and production, in order

**1. Detection was suppressed by a delivery check.** Fixed today (`tick.sh`): the incident scan sat *after* the
gateway block, which ends `exit 0` when it believes the gateway is down. Its probe costs 10 s per call here and
misfires under load — its own log said "the gateway is down and would not start" while the gateway had been up for
days. Detection now runs first, and a live process is checked with `pgrep` before any 10-second IPC probe.
*Evidence:* `scripts/autonomy/tests/detection-before-delivery.test.sh` — 4 assertions, including a control where the
gateway is down and the incident line still comes out.

**2. The incident surface that exists today is a file.** `scripts/autonomy/incidents-md.sh` publishes one dated row
per new incident to `_tmp/autonomy/INCIDENTS.md`, once, with a cursor so a re-run never repeats one, wired into the
beat. No credential, no daemon, no restart. *Evidence:* 9 assertions in
`incidents-md-publishes-once.test.sh`. Discord delivery stays open (see step 4) and must not block this.

**3. Nothing posts, because the daemon never received the channel credential.** `/root/.config/pii/pii.env` exists
(mode 0600, both `PII_DISCORD_*` variables) but the unit's `EnvironmentFile=` only applies to a systemd start, and
**this container has no systemd bus** — the daemon is hand-started, so its environment has zero `PII_DISCORD_`
variables, `cron_entries: 0`, `armed: 0`, `channel_posts: 0`. The installer also appends the digest `[[cron]]` block
only when `HAVE_CHANNEL=1`, so it never ran with the credential either. Consequence: M5's "≥7 posts over 7 days"
cannot start counting until the daemon is started *with* that environment.
*One command, one restart, one decision (it posts to your Discord):* source `pii.env`, re-run `install-pii.sh`
(`SKIP_BUILD=1 CONFIG_ONLY=1`) so the digest block is appended, restart with the environment, then expect the first
digest at 12:00Z.

**4. Discord is an interaction surface, not only an output.** The adapter died on 2026-10-31 00:29
(`WebSocket unhealthy`) and the gateway process never re-established it — measured today: **no Discord traffic of
any kind**, which is why your messages got no answer. Restarted 2026-11-01 17:05: **`Connected as bella#9209`**.
For pii's own channel work, the daemon still needs step 3's credential.

## What is mine, and what waits on you

- **Mine, no decision needed:** land today's fixes (steps 1–2) with their gates; keep `leftover.sh` and the tree
  hold enforced; rewrite or retire row 330 (its criteria pass by hand while its test proves a stand-in over a
  synthetic fixture); `minilam` housekeeping.
- **Yours, one line each:** (a) may the daemon hold the Discord credential and post the daily digest — yes/no;
  (b) six M7 decisions; (c) whether `pii` should answer in the Discord channel as an agent (a different thing from
  posting digests, and the gateway already does it).

## The honest summary

The product's *detection* now runs and its *record* is readable. The product's *delivery* — the thing that makes it
a daily presence rather than a log — is one credential and one restart away, and that restart is the only outward
step left: after it, M5 stops being a claim and starts being a measurement.

## Armed, 2026-11-01 — and what that did and did not prove

`deploy/arm-digest.sh --apply` ran against production. The live `pii.toml` now carries

```toml
[[cron]]
name = "digest"
graph = "discord-digest"
schedule = "0 12 * * *"
input = { to = "1558589728671146105" }   # interpolated from PII_DISCORD_CHANNEL_ID
```

and the daemon (pid 1744962) holds both `PII_DISCORD_*` variables in its environment — measured by reading
`/proc/<pid>/environ`, not assumed from the file being present.

**Proved:**
- the digest is armed in the daemon: `pii graph list` → `discord-digest · ce7445f0962b · nodes 6 · timer 0 12 * * * ·
  armed 1 · same`, and the journal shows `cron.rearmed` for `digest` with run ids (`r000000004356`, …);
- the channel **delivers**: `channel.verification_posted` twice, text `pii 0.1.0 is up (revision 669f5803d90a)`, and
  15 `channel.posted` before them. The 41 `errors` on the channel entry are the historical
  `not started: PII_DISCORD_TOKEN … not set` and `no address reachable for discord.com` warnings from every start
  before today.

**Not provable outside the daemon, by design:** a standalone `pii graph run graphs/discord-digest.toml` cannot
exercise the digest — the recall nodes need `memory_read`, which only the daemon grants:

```
capability denied: memory: read not granted (the graph and the node need the memory_read capability)
```

and a manual run of the real graph simply re-suspends on its timer (`in_ms` ≈ 44 178 269, i.e. the next 12:00Z). So
the digest's proof is the 12:00Z run and nothing else. **A daily observer now watches that moment**: cron job
`745f25ecd51a` runs `pii-digest-check.sh` at 12:20Z, counting *digest* posts since noon and explicitly excluding
verification posts — counting those would make the check pass on a day the digest never delivered, which is the exact
false green this page exists to prevent.

**One regression paid for on the way** (`1944bcb`): the arm script read the model host as "the first URL in the
config", which was a comment pointing at `127.0.0.1:8080` — a server that answers 200 but serves `laguna-xs-2.1`,
while the configured model `granite-4.2-3b` lives on `:8084`. Nothing failed loudly. The host is now read from the
`[llama]` section only and the post-install check compares the **values** of `model` and `[llama] host`, because a key
that exists with a changed value passes a key-level verify and still breaks production.
