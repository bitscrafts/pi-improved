# Deploy runbook — pii on the host (spec 142)

The tree is ahead of the machine (059 §0). This is the operator's document: a script installs, you do the two
host actions, then you verify. Nothing here has been run on the host by the orchestrator (no host access).

## 0. Preconditions (the script aborts, changing nothing, if one is missing)

- llama.cpp answers: `curl -s http://10.10.10.217:8082/v1/models`
- Optional (spec 157): `PII_DISCORD_TOKEN` (bot token) and `PII_DISCORD_CHANNEL_ID` (the one forum channel id)
  exported in the shell that runs the installer. When both are set the installer writes
  `~/.config/pii/pii.env` (mode 0600, outside the repo) and the digest cron's `to`; the token is never written
  to the config, the unit or the store. When either is absent nothing is written and the daemon starts with
  `channels: []` (a state to report, not a failure). With them, the daemon starts the channel and makes one
  startup post naming its revision (`pii health --json`: `channel_posts >= 1`).
  Check: `bash scripts/checks/no-secret-in-tree.sh`.
- `$PREFIX` (default `~/.local/bin`) exists or can be created.

## 1. Install

```sh
bash deploy/install-pii.sh                 # SCOPE=user (default); SCOPE=system for /etc/systemd/system
```

It builds `pii`, `pii-web`, `pii-evolve` and `pii-mcp`; moves existing binaries/config/unit to
`$PREFIX.old-<epoch>/` (never deletes); installs; writes `~/.config/pii/pii.toml` (model
`llamacpp-server:granite-4.2-3b`, `[llama] host`, trajectories on, one `discord` channel, one daily
`discord-digest` cron) and the unit; copies `graphs/*.toml` to `$STATE_DIR/graphs` (default
`~/.local/state/pii/graphs`; a differing existing copy is moved aside to `$PREFIX.old-<epoch>/graph.<name>`); then prints the operator commands. **`pii-mcp` is only staged as
`$PREFIX/pii-mcp.new`** — see §5.

## 2. The two host actions

1. **Place the token** if the installer did not (mode 0600; the file is yours, never commit it):
   ```sh
   install -m 0600 /dev/null ~/.config/pii/pii.env
   printf 'PII_DISCORD_TOKEN=%s\nPII_DISCORD_CHANNEL_ID=%s\n' '<token>' '<forum-channel-id>' > ~/.config/pii/pii.env
   ```
2. **Enable the unit**:
   ```sh
   systemctl --user daemon-reload && systemctl --user enable --now pii
   loginctl enable-linger "$(id -un)"
   ```
   (system scope: drop `--user`; the unit runs as the installing user, not root.)

## 3. Verify (expected output)

| Command | Expected |
|---|---|
| `PII_RUNTIME_DIR=/run/user/$(id -u)/pii pii health --json` | exit 0; build identity equals the deployed commit (`git rev-parse --short HEAD`) |
| `pii run get <id>` on a fresh run (start one with the usual `pii run` path) | the run record, state finished |
| `pii metrics \| grep pii_channel_posts_total` | the counter **increments** after a triage message or the digest |
| the Discord forum | receives the daily digest post |
| `ls /run/user/$(id -u)/pii/graphs/` (`<runtime_dir>/graphs/`) | lists `discord-triage.toml` and `discord-digest.toml` |
| `journalctl --user -u pii` / `tail ~/.local/state/pii/pii.log` | `daemon.started`, no startup failure |

The graphs live in `$STATE_DIR/graphs` (persistent); the runtime dir is tmpfs, so the unit's `ExecStartPre=`
symlinks `<runtime_dir>/graphs` to it on every start.

Caveat: `[[cron]]` only mirrors the schedule; firing is the graph's own `timer` entry node, and
`graphs/discord-digest.toml` has none yet (its entry is `devices`). Until a timer-fronted graph exists, the digest
is exercised by running the graph once with `{"to": "<forum id>"}`; that gap is a separate item.

## 4. Measurement window

Deployment is **not** closed by the install. It is closed by **≥ 7 forum posts over 7 days** with the Hermes
call/error rate unchanged (059 §2.3). Record posts and the Hermes rate at day 0 and day 7.

## 5. The `pii-mcp` swap — a separate step with its own probe

Do this only after §4's first checks pass, never as part of the install:

```sh
mv "$PREFIX/pii-mcp" "$PREFIX/pii-mcp.prev"      # if one is installed
mv "$PREFIX/pii-mcp.new" "$PREFIX/pii-mcp"
```

Probe: call `tools/list` once through Hermes, then compare the `mcp__pii__*` call rate against the previous
baseline of **161 calls per 30 days** (059 §2.3). If the rate drops, **roll back**: reverse the two `mv`s.

## 6. Rollback

```sh
bash deploy/rollback-pii.sh        # same PREFIX / SCOPE as the install
```

Stops the unit if running, **moves** the newest `$PREFIX.old-<epoch>/` contents back (binaries, `pii.toml`,
unit), keeps what it replaced in `$PREFIX.rolledback-<epoch>/`, and prints each file restored. Then
`systemctl --user daemon-reload && systemctl --user restart pii`. Exercised in a scratch prefix by
`bash scripts/checks/deploy-dry-run.sh` (last line `DRY-RUN OK`).

## In-process llama.cpp binding check (on demand, not in the gate)
Run `cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings` by hand (needs clang, cmake, make, ninja; ~12 min of C++). The routine gate skips it: pii reaches models over HTTP (`llamacpp-server:`) and the feature is opt-in.
