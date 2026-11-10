# Design: the host dev container (`modules/devbox`), and how the host tests from it

Written 2026-10-23. Status: SUPERSEDED the same day by what was built and measured: see `modules/devbox/` (Containerfile, compose.yaml,
build.sh, gate.sh, check.sh). Differences from this draft: root inside (not a non-root user), the project mounted at its own path
<repo> (not /work), Python only through uv, clang and procps added after the first in-container gates failed without
them. Kept as the record of the original reasoning. Original status: DESIGN, nothing built or run on the host. Every "measured" line below was read from the
the host host over `ssh mvcorrea@<host>` this session (read-only: no container was started, no file written there).

## What the operator asked

> a specific container in the host for this special subject (with all needed tools, skills, hooks and so on), then from
> the host make the testing, using a mounted volume (the working subject). Split the modules in a `modules` folder.

So: ONE purpose-built container image on the host; the working tree is a bind-mounted volume; the host starts the container,
runs the gate and the agents inside it, and reads the result back from the volume. Nothing is installed on the host itself
(spec 209: pii installs nothing, it uses podman).

## Measured on the host

- podman 5.4.2, rootless, user `mvcorrea` (uid 1000), `overlay` storage under `~/.local/share/containers/storage`,
  `/etc/subuid` has `mvcorrea:100000:65536`, `podman.socket` active at `/run/user/1000/podman/podman.sock`.
- No Rust on the host. 32 cores, 125 GB RAM; `/podman` has 90 GB free (85% used), `/` 153 GB free.
- The working tree is `/podman/assistants/hermes<repo>` (owner mvcorrea, mode 755) — the same bytes this
  container sees as `<repo>`. So the "mounted volume" already exists on the host; no copy is needed.
- Image `localhost/pii-agent:1` (1.91 GB): pi 0.1.23, claude 2.1.304, **rustc/cargo 1.85.1**, git 2.47.3; it runs as root
  inside (rootless: root maps to mvcorrea), `WORKDIR /work`. It resolves `api.anthropic.com` and `ollama.com`.
- **The repo pins Rust 1.95** (`rust-toolchain.toml`, `rust-version = "1.95"`, edition 2024). The existing image has 1.85.1,
  so it CANNOT build this workspace. This is the first concrete gap.
- No `ANTHROPIC_API_KEY`, `OLLAMA_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` in the host user's environment, and no
  `~/.claude/.credentials.json` on the host. The keys live in the hermes container's `.env`
  (`/podman/assistants/hermes<agent-host-config>`, mode 600).
- Two production-ish pii processes already run there (a daemon inside the hermes container, pid 24108, and one at
  `~/.local/bin/pii`, pid 3539364). The devbox must not touch either.

## The module layout (`modules/`)

Each module is a directory with its own `README.md`, a `Containerfile` or script set, and a `check.sh` that exits 0 only when
the module's own claim holds. A module depends on others only through a documented interface, so each can be built, run and
replaced alone, and a worker given one module cannot reach the others' files.

```
modules/
  devbox-image/     Containerfile for localhost/pii-devbox:<rev>  (Rust 1.95 + cargo tools + pi + claude + git)
  devbox-run/       start/stop/exec scripts: one container per run, labelled io.pii=1, the tree mounted at /work
  devbox-agents/    per-agent credential + config wiring (env NAMES only), pii-owned PI_CODING_AGENT_DIR
  devbox-skills/    the skills and hooks the container carries, as pinned documents (sha256), read-only
  devbox-gate/      the host-side driver: build image -> run quality-gate inside -> read the verdict from the volume
```

Rules: nothing in `modules/` depends on `_tmp/`; no module writes outside its own directory or the mounted tree's `_tmp/`
(which is scratch by the workspace rule); secrets are env-var NAMES only.

## How the host tests (the flow)

1. `devbox-image` builds `localhost/pii-devbox:<git-rev>` from a base pinned by digest. It installs Rust 1.95 with
   `rustfmt` and `clippy` (the toolchain file's own components), `cargo-llvm-cov` for the coverage gate, and pi and claude
   as the current image does.
2. `devbox-run` starts ONE container per request: `--rm`, label `io.pii=1`, `--network` per the module's declared grant,
   the working tree bind-mounted read-write at `/work`, a named volume for `CARGO_HOME` and `target/` (so the cache lives
   outside the tree and survives runs), `--userns=keep-id` so files written in the tree keep the host user's ownership.
3. `devbox-gate` runs `quality-gate.sh` inside and copies nothing back: the verdict is read from the mounted tree
   (`_tmp/...`), the same place the loop reads it today. The host user never needs a Rust toolchain.
4. The agent run is the same container: pii's coder node (specs 207/209/230/241) already starts a container through the
   podman API socket, labels it `pii.run=<id>`, bounds it with `timeout_ms` and returns its logs; the devbox is the image it
   names.

## What is NOT designed yet, and why

- **Credentials into the container.** The keys are in the hermes container's `.env`, not the host user's environment, and
  the claude login file is on the container side only. The coder passes env NAMES from the daemon's environment
  (`env_pass`); so something on the host must hold the values as env vars for the daemon that starts the container. I
  will not copy a key value anywhere; the operator decides where the host daemon gets them (a root-owned `0600` env file
  read by the unit, as `deploy/pii.service` already documents).
- **Which daemon starts it.** Two pii daemons already run on the host; which one owns the devbox, and which store records
  its runs, is the unresolved two-daemons/two-stores topology. The first module that needs the answer is `devbox-run`.
- **The `mkdir /run/user/0/pii: Permission denied` seen earlier** is unexplained. The host user's runtime dir is
  `/run/user/1000`, so a path under `/run/user/0` points at a daemon configured for root; I have not read why.
- **Network for the gate.** `cargo` needs crates.io the first time; the gate has `net-check` and the coder defaults
  `network = none`. The image build can fetch (build is a recorded act, spec 209); the run stays `none`, with the cache
  volume pre-populated by a `cargo fetch` step.

## Build order (each step independently checkable, none touches production)

1. `devbox-image`: build on the host, check `rustc --version` = 1.95 and `cargo clippy --version`. Gate: a script that
   prints the versions and exits 0. **Needs the operator's go: it writes images into the host's podman storage (~2-3 GB)
   and the host is production-adjacent.**
2. `devbox-run`: start/exec/stop one container against a scratch copy of the tree, never `/podman/assistants/...` directly.
3. `devbox-gate`: run `lint-files` first (cheap, no network), then the full gate.
4. `devbox-agents` / `devbox-skills`: after the credential decision above.

## Why it is split into modules

The 450-line file gate was binding the workers (the last two specs both stopped on it). Applying the same rule to the
deployment side stops the same failure: five small units with a `check.sh` each, rather than one large script that no
worker can change without reading all of it.
