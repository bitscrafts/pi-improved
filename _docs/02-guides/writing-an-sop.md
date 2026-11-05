# Writing an SOP

An SOP is a declarative procedure: an id, a title, the capabilities it may use,
and an ordered list of steps. It **compiles into the existing graph**
(`crates/pii-graph/src/sop/`), so it runs on the same engine as every other
graph: journal, resume, `Suspend`, the capability-gated `Ctx`, and the approval
store. There is no second runner. The SOP says WHAT happens; the graph does HOW.

```
pii sop compile <file.sop.toml>      # print the graph (byte-identical on every run)
pii sop lint    <file.sop.toml>      # refuse a malformed SOP, one message per problem
pii sop show    <file.sop.toml>      # the compiled graph as ASCII topology
pii sop run     <file.sop.toml> --db <path> [--run-id <id>] [--input <json>] [--allow-write <dir>]...
pii sop resume  <run_id> --db <path> [--allow-write <dir>]...
```

Exit codes: `0` ok, `1` the run failed, `2` unreadable file / usage, `3` the SOP
(or its compiled graph) was refused, `4` the run suspended.

## The format

```toml
id = "cut-release"            # letters, digits, '_' and '-'
title = "Cut a release"
caps = ["fs_read", "fs_write", "proc"]

[[step]]
id = "prepare"                # unique; becomes the step's graph node id
kind = "act"
prompt = "Bump the version."
tools = ["fs_read"]           # optional (act): the agent's tools
max_turns = 4                 # optional (act)
deadline_ms = 86400000        # required (approve): how long it may wait for a decision
```

| `kind`    | required | what it compiles to |
|-----------|----------|---------------------|
| `act`     | `prompt` | an `agent` node; the prompt is its user message |
| `check`   | `command`| a `bash` `tool` node + a `route` on its exit code; non-zero fails the run |
| `approve` | `prompt`, `deadline_ms` | a `gate` node named after the step (policy `sop`), carrying the same `deadline_ms` |
| `notify`  | `message`, optional `path` | an `emit` node that appends the message to `path` (default `_tmp/sop-<step>-notify.log`, relative to the run's working directory) |

`requires = ["a", "b"]` names steps that must have **finished** before this one
starts. Steps run in declaration order; `requires` adds dependencies on top, so a
`requires` that points at a later step (or at itself) is a dependency cycle and
`lint` refuses it. `notify` writes to a file under the run's write scope; wiring
it to a `pii-channels` adapter is not part of this engine yet.

Strings in `prompt`/`command`/`message`/`path` must not start with `$.` (the graph
would evaluate them as expressions); `lint` refuses that.

## The compilation rule

Every step `S` becomes: `S.begin` → the step's own node(s) → `S.end` → next
step's `.begin`, with every failure path ending in `S.fail`. `.begin`, `.end`,
`.fail` (and `.await` for `approve`) are `sop` nodes: they carry no work, they
check `requires`, emit the `sop.step.*` events, and fail the run naming the
step. The compiler is pure text generation in a fixed order — the same SOP is the
same bytes. A test pins the step set and order, so a compiler change that drops
or reorders a step turns it red.

## Authority

`caps` comes from the SOP file and nowhere else. It becomes the compiled graph's
`caps`; the graph validator rejects a node whose kind needs a capability the
graph does not grant (an `act` step needs `fs_read`, `fs_write` and `proc`; a
`check` needs `proc`; a `notify` needs `fs_write`), and at run time the `Ctx` still attenuates
them, so an SOP can never widen what the process granted it.

## Preconditions refuse loudly

- A `check` whose command exits non-zero ends the run: `sop.step.failed` is
  emitted and the terminal is `Failure` reading
  `sop '<sop>' step '<step>' failed: exit code N: <output>`. Later steps do not
  start.
- A step whose `requires` names a step that has not finished fails the same way
  before it starts. (Declaration order normally guarantees this; the check is the
  backstop for a hand-edited graph or a divergent resume.)
- A human `deny` on an `approve` step fails the run with `approval denied`.

## The approval flow

An `approve` step is a `gate`: the run suspends with the usual approval key, the
approval id is run-scoped (spec 106) and re-requesting it after a lag is
idempotent (spec 88). The approval's `node` is the step id and its prompt reads
`SOP '<sop>' step '<step>': <prompt>`, so `pii approve ls` says what is being
approved. `sop.step.awaiting_approval` carries the same `approval` id.
`pii approve allow|deny <id>` works unchanged and resumes the run.

An approval has an end (spec 397): `deadline_ms` is required on every `approve` step, and `lint` refuses a
missing or zero one, naming the step. A run that waits for ever holds its queue row for ever; past the
deadline the daemon fails the run with `wait_deadline_exceeded` (naming the node and the deadline) and returns
its queue row to `pending`, session and attempts intact. It is a failure, not a denial: nobody decided.

Approvals are resolved by the daemon, so run an SOP that has an `approve` step
under it: put the compiled graph in the daemon's graph dir
(`pii sop compile x.sop.toml > <graph_dir>/sop/<id>.toml`) and start it with
`pii run start sop/<id>`. `pii sop run`/`resume` use the offline lint registry —
fine for `lint`, `check`/`notify` SOPs and for reading the events, but an
offline `act` step has no live model and an offline gate has no one to answer it.

## Events

| name | fields |
|------|--------|
| `sop.step.started` | `sop`, `step`, `kind` |
| `sop.step.finished` | `sop`, `step`, `kind` |
| `sop.step.failed` | `sop`, `step`, `kind`, `reason` |
| `sop.step.awaiting_approval` | `sop`, `step`, `kind`, `approval` |

All are tagged with the run id, so a run's trajectory reads as a procedure.

## The example

`examples/cut-release.sop.toml`: `prepare` (act) → `manifest-present` (check) →
`approve-tag` (approve) → `publish` (act) → `announce` (notify). It is exercised
end to end by `crates/pii-server/tests/sop_engine.rs` with the scripted model
`fixtures/sop/cut-release-model.json`: the run suspends at `approve-tag`, and
resumes to `Succeeded` after the approval is allowed.
