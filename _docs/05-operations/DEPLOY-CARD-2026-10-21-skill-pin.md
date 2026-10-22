# Deploy card — the skill-pin reading (spec 225 c6 / spec 227 c7, rows A94 and A95)

**Status: NOT RUN.** This card pre-registers the reading; it records nothing yet. The deploy it describes
restarts the live daemon on the lnx host, so it waits on two things that are an operator's to give: the switch
in `_tmp/autonomy/control` (currently `PAUSED`, written 2026-10-20T18:11:53Z) and authorization for a host action.
Written at HEAD `85389e7`; the deploy script rebuilds at whatever HEAD is when it runs and prints that revision.

## Why one reading closes two rows

Spec 225's criterion 6 and spec 227's criterion 7 are the same observation, stated once in each closure:
a coder run whose skill pin matches the stored document reaches a spawn and journals the pin, and the same graph
refuses `skill.drift` once the document is edited. Everything behind it is landed — `pii skill add` and
`pii skill edit` reach the daemon's store (specs 233/234), the registry hands the coder node a `StoreSkills`
source (spec 227 R2), and a confined process starts a container through `podman.socket` (spec 230, M0 PASS on lnx).
What is missing is not code: it is one deploy of HEAD and the two runs below.

## Preconditions

- `git status --porcelain` in this tree is empty.
- The operator switch is off (`truncate -s0 _tmp/autonomy/control`, or however the operator lifts it) **and** the
  deploy is authorized: it stops and restarts lnx's daemon.
- The skill document to store has a `PII-SKILL` marker line; without it `pii skill add` refuses
  (`the file has no PII-SKILL marker line, so it is not a skill`).
- No second coder run is in flight on lnx (`pii run ls --status running`).

### Credential path — added 2026-10-22, read before step 3

A commit landed after this card was written: `269bc39` "coder: keep the provider key off the pi argv". The coder no
longer puts `--api-key <value>` on the agent's argv (the value is readable in `/proc/<pid>/cmdline`); the key has to
reach `pi` through the environment now — a named graph `provider`, `env_pass` carrying the key variable, and
`PI_CODING_AGENT_DIR` pointing at a directory whose `models.json` holds `apiKey = "$ENV:<NAME>"` (the commit
records the measurement: `pi` exits 0 via ollama-cloud with no `--api-key`).

Step 3's template below predates that commit. It names no `provider` and passes `env_pass = ["PATH", "HOME"]`.
Copied as it stands the run cannot authenticate, and it fails **before** the skill pin is ever checked — the reading
is lost and the deploy has to be spent a second time. Two readings in this tree settle the shape, and neither
touches the host:

- `crates/pii-server/src/node_kinds/nodes/agent_worker.rs:273` refuses a `pi` dispatch that names a provider whose
  key is absent from its env (`no credential for provider <p> in the environment`). A graph that names no provider
  skips that refusal and lets the agent fall back to its own stored route — the silent failure the check exists to
  prevent, and the forbidden Anthropic route on this path.
- `graphs/coder-selffix.toml` is the graph measured green after `269bc39`: it names `provider = "ollama-cloud"` and
  passes `PI_CODING_AGENT_DIR` by name. Step 3's graph runs inside `localhost/pii-agent:1` with the workspace at
  `/work`, so whatever path `PI_CODING_AGENT_DIR` names must exist **inside that container** and hold a
  `models.json`. pii mounts the per-run skills at `/work/.pi/skills` but writes no `models.json`
  (`grep -rn 'models.json' crates` returns comments only), so that file is the operator's to place or the image's
  to ship.

Treat step 3 as a shape to adapt from the selffix graph's credential fields, not as a file to copy verbatim.

## Steps and what each one must print

### 1. Install HEAD and restart the daemon

```bash
bash scripts/deploy-lnx-test.sh
```

It builds `target/release/pii` here, `mv`s it into place on lnx (a plain `scp` onto a running binary fails),
places the graphs under `/run/user/1000/pii/graphs`, stops the daemon **by env-matched pid** and starts it again,
then prints `READING build:`. That line must name the revision this card names, not the previous install — a stale
revision means the daemon kept the old binary and everything below measures the wrong tree. The script's own
`coder-podman` run follows; it is a liveness probe here, not the reading.

### 2. Put the skill in the store the daemon reads

```bash
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii skill add --file ~/SKILL.md'
```

Expected, and both lines matter:

- stdout `added <id> <slug> sha256=<hex>` — that `sha256` is the pin the graph must carry;
- stderr `via daemon`.

A `via store <path>` line means the command wrote the store file directly instead of going through the daemon
(the daemon was not serving, or the socket was not the one the daemon holds). That is **not** a pass for this
reading — it would prove the store half while leaving the daemon half unmeasured. Stop and find out why the
daemon did not answer before continuing.

A refusal naming `already live as <id>` is also a stop: add refuses a slug that exists, and the reading wants the
pin printed by a fresh put. Use `pii skill edit` on that id only if the existing document is the one this reading
is about.

### 3. Write the pinned graph

Copy `graphs/coder-podman.toml` to `graphs/coder-pinned-skill.toml`, change `name`, keep `kind = "coder"`,
`caps = ["spawn", "proc"]`, `env_pass`, and add the pin — the schema the node parser reads
(`crate pii-server`, `node_kinds/nodes/coder_skills.rs:89`) is a list of `{ name, sha256 }`:

