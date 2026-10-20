# 078 — pii as an agent runtime: default image, mounted workspace, project scaffold (Opus review, 2026-10-20)

> Plan-mode note: this is the full text of the deliverable. On approval it is written verbatim to
> `/projects/pi-improved/_docs/06-research/078-opus-agent-runtime-design-2026-10-20.md`. Nothing else is modified and nothing is committed.

**Read:** brief §0–§5; 077; `coder.rs`, `coder_container.rs`, `criteria.rs`, `agent_worker.rs` (pii-server
`node_kinds/nodes/`); `pii-tools/src/sandbox.rs`; `pii-cli/src/cmd_daemon.rs`, `cmd_sandbox_exec.rs`; `deploy/pii.service`;
root `Cargo.toml` and `rust-toolchain.toml`. **Not run:** podman is not on this machine and `/tmp/pii-ws-3aQWp1` is not here.
Everything marked *predicted* has to be measured on lnx (§3).

## §0 Verdicts

| Q | Verdict | One line |
|---|---|---|
| Q1 one image + mount | **AMEND** | Right shape, but the image ignores the project's declared toolchain (pii itself pins 1.95; the image has 1.85). Guard: refuse before the spawn on a toolchain mismatch, by name. |
| Q2 scaffold | **AMEND** | Copy **nothing** from `$HOME`. Generate everything from pii's templates and the skill store. The real leak is the agent writing a secret into the diff, so scan the diff for the secret values themselves. |
| Q3 materialise skills + hooks | **AMEND** | Materialise from the store into a per-run dir and bind it **read-only** over `.claude/skills` and `.pi/skills`. The store wins, drift gets journalled, nothing is auto-imported. Hooks give the agent feedback, they don't enforce anything. The enforcement is the post-run path check. |
| Q4 seams | **AMEND** | Blast radius does the least work. It has never contained a process, and the criteria step runs agent-written code **on the host**. Fix it first by running the criteria in the image against a tree whose tests come from the pre-run snapshot. (a) AMEND, (b) ACCEPT with additions. |
| Q5 measurements | **AMEND** | Add M0 (does a sandboxed daemon start a container at all?) ahead of the six. M0 can sink every other one. |
| Q6 claims | **ACCEPT** | §4. Nothing about agent execution can be claimed tonight. |
| Q7 premises | **AMEND** | §2 has 9 entries. The biggest ones: the inherited Landlock domain, criteria running on the host, the "rootless" assumption, and claude `-p` denying edits by default. |

## §1 Answers

### Q1 — one default image + mount the project

The shape is right: one runtime image, N repos, the source mounted in. That's devcontainers, and it fits "pii installs
nothing". The thing it loses compared with per-project images is **the project's own declaration of its toolchain**.

**The failure that bites first** (after M0): toolchain skew. The repo's `Cargo.toml` says `rust-version = "1.95"` and
`rust-toolchain.toml` pins `channel = "1.95.0"`. The image carries debian cargo/rustc **1.85**, and an apt cargo
ignores `rust-toolchain.toml`. So the first time a coder is pointed at pii's own tree, every in-container `cargo` call fails
to compile (`Duration::from_mins` alone is newer than 1.85). The agent then "fixes" that, either by downgrading code or by
editing `rust-version`, inside a task that had nothing to do with it. The temp workspace passed only because it is a toy crate.

**The smallest guard:** before the spawn, read the project's declared toolchain (`rust-toolchain.toml` channel, else
`Cargo.toml` `rust-version`) and the image's `rustc --version` (probe once per digest and cache it next to the digest). Refuse
`coder.toolchain_mismatch{required, image}` when the image is older. Don't put rustup in the image: under `--network none` it
would try to download the pinned toolchain and fail at run time instead. The answer to a mismatch is a second allow-listed
image, chosen by the operator, not by the graph.

Two lesser losses to record, not to fix now:
- **Per-project system deps** (a `-sys` crate needing `libssl-dev`, a test needing `sqlite3`). These fail late and look like
  agent errors. Record the image's package list in the journal so a failure can be read against it.
- **The image is not yet pinned by digest.** `launch_in_container` accepts the tag `localhost/pii-agent:1`, resolves a digest at
  run time and journals it. That records the pin but doesn't enforce it. The allow-list from 077 §4 is not implemented. The
  graph also names `network` freely: `coder_container::run_argv` passes `--network <graph text>` verbatim, so `host` is
  accepted.

