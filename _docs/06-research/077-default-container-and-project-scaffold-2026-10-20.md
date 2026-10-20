# 077 — A default agent container, and a `.pii/` project scaffold (research, 2026-10-20)

**Question (user):** "pii must have a default container (with needed tools installed) and on a coding request it just mount the
volume with the code it should act on … and in that folder all related pii data should be in a `.pii` folder (together with the
`.claude` and `.pi`) copied on the project setup."

**Answer in one line:** yes — this is the CI/devcontainer pattern (pinned runtime + mounted workspace), and the two agent
tools we use already define *safe* project-scoped config directories, so the scaffold is mostly assembly rather than invention.
Below: what the sources actually say, what that implies for pii, and the open questions that must be measured before a spec.

## 1. What the primary sources say

### 1.1 pi (pi.dev/docs/latest/configuration)
- User-level config lives in the **agent directory**, default `~/.pi/agent`, relocatable with **`PI_CODING_AGENT_DIR`**.
- **Project config lives in `.pi/` under the working directory** and loads after *project trust*: `settings.json`, `mcp.json`,
  `SYSTEM.md`, `APPEND_SYSTEM.md`, `extensions/`, **`skills/`**, `prompts/`, `themes/`.
- Sessions: `~/.pi/agent/sessions/` by default, grouped by working directory; `--session-dir` or
  `PI_CODING_AGENT_SESSION_DIR` override it. `sessionDir` is read **before** trust resolves.
- **The agent directory holds `auth.json` (saved API keys and OAuth credentials).** This is the decisive fact for the scaffold:
  the *project* `.pi/` is safe to copy into a repo; the *user* agent directory is not.

### 1.2 Claude Code (docs.claude.com …/claude-directory, …/settings)
- Project scope is a **`.claude/` directory at or above the cwd**; project settings **outrank** user settings. It carries
  `CLAUDE.md`/`AGENTS.md`, `settings.json`, hooks, skills, subagents, commands.
- `~/.claude` is personal, across all projects; **`CLAUDE_CONFIG_DIR`** relocates every `~/.claude` path.
- `CLAUDE.local.md` is the explicitly private, gitignored per-project file — the documented place for anything that must not
  be committed.

### 1.3 Rootless podman bind mounts (redhat.com, blog "debug issues with volumes mounted on rootless containers")
- Rootless podman runs in a **user namespace**; bind-mount ownership is the single most common failure area.
- `--userns=keep-id` maps the caller's UID into the container; the `:U` volume option instead **chowns** the host files to the
  container's user — which changes ownership of the operator's own files.
- Practical consequence for pii: files an agent writes must come back owned by the caller. **`--userns=keep-id` is the option to
  test; `:U` must be avoided on a mounted workspace.**

### 1.4 Precedent (devcontainers / Codespaces)
- The established pattern is precisely what the user proposed: **one image carrying the toolchain, the source mounted in**, so
  N repositories share one reproducible runtime instead of N Dockerfiles. The known cost is I/O on bind mounts, and the known
  discipline is that the image is versioned and the mount is explicit.

## 2. What this implies for pii

```
RUNTIME            one image, digest-pinned, daemon-owned default (localhost/pii-agent:1 — already built tonight with
                   pi 0.1.23, claude 2.1.304, cargo/rustc 1.85.1). A graph may name an image only from an allow-list.
REQUEST            mounts the project directory rw; nothing else from the operator's home.
PER-PROJECT DATA   <project>/.pii/  — pii's own project scope: graphs/, skills/ (DocKind::Skill mirrors), policy.toml
                   <project>/.pi/     — pi project config (settings.json, skills/, prompts/) — NO auth.json
                   <project>/.claude/ — claude project config (settings.json, skills/, commands/) — no credentials
                   all three are TEMPLATES copied at `pii project init`, and all three are git-committable.
NEVER COPIED       ~/.pi/agent/auth.json · ~/.claude/.credentials.json (or any CLAUDE_CONFIG_DIR equivalent) ·
                   ~/.config/pii/pii.env · any session transcript. Credentials travel as ENV by NAME into the container,
                   never as files inside the project.
```

