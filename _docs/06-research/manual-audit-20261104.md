# Findings — task 1 (USER/ADMIN manual, chapters 05–08)

Every item below was verified by running the checked-in binary (`target/debug/pii`) or by
reading the cited code. Nothing here is fixed; the editor queues it.

## F-1: `tdd` does not lint, and takes `sdd` and `night` with it
- where: `graphs/tdd.toml` (`not-red`, `unrunnable`, `still-red`), `graphs/sdd.toml`,
  `graphs/night.toml`; `crates/pii-graph/src/validate.rs` (V9)
- expected: the TDD loop — the flow the library exists for — passes `pii graph lint` and can
  be run or scheduled.
- actual: each refusal terminal declares `join = "any"` and receives one edge, so the
  validator refuses the graph. `sdd` inherits it by splicing `tdd`, `night` by splicing `sdd`.
- evidence:
  ```
  $ ./target/debug/pii graph lint graphs/tdd.toml
  V9: node 'not-red' uses a multi-arm join with 1 inbound data edges
  V9: node 'unrunnable' uses a multi-arm join with 1 inbound data edges
  V9: node 'still-red' uses a multi-arm join with 1 inbound data edges
  rc=3
  $ ./target/debug/pii graph lint graphs/sdd.toml
  V9: node 'tdd__not-red' uses a multi-arm join with 1 inbound data edges
  rc=3
  $ ./target/debug/pii graph lint graphs/night.toml
  V9: node 'sdd__tdd__not-red' uses a multi-arm join with 1 inbound data edges
  rc=3
  ```
- severity: blocks-an-operator

## F-2: `queue` never wires the `running` port its `select` node declares
- where: `crates/pii-server/src/node_kinds/nodes/queue.rs:100` (ports), `graphs/queue.toml`
  (edges)
- expected: every port a node can leave by has an edge, or the graph states what happens when
  another run holds the row.
- actual: `select` declares `ok`, `blocked`, `empty` and `running`; `queue.toml` wires the
  first three only. A `running` claim therefore leaves through a port with no edge, and the
  run stops without writing a verdict. Lint reports it as a warning (V5), not an error.
- evidence:
  ```
  $ ./target/debug/pii graph lint graphs/queue.toml
  V5: node 'select' declares port 'running' with no outgoing edge
  rc=0
  ```
  and, from the node:
  ```
  fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
      Ok(["ok", "blocked", "empty", "running"] ...)
  ```
- severity: blocks-an-operator

## F-3: `night` does not wire the refusals `sdd` can return
- where: `graphs/night.toml` (3 edges), `graphs/sdd.toml` (`unverified`, `refused` are
  `refuse` nodes), `graphs/COMPOSABILITY.md` §"The exit rule that surprises people"
- expected: a refused implementation ends the night on a named port, as `queue` does with
  `implement.unverified` / `implement.refused`.
- actual: `night` has exactly three edges (`tick.fired`, `sdd.ok`, `review.ok`). A `refuse`
  node declares no `ok` port, so the splice has nothing to return and the night stops with no
  report and no record.
- evidence:
  ```
  $ grep -c '^\[\[edge\]\]' graphs/night.toml
  3
  $ grep -n 'kind = "refuse"' graphs/sdd.toml
  79:kind = "refuse"
  86:kind = "refuse"
  ```
- severity: blocks-an-operator (the row-364 defect, moved one layer up)

## F-4: two library files cannot load at all
- where: `graphs/code-index.toml:18`, `graphs/intervention-report.toml:13`
- expected: a file under `graphs/` runs, or says in one line why it does not.
- actual: both put `name` inside a `[graph]` table and express edges as `depends_on` node
  fields. The loader reads a top-level `name`, an `entry` and `[[edge]]` triples, so both are
  refused before anything is parsed.
- evidence:
  ```
  $ ./target/debug/pii graph lint graphs/code-index.toml
  error: invalid: missing 'name'
  rc=2
  $ ./target/debug/pii graph lint graphs/intervention-report.toml
  error: invalid: missing 'name'
  rc=2
  $ ./target/debug/pii graph list | grep unloadable
  code-index · 43c9535f3e24 · nodes unloadable · - · armed 0 · same
  intervention-report · de1b1e430cce · nodes unloadable · - · armed 0 · same
  ```
- severity: lies-to-a-reader

## F-5: the installed graphs are a different revision from the repository
- where: `~/.local/state/pii/graphs/*.toml` vs `graphs/*.toml`
- expected: the daemon runs the graphs the repository reviews.
- actual: `graph list` reports DRIFT for `queue`, `spec`, `coder-selffix` and
  `discord-digest`. The installed `queue.toml` is the four-node `dispatch`/`judge` version
  dated 2026-11-03; the repository holds the three-node `call spec` version dated 2026-11-04.
  `graphs/DOCS/queue.md` describes the installed copy, so it disagrees with `queue.toml`.