### Q2 — the scaffold: what may be copied, what never, what is generated

Rule: **the scaffold has no source in `$HOME`.** "Copied at project setup" has to mean copied from pii's own versioned
templates and its skill store, never from the operator's agent dirs. Every file under `~/.pi` and `~/.claude` mixes
configuration with identity or credentials, and 077's deny-list (auth.json, .credentials.json) misses several of them.

| Class | Items | Why |
|---|---|---|
| **Never copied** | `~/.pi/agent/auth.json`; `~/.pi/agent/sessions/`; any pi `models.json` / settings holding provider `apiKey`; `~/.claude/.credentials.json`; **`~/.claude.json`** (in `$HOME`, *not* in `~/.claude/`: OAuth account, user id, per-project history); `~/.claude/settings.json` (its `env` block and `apiKeyHelper` commonly carry keys); `~/.claude/projects/` and `history.jsonl` (transcripts); any `mcp.json` / `.mcp.json` (MCP servers embed tokens in `env`/`headers`); `CLAUDE.local.md`, `settings.local.json`; `~/.config/pii/pii.env`; the operator's **global `CLAUDE.md`** (personal policy, e.g. the commit-identity rule and email, has no place in a repo) | credentials, identity, or private instructions |
| **Generated** (from pii templates, deterministic, content-hashed) | `.pii/settings.toml`, `.pii/policy.toml` (protected-path globs, Q4b); `.claude/settings.json` (permissions only, **no hooks**, see Q3); `.pi/settings.json` (no provider keys); `AGENTS.md`/`CLAUDE.md` project text written by pii; `.gitignore` entries | pii owns them and can regenerate them byte for byte |
| **Materialised** (from `DocKind::Skill`, Q3) | `.claude/skills/<n>/SKILL.md`, `.pi/skills/<n>/SKILL.md` | audited and pinned in the store, so the files are build outputs |
| **Runtime, never in the tree** | sessions, env files, caches | see below |

Three corrections to 077 §4:
1. **Sessions are not where 077 thinks.** `agent_worker::plan` passes `--session <workdir>/.pii-sessions/<stem>.jsonl`, i.e.
   `/work/.pii-sessions/…`. That is inside the project, outside `.pii/`, and **not** in 077's gitignore list (which names
   `.pi/sessions/`). Transcripts contain tool output, and tool output contains whatever `env` or `cat .env` printed. Move
   them out of the tree into a daemon-owned dir mounted at a fixed container path, and gitignore `.pii-sessions/` in the
   meantime.
2. **`.gitignore` is not a guard.** It lives in the agent-writable tree, and an agent can un-ignore a file.
3. **The scaffold grep (077 §3.7) checks the wrong moment.** A fresh scaffold is clean by construction. The leak happens
   when an agent, which has `ANTHROPIC_API_KEY` etc. in its env, writes a value into a file in `/work`. The guard: before the
   orchestrator commits, the daemon (which holds the exact `env_pass` *values*) searches the staged diff and the untracked
   files for each value, and its base64 form, and refuses `change.secret_value{name}` on a hit. This matches exact values,
   so it has no false positives and needs no gitleaks heuristics. It's an exit criterion.

### Q3 — materialising skills; drift; the "never edit tests" hook

**Materialise: yes, but not into the writable tree.** At run start the daemon renders each `DocKind::Skill` the node names into a
per-run dir (`<runtime>/runs/<id>/skills/`) and binds it **read-only** over `/work/.claude/skills` and `/work/.pi/skills`.
The agent reads exactly the audited bytes and can't change them mid-run. The committed copies in the repo stay for humans
who run an agent on the host.

**The drift rule** (memory document vs on-disk file at run start):
- **The store wins for the run, always.** The agent sees the ro overlay, not the disk copy.
- If the disk copy's sha256 differs from the store's render, journal `skill.drift{name, disk_sha, store_sha}` and show it in
  `pii skill explain`. The run proceeds, because the drifted file is not what the agent reads.
- **Never auto-import disk → store.** That would be an unaudited write path into audited memory. A human imports
  through `pii skill edit`, which re-pins.
- If the post-run diff touches any materialised path, fail the node (protected path, Q4b). With the ro overlay this can only
  happen through a mount failure, which is exactly what you want to hear about.

*Predicted surprise:* if `.claude/skills` doesn't exist in the project, podman creates the mountpoint **in the host tree**,
owned by whoever the mapped root is. Pre-create it in the scaffold (M1 measures who owns it).