```toml
name = "coder-pinned-skill"
entry = "code"
caps = ["spawn", "proc"]

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "pi"
model = "deepseek-v4-flash"
workdir = "/home/mvcorrea/pii-work"   # must exist on lnx and be a path the sandbox grants (spec 232); coder-podman.toml uses /tmp
spec = "SPEC.md"                      # read relative to workdir, so copy the spec beside it first
brief = "Implement the spec named above. Follow it as the contract."
container_image = "localhost/pii-agent:1"   # the image the M0 run used on lnx (row A98); the runtime records its digest
env_pass = ["PATH", "HOME"]
skills = [{ name = "<slug from step 2>", sha256 = "<hex from step 2>" }]

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "transform"
value = "refused"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
```

Place it where `run start` resolves graphs, and confirm it is seen before running it:

```bash
ssh -o BatchMode=yes mvcorrea@10.10.10.217 'cp ~/coder-pinned-skill.toml /run/user/1000/pii/graphs/'
ssh -o BatchMode=yes mvcorrea@10.10.10.217 'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii graph list'
```

`graph list` naming `coder-pinned-skill` is the control that the run below is not resolving some other file
(`run start` resolves through the same path — spec 222).

### 4. The pin half — a matching pin reaches the spawn

```bash
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii run start coder-pinned-skill'
# then, with the run id it prints:
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii run trace <run_id>'
```

The accepted outcome, quoted from the live path (`coder.rs:335` builds the refusal; the spawn event is
`names::CODER_SPAWNED`, one JSON object per line in the daemon's event log):

- `coder.spawned` present, carrying `skill=<slug>` and `sha256=<the pin from step 2>`;
- `coder.exited` for the same pid afterwards;
- the run reaches its `done` terminal.

Where to read that line: the daemon writes each event as one JSON line to `[core] log_file` when a log file is
configured (`pii-cli/src/daemon_log.rs:25`), otherwise it goes to the daemon's own stdout — on lnx that is
`~/pii-daemon.log`. `pii run trace` prints per-node status, pid, image, criterion counts and the failure name, but
**not** the skill fields, so the pin is read from the event line, not from the trace:

```bash
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'grep -h coder.spawned ~/pii-daemon.log | tail -2'
```

The materialised file is per run, under the daemon's runs dir — `<runs>/<run_id>/skills/<slug>/SKILL.md`, with a
`sha256=<hex>` header line — and is bound read-only over `/work/.pi/skills` and `/work/.claude/skills`. Nothing is
written into the project.

### 5. The drift half — an edited document refuses before anything spawns

```bash
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii skill edit <id> --file ~/SKILL-edited.md --note "drift probe"'
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii run start coder-pinned-skill'   # still pinned at the OLD hash
```

Expected: stderr `via daemon` from the edit (same rule as step 2), then a run whose terminal is the `refused` node.
The refusal is journalled as a `skill.drift` event carrying `name`, `pinned` and `found` (one JSON line in the same
event log), and the node's reason, which `pii run trace` prints, reads

```
skill.drift: <slug> pinned <old hex> found <new hex>
```

with **no `coder.spawned` for that run** — the refusal precedes the spawn. `found <new hex>` is the pin for the
last step; take it from this line rather than hashing the file by hand, because the daemon stored the text.

Two readings that are not this outcome and do not count as a pass:

- `found missing` — the store does not hold the skill under that name; step 2 measured something else
  (a different runtime dir, or a slug the resolver does not pick uniquely).
- a run that starts a container and then fails inside it — the drift check is before the spawn; a container
  running means the pin was not checked.

### 6. The new pin runs (spec 234's criterion, same deploy)

Set `skills[0].sha256` to the `found` hash from step 5, replace the graph file, and run it again: expected
`coder.spawned` with `sha256=<new hex>`. This is the half that proves the edit did not merely break the old pin.

## What gets recorded, and where

- This card's `## Result`, with the revision from step 1, the two run ids, the `coder.spawned` line, the
  `skill.drift` line and the `graph list` line — pasted, not paraphrased.
- The same reading in `specs/225-project-scaffold-and-per-run-skills.md` and
  `specs/227-the-coder-gets-its-skills-from-the-store.md`, in `## Closure`.
- State rows A94 and A95 move to `done` only with all of the above present. A green unit test is not this reading:
  the fake-podman shim in `skills_are_materialised_read_only` is coverage of the mount plan, and a mock answers
  what you send it.

## Result

_(empty — nothing in this card has been run; it is the pre-registration, not a reading.)_

## Stop conditions

- The daemon does not come back (`control.sock` missing, `pii health` error) — stop; do not retry the install with
  a different pid-matching rule by hand.
- `via store` anywhere — stop; the daemon half is unmeasured.
- The image probe answers `image=missing` for a present image — that is spec 231's shape and a separate defect;
  record it as a finding instead of pinning around it.
- A run that refuses with `no credential for provider ...`, or that reaches the agent with no credential at all —
  that is the credential path ("Credential path" above), not the skill pin. Fix the graph's `provider`/`env_pass`
  and re-run; no part of that run counts as this reading.
- Any temptation to weaken the graph (`skills` pin removed, criterion loosened) to get a green: that is the
  forbidden move, and a worker that stops to flag a wrong premise is behaving correctly.