- evidence:
  ```
  $ ./target/debug/pii graph list
  coder-selffix · 333b5658651b · nodes 3 · manual · armed 0 · DRIFT
  discord-digest · ce7445f0962b · nodes 6 · timer 0 12 * * * · armed 1 · DRIFT
  queue · 1fafe548188f · nodes 4 · manual · armed 0 · DRIFT
  spec · 30580132193f · nodes 10 · manual · armed 0 · DRIFT
  $ diff ~/.local/state/pii/graphs/queue.toml graphs/queue.toml | head -6
  < ... `dispatch` is a coder
  > ... `implement` calls the spec graph
  < id = "dispatch"
  > id = "implement"
  ```
- severity: blocks-an-operator (a scheduled run executes a graph nobody reviewed)

## F-6: `run ls --status` cannot name a status the store holds
- where: `crates/pii-cli/src/cmd_sys.rs:67`, `crates/pii-abi/src/journal.rs:26`
- expected: the four statuses the wire accepts cover what a run can be.
- actual: `RunStatus` has five variants (`Running`, `Suspended`, `Succeeded`, `Failed`,
  `Refused`); the filter accepts four. A refused run appears in an unfiltered listing and in
  no filtered one.
- evidence:
  ```
  $ ./target/debug/pii run ls --status refused
  error: run ls: unknown --status 'refused' (accepted: running|suspended|succeeded|failed)
  rc=2
  ```
- severity: lies-to-a-reader

## F-7: a missing `--config` file is silently replaced by defaults
- where: `crates/pii-cli/src/cmd_daemon.rs:69`
- expected: `pii daemon --config /typo/pii.toml` says the file does not exist.
- actual: `if config_path.exists() { Config::load } else { Config::default() }` — a typo'd
  path is indistinguishable from an empty config, and the operator sees `no model configured`
  (exit 5) a moment later, which points at the model rather than at the path.
- evidence:
  ```
  let cfg = if config_path.exists() {
      match Config::load(&config_path) { ... }
  } else {
      Config::default()
  };
  ```
  and the exit-code table in the same file: `5 - no model configured.`
- severity: lies-to-a-reader

## F-8: `pii store stats` is documented as safe while the daemon runs, but opens the store
- where: `crates/pii-cli/src/cmd_store.rs:29` (usage line), `:91` (`print_stats`)
- expected: the usage string marks every verb that needs the daemon stopped.
- actual: the usage says `(compact, prune: daemon must be stopped)`; `print_stats` calls
  `pii_store::Store::open(&path)`, the same exclusive open `prune` uses, so `stats` fails the
  same way against a running daemon.
- evidence:
  ```
  usage: pii store compact|stats|prune [--dir <store dir>]   (compact, prune: daemon must be stopped)
  ...
  let info = match pii_store::Store::open(&path).and_then(|s| s.info())
  ```
- severity: lies-to-a-reader

## F-9: `pii graph` usage strings disagree with each other and omit `check`
- where: `crates/pii-cli/src/main.rs:289`, `crates/pii-cli/src/cmd_graph_installed.rs:28`
- expected: one usage line per verb, listing every accepted spelling.
- actual: the group usage is `pii graph {lint|show|run|resume|trace|list|explain|edit}` and
  omits `check`, which the dispatcher accepts (`pii graph check …` exits 0). The same group
  prints `pii graph show <file.toml>` when the argument is missing, and
  `pii graph show <name> [--dir D]` in `list --help`.
- evidence:
  ```
  $ ./target/debug/pii graph
  usage: pii graph {lint|show|run|resume|trace|list|explain|edit} ...
  rc=2
  $ ./target/debug/pii graph check graphs/queue.toml; echo rc=$?
  rc=0
  $ ./target/debug/pii graph show
  usage: pii graph show <file.toml>
  $ ./target/debug/pii graph list --help | sed -n 2p
         pii graph show <name> [--dir D]
  ```
- severity: cosmetic

## F-10: chapter 04 breaks the book build with a bad TikZ arrow option
- where: `_tmp/manuals/user-admin/src/chapters/04-operating.tex:127`
- expected: the block diagram's axis arrow renders.
- actual: `\draw[-\textgreater{}, thick, gray]` puts `\textgreater` inside the option list;
  TikZ/xcolor read it as a colour name and the compile stops. `-{>}` or `->` is the
  working spelling.
- evidence:
  ```
  ! Missing \endcsname inserted.
  <to be read again> \T1\textgreater
  l.137 \end{tikzpicture}}
  ! Package xcolor Error: Undefined color `\T1\textgreater {}'.
  ```
  (Full book: `pdflatex main.tex` → 100 errors, no PDF. Chapters 05–08 alone compile clean:
  `Output written on main-check.pdf (43 pages)`.)
- severity: blocks-an-operator

## F-11: `night`'s report needs an input key nothing validates at start
- where: `graphs/night.toml` (`report.to = "$.input.to"`)
- expected: a run started without `to` says so before it spends two coder passes.
- actual: the missing key can only fail at the last node, after `sdd` has run; the node fails
  at post time, so the work is done and the report is lost.
- evidence:
  ```
  $ grep -n 'to = ' graphs/night.toml
  55:to = "$.input.to"
  ```
  `night.toml` declares no `needs = [...]`, unlike `sdd`/`tdd`, so nothing checks the input
  at load or start.
- severity: lies-to-a-reader