**Is a hook the right enforcement for "never edit tests"? No.** It gives cheap, early feedback, and it is bypassable in both
dialects:
- Claude `PreToolUse` on `Edit|Write|MultiEdit|NotebookEdit` doesn't see `Bash` (`sed -i`, `cat >`, `git checkout`), and
  pattern-matching shell text is the thing spec 215 removed.
- Hooks in **project** `.claude/settings.json` are in the agent-writable tree. Claude snapshots hooks at startup, so an edit
  doesn't disarm the current run. It does disarm the next run and the operator's own host session in that repo.
- A pi extension is TypeScript in `.pi/extensions/`: the same writability problem, and it also runs on the host when a human
  opens the repo with pi.

**The smallest pair that holds:**
1. *Feedback (in the agent):* claude: a `PreToolUse` hook matching `Edit|Write|MultiEdit|NotebookEdit` that exits 2 when
   `tool_input.file_path` matches the policy's protected globs. Deliver it through **image-level managed settings**
   (`/etc/claude-code/managed-settings.json`, which outranks project scope), or via `--settings <ro path outside /work>`, and
   never through project `.claude/`. pi: one extension intercepting write/edit tool calls with the same globs, loaded from a
   ro path outside `/work` (check the exact event name and how to load an external extension against pi 0.1.23; nothing in the
   brief establishes it).
2. *Enforcement (outside the agent):* the post-run path check in Q4b, plus criteria run against snapshot tests (Q4a). These two
   are what "holds". If both hooks were deleted, the guarantee would be unchanged, and that's the property to test.

### Q4 — the four seams

Ranked by the work each does **tonight**:

1. **Story (journal).** It works. `coder.spawned/exited`, argv digest, env names, and with 224 in the tree the refusal reason now reaches
   the run. It's the only seam that has caught a real defect tonight (the masking).
2. **Claim (named criteria).** It does real work: it refused `cargo test` by name. But it computes its verdict **over the
   tree the agent just edited, with tests the agent could edit**, and the production table names only pii's own tests (the
   any-workspace row was reverted in 26ae945). So it can only ever refuse on a foreign workspace, and on pii's tree it can be
   gamed.
3. **Change (diff + orchestrator commit).** It's real, but a human does it. It is the only point where the agent's output is judged
   by something the agent can't touch. Nothing in pii yet checks the diff mechanically (no path check, no secret check).
4. **Blast radius (container + landlock).** **It does the least: zero.** No agent process has run inside it. Structurally:
   - `coder.rs` runs every criterion with `ctx.proc().run(&argv, &workdir, …)` **on the host** after the container exits.
     `cargo test` on the agent's tree executes agent-written `build.rs`, proc-macros and test bodies as the daemon's user,
     outside the container. The container contains the agent's thinking and not its code.
   - The mount is the whole project **including `.git/`**. An agent can write `.git/hooks/pre-commit` or
     `core.fsmonitor`/`core.hooksPath` in `.git/config`, and the orchestrator's next `git status`/`git commit` on the host
     runs it. The same goes for `.claude/settings.json` hooks and `.pi/extensions/` the next time a human opens the repo.
   - The two layers aren't additive. Landlock is inherited by podman (see §2.1), so tonight they more likely cancel each other
     out than stack.

**Strengthen first: blast radius, by moving the criteria into the image.** Run each criterion in a *second* `podman run`
(`--network none`, same digest) over a tree assembled as *post-run source + pre-run tests*. That one change closes the
host-execution hole, makes the toolchain the same for agent and verdict (no host/image rustc thrash in `target/`), and gives
Claim its snapshot. Separately, mount `.git` read-only or not at all: give the agent a worktree whose `.git` is a file pointing at
a ro gitdir, or just no `.git`, and the daemon computes the diff itself.

**(a) Mount tests read-only, or run criteria against a pre-run snapshot. AMEND: the snapshot, not the mount.**
- A ro mount of `tests/` misses Rust unit tests in `src/**` (`#[cfg(test)] mod tests`), which are the most common kind, and
  can't cover a task whose job is to *write* a test.
- An agent hitting EROFS improvises (copies the test elsewhere, edits `Cargo.toml` `[[test]]`/`autotests`). That's noise, not
  safety.