The scaffold is therefore an **allow-list of what to copy**, not a copy of a home directory: a whitelist of three directory
trees whose content is configuration, with the auth-bearing files named explicitly as excluded.

## 3. Open questions that must be MEASURED before a spec

1. **Ownership.** Does `podman run --userns=keep-id -v <project>:/work` return files owned by the caller on this host, with the
   agent running as the image's user? Measure with a `touch` inside and `stat -c %u:%g` outside. Control: the same test
   without `--userns`.
2. **Sessions and resumability.** pi groups sessions "by working directory". If the workspace is a mount at `/work`, the
   session path derived inside the container differs from the host path — does a resumed session still resolve?
   `PI_CODING_AGENT_SESSION_DIR` is the lever; measure whether a session written in the container resumes on the host.
3. **Cache strategy.** A shared `target/` serialises on cargo's lock (measured tonight: two concurrent builds turned a 4-minute
   build into 13+). Choose: per-run target dir (clean, slow) vs read-only warm cache + copy (fast, two-layer) — and measure
   both on a cold and a warm run.
4. **Network.** `--network none` cannot reach a model API. A default image with an agent needs egress to exactly one host,
   declared per graph. Measure that a graph declared `api-only` cannot reach anything else.
5. **`.claude`/`.pi` precedence.** Project `.claude` outranks user config (claude docs) and pi loads project `.pi` after trust.
   For an agent in a container with NO home directory config, does the project scope behave as the ONLY scope (desired), or do
   the tools refuse to start without a user-level agent dir? Measure both agents with an empty `$HOME`.
6. **Trust.** pi loads project config only after *project trust* is granted; headless runs must have a documented,
   non-interactive way to trust the mounted project — otherwise the scaffold silently does nothing.
7. **Secrets hygiene.** A test that greps a freshly scaffolded project for `auth.json`, `credentials`, `*_API_KEY=` and fails on
   any hit. This must be an exit criterion, not a convention.

## 4. Proposed shape (for the spec, not the code)

```
pii project init <dir>
    .pii/            settings.toml, graphs/ (empty or a starter), skills/ (empty), policy.toml
    .pi/             settings.json, skills/, prompts/            (no auth.json — asserted)
    .claude/         settings.json, skills/, commands/           (no credentials — asserted)
    .gitignore +=    .pii/runtime/  .pi/sessions/  .claude/local/  *_env  *.env
    prints           "image: <digest>  ·  mounts: workspace rw  ·  network: none|api-only"

[coder] in pii.toml (daemon-owned)
    default_container_image = "localhost/pii-agent@sha256:…"
    allowed_images          = ["localhost/pii-agent@sha256:…"]
    userns                  = "keep-id"          # measured in §3.1
    mounts                  = workspace:rw, cache:/cargo:ro
    network                 = "none" | "api-only"

graph:  kind = "coder", workdir = <project>, container_image = optional (must be allow-listed)
journal: coder.spawned{image_digest, workdir, mounts, userns, network, agent, model, pid}
```

## 5. What I would NOT do

- Copy a home directory into a project "so the agent has its settings". That is how `auth.json` and session transcripts end up in
  a commit.
- Let a graph name any image it likes: the allow-list is what keeps a graph from becoming an arbitrary-code channel — the same
  reasoning that removed shell strings from criteria (spec 215).
- Use `:U` on the workspace: it chowns the operator's files.

## 6. Status of the claim

Nothing here has been run yet: §3 is a measurement plan, not evidence. The image (`localhost/pii-agent:1`) and the temp
workspace from tonight exist and are the test bed for §3.1–§3.3. A spec should be written only after those five measurements,
because every one of them can invalidate the shape above — and this project has paid repeatedly tonight for building before
measuring.