- The snapshot does it by construction: at spawn, record the base tree (git tree id). At verdict, overlay the
  **protected** paths from the base onto the post-run tree, then run the criteria. The protected set comes from `.pii/policy.toml`,
  whole files, default `tests/**`, `**/tests/**`, `Cargo.toml`. Note that unit tests inside `src/**` are *not* covered by a
  path rule. For those the guard is (b) plus the diff review, and the spec should say so rather than imply coverage. The node may name exceptions (`allow_test_edits = ["tests/new_case.rs"]`) for write-a-test tasks. Keep the ro mount
  only as optional feedback, not as the guard.

**(b) Per-run tree hash + allowed-path refusal. ACCEPT, with three additions:**
1. The hash covers what git doesn't show: untracked **and ignored** files (`git status --porcelain --ignored`), plus
   `.git/config` and `.git/hooks/*`. Otherwise the most dangerous changes go uncounted.
2. The default allowed set excludes `.git/`, `.pi/`, `.claude/`, `.pii/`, `build.rs`, `rust-toolchain.toml`, `.cargo/`
   (an agent-written `.cargo/config.toml` can set `runner`/`rustc-wrapper` and run code under every later host cargo). A
   node widens the set by naming paths, never by glob-all.
3. The daemon evaluates it **outside the container, before any criterion runs**, and journals
   `coder.spawned{tree_before}` and `coder.exited{tree_after, paths_changed, paths_refused}`. Refusal fails the node with
   `change.path_refused{path}`, which is the 224 lesson applied in advance: the reason has to be the run's error, not a log line.

### Q5 — measurements before spec 225

Listed in §3. Order matters: **M0 first.** If a sandboxed daemon can't start a container, M1–M6 describe a configuration
that doesn't exist.

### Q6 — §4.

### Q7 — §2.

## §2 Premises in the brief that are wrong (or will surprise at runtime)

1. **"Container + landlock" as two layers.** `pii daemon` applies Landlock to **itself** (`cmd_daemon.rs:86`,
   `apply_workspace`), default profile `workspace`, and refuses to serve unless `no_new_privs` is set (`sandbox.rs:113`).
   Everything the daemon spawns inherits both: podman, conmon, crun, the agent, cargo. *Predicted:* (i) Landlock denies
   `mount(2)`/`pivot_root(2)` to any sandboxed thread, so crun can't build a rootfs; (ii) `no_new_privs` neuters the
   setuid/file-cap `newuidmap`, so rootless podman can't build a user namespace; (iii) writes to
   `/var/lib/containers` or `~/.local/share/containers` are outside the grant. Every 1–2 ms refusal tonight happened *before*
   podman ran, so none of this has been exercised. M0 decides it. The likely remedies are a daemon with `--sandbox off`
   (and the container as the only boundary) or podman over its API socket from an unconfined service. Both are decisions,
   not fixes, and both belong in 225.
2. **"Rootless podman."** This host is uid 0; `AGENT_BIN_DIR` is `/root/.local/bin`; but `deploy/pii.service` says
   `DynamicUser=yes` (no home, no subuid range, so rootless podman can't work at all). Which user the lnx daemon actually
   runs as decides whether the Red Hat rootless guidance applies. Under rootful podman, files written by container root are
   host root, and `keep-id` is not the lever. M0 records it.
3. **"The image is used by the coder's verdict."** It isn't. Criteria run with **host** `cargo` (`coder.rs:576–585`). The
   baseline "fails inside the image" was a manual measurement, not what the node would do. And agent-side cargo (1.85, image)
   and verdict-side cargo (host) sharing one `target/` invalidate each other's fingerprints, so every alternation is a full
   rebuild, plus root-owned artifacts from the container.
4. **"pi sessions are grouped by working directory."** True for pi, irrelevant here: pii passes an explicit `--session
   /work/.pii-sessions/<stem>.jsonl`. The file survives `--rm` because it's in the mount, and it lands in the repo tree. The
   resume question is not path grouping. It's whether pi on the host accepts a session file whose recorded cwd is `/work`
   (M2).
5. **"Project `.claude/` outranks user scope."** True for *settings keys*, but: `CLAUDE.md` files from every level are
   **concatenated**, not overridden; permission `deny` rules from any scope still apply; **managed** settings outrank project.
   And "at or above cwd" means a workspace nested under another repo (e.g. under `/projects/pi-improved/_tmp/`) inherits
   *that* repo's `CLAUDE.md` and skills on the host. In the container only `/work` exists, so no walk-up happens. Host and
   container runs of the same project see different instructions.
6. **claude headless can edit.** By default `claude -p` with no `--permission-mode`/`--allowedTools`/settings
   permissions denies every tool that needs approval, so edits silently don't happen and the exit code is 0. With **no
   criteria**, `deferred_verdict` turns rc 0 into `ok` (`coder.rs:148–157`), so a claude coder that changed nothing reports
   success downstream. Also, `--dangerously-skip-permissions` refuses to run as root, and the image's default user is
   probably root.
7. **pi trust.** Without a headless trust grant the scaffold may *silently* not load, and pi still exits 0. That is the
   same masking shape as 224. Also, per 077 §1.1, `sessionDir` is read *before* trust, so an untrusted repo's `.pi/settings.json`
   can still redirect where transcripts go. Check that against the docs, and pin the session dir by flag, not by settings.
8. **"The auth-bearing files named as excluded" (2e7de97)** is the wrong shape. A deny-list missed `~/.claude.json`,
   `settings.json` `env` blocks, `mcp.json` and `models.json` (Q2). An allow-list of generated files can't miss them.
9. **"`criteria_are_named_not_shell` is RED (3/1)."** The working tree has *uncommitted* 224 changes (`coder.rs`
   `refuse_criterion` now returns `Outcome::Fail`, `cmd_trace.rs`, `agent_calls.rs`, the test file, and three new tests). The
   red count may describe the tree before or during 224. Re-measure it, don't quote it. (Not run here: running cargo would
   write `target/`.)

## §3 Measurements owed (run on lnx, as the daemon's real user, through the daemon's real sandbox)

Abbreviations: `IMG=localhost/pii-agent:1`, `WS=/tmp/pii-ws-3aQWp1` (recreate it if it's gone), `SB="pii sandbox-exec --profile workspace --workspace $WS --"`.

**M0 — Can a sandboxed daemon start a container at all?**
```sh
systemctl show pii -p User -p DynamicUser -p ExecStart
pii loop events --log | grep daemon.sandbox.in_force | tail -1
podman info --format '{{.Host.Security.Rootless}}'            # as the daemon's user
podman run --rm --network none $IMG true; echo rc=$?          # control: unconfined
$SB podman run --rm --network none $IMG true; echo rc=$?       # under the daemon's profile
```
*Reading:* control rc=0 and sandboxed rc≠0 means Landlock/NNP blocks containers, and 225 must choose `--sandbox off` or
podman-over-socket before anything else. Both rc=0: record why (kernel/ABI) and continue.
*If wrong:* every later measurement passes in a shell and the first real coder run fails inside podman with an error the
journal names only as a spawn failure.

**M1 — Ownership under the mount.**
```sh
for U in "" "--userns=keep-id"; do
  podman run --rm $U -v $WS:/work $IMG sh -c 'id -u; touch /work/own$$; mkdir -p /work/.claude/skills'
  stat -c '%n %u:%g' $WS/own* $WS/.claude/skills; done
```
*Reading:* files are caller-owned in the mode pii will use. Rootful: `keep-id` either errors or maps differently; record which.
*If wrong:* root-owned files in the operator's tree, `git` refusing a "dubious ownership" repo, and orchestrator commits
failing on the first run that wrote anything.

**M2 — Session resume across the mount.**
```sh
podman run --rm --env-file <keys> -v $WS:/work -w /work $IMG \
  pi --session /work/.pii-sessions/m2.jsonl --model <m> -p 'say ok'
head -c 400 $WS/.pii-sessions/m2.jsonl                         # recorded cwd?
cd $WS && pi --session $WS/.pii-sessions/m2.jsonl --model <m> -p 'what did you say?'
```
*Reading:* the host run sees the first turn. Note the recorded cwd (`/work`) and whether pi warns, refuses or rebases it.
*If wrong:* "resume" in the queue silently starts a fresh session, so the retry has no memory of the failure it's retrying.

**M3 — Cargo cache under a shared `target/`.**
```sh
# cold and warm, each: (a) per-run CARGO_TARGET_DIR in the container; (b) shared target, image cargo then host cargo
/usr/bin/time -f %e podman run --rm -v $WS:/work -e CARGO_TARGET_DIR=/work/target $IMG cargo test --no-run
/usr/bin/time -f %e cargo test --no-run --manifest-path $WS/Cargo.toml
```
*Reading:* whether alternating image cargo and host cargo rebuilds from scratch (fingerprint churn), and the cold/warm cost of a
per-run target. This also shows whether moving criteria into the image (Q4) makes the shared cache coherent.
*If wrong:* 4-minute builds become 13+ (already measured tonight with two concurrent builds), so criteria time out at 10 min
and fail as "criterion failed", which reads as an agent defect.

**M4 — Network: api-only.**
```sh
podman network create --internal pii-api-m4
# + one egress proxy container on that network allowing exactly the model host; agent gets HTTPS_PROXY
podman run --rm --network pii-api-m4 -e HTTPS_PROXY=http://proxy:3128 $IMG sh -c \
  'curl -sS -o /dev/null -w "%{http_code}\n" https://<model-host>/; curl -sS -m5 https://example.com; echo rc=$?'
```
*Reading:* the model host answers, everything else fails (including direct IP without the proxy). There's no podman mode
called "api-only". It's an `--internal` network plus an allow-list proxy, or it doesn't exist. Separately: `network`
must become an enum the daemon maps (`none`|`api-only`), never free text (`host` is accepted today).
*If wrong:* "api-only" means "the internet", and with the keys in the env that's an exfiltration path.

**M5 — Empty `$HOME`, project config only.**
```sh
podman run --rm --network none -e HOME=/tmp/h -v $WS:/work -w /work $IMG sh -c 'mkdir -p $HOME; pi --version; claude --version'
# then each agent with keys via env + a project skill whose body says "answer only SKILL-LOADED":
podman run --rm -e HOME=/tmp/h --env-file <keys> -v $WS:/work -w /work $IMG claude -p 'reply with your skill rule'
podman run --rm -e HOME=/tmp/h --env-file <keys> -v $WS:/work -w /work $IMG pi --model <m> -p 'reply with your skill rule'
```
*Reading:* both start, and both reply `SKILL-LOADED`. Starting isn't loading, so the marker is the test. Also check whether
claude writes `~/.claude.json` and fails on a read-only HOME.
*If wrong:* the scaffold "works" (rc 0) while the agent never read it, so instructions are a placebo.

**M6 — Headless pi trust, and claude headless permissions.**
```sh
# pi: run M5's marker test with NO trust grant → expect NOT loaded; then with the documented headless grant → loaded
# claude: in $WS ask it to create a file, three ways:
claude -p 'create file m6.txt'                                       # default → expect denied, rc 0
claude --permission-mode acceptEdits -p 'create file m6.txt'
claude --settings /ro/managed.json -p 'edit tests/x.rs'              # protected-path hook fires, exit 2 feedback
ls -l $WS/m6.txt; git -C $WS status --porcelain
```
*Reading:* for pi, the exact flag/env/file that grants trust non-interactively, and evidence that without it the marker is
absent. For claude, which mode pii must pass for edits to happen, and that the hook from a ro path fires.
*If wrong:* pi silently ignores `.pi/`, claude silently edits nothing, and both report rc 0. With no criteria, that's an `ok`
(§2.6).

## §4 What must not be claimed after tonight

- **Not** "pii runs coding agents in a container." No agent process has run under a coder node: every run refused in 1–2 ms,
  before podman.
- **Not** "the default image works." It exists, and one manual cargo run in it reproduced a failing test. No coder run has used it,
  it can't build pii's own tree (1.85 vs 1.95), and the coder's verdict wouldn't use it anyway (criteria run on the host).
- **Not** "the agent is sandboxed / blast radius is contained." Criteria execute agent-authored code on the host, `.git` is
  writable through the mount, and whether Landlock permits a container at all is unmeasured (M0).
- **Not** "refusals are visible." Only once 224 is committed and the red test is re-measured green on the committed tree.
  Until then the operator sees `no_call_events` / `fail=-`.
- **Not** "the image is digest-pinned" or "images and networks are allow-listed." The tag is accepted, the digest is recorded
  and not enforced, and `network` is free text.
- **Not** "the scaffold is secret-safe." No scaffold exists, the session files land in the tree outside `.pii/`, and there is no
  check on the diff for secret values.
- **Not** "a coder run can be resumed." M2 hasn't run.
- **Not** "criteria prove the work." They run over a tree whose tests the agent may have edited, and the table names only pii's
  own tests.

What may be claimed: the coder's refusal paths fire before any spawn and are journalled (modulo 224). Criteria are named
argv with no shell. `coder.spawned/exited` exist with digests and env *names*. And the design now has a measurement plan
whose first item could invalidate it.
