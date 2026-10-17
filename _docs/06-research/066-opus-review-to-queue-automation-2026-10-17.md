# 066: The automation that turns a review into queue rows (Opus, 2026-10-17)

This is a read-only review. The only repo file written is this one, and nothing was committed. Evidence labels:
- **C-run**: I ran it today.
- **C-read**: I read the lines.
- **asserted**: from the brief.

Inputs: 064, 065, `specs/158-one-point-of-control.md`, `scripts/autonomy/plan.tsv`, the skill-lane job
`e36a2afdb65f`, `tick.sh`, `cycle.sh`, `state.py`, `/root/.hermes/scripts/ctl.sh`, and commit `5bf0db7`.

_Written incrementally._

## Verdict

**The human is not only the slow step. Measured tonight, the human is also the step that changes the meaning.** Of
the six rows converted by hand in `5bf0db7`:
- **five carry commit paths that cannot stage their own work**;
- **one drops half its spec's exit criterion**;
- **three specs say something different from the review item they came from**, one of them the opposite.

So the automation's job is not to *write* rows faster. It is to **copy** them **without paraphrase** from a block
the reviewer authored, and to **refuse** anything it cannot copy and verify. The judgement stays where it already
is, in the review. The conversion becomes a mechanical copy that can fail.

## 0. Measured: what the hand conversion produced (`5bf0db7`)

| row / spec | defect | evidence | label |
|---|---|---|---|
| A28-A33, column 6 (commit paths) | **All six rows carry the identical string** `crates/pii-learn;crates/pii-cli;_docs/05-operations`. The specs target `pii-graph` (161, 162), `pii-server` (163, 165) and `pii-net` (164); only 166 (`pii-learn`) is covered. `cycle.sh:267-283` stages **only** those paths. A worker's correct edit to `crates/pii-graph` stages nothing, `git diff --cached --quiet` is true, and the item closes as **`done-satisfied` by an older commit** (`:270-279`). The real work stays in the tree and becomes the next "leftover" commit. **Predicted for 5 of 6 rows.** | C-read (`plan.tsv`, `cycle.sh`) |
| A28 (161), column 5 (verify) | The row has `cargo test -p pii-graph --test write_amp`. The spec's exit criterion is `write_amp && cargo test -p pii-server --test boss_watch`. **Half the criterion is dropped**: the exact case `plan.tsv`'s own header forbids ("a row that drops one is a gate nobody sees"). | C-read |
| spec 161 | It says "**Flush at most once per N seconds**". 065 Q1 says: "Time-scoped. But as a deadline on the oldest unflushed event, **not as 'at most once every N seconds'**", because the rate-limit form skips the first node after a resume, which is the 064 case itself. The spec also turns 304/100 (hundredths, i.e. 3.04 commits per run) into "~100 write transactions per run". **The meaning is inverted.** | C-read |
| spec 162 | It says "a NodeProgress **event**". 065 Q3: "Progress is state, not history. **It does not go into the journal.**" A journaled progress event re-pays the write budget that 161 is about. | C-read |
| spec 163 | It says "read the verdict the gate **already recorded**". 065 Q2 measured that "**there is no recorded verdict to read today**", and that `silent-noop-check` is not a gate step. The premise is false, and the two prerequisites (R1 gate step, R2 `verdicts.jsonl`) are absent. | C-read |
| spec 164 | Its premise "the fix is in" is **true**: `027895e` (10-16 22:31), a per-connection window. This is the control case where the conversion was faithful. | C-run (`git log -p`) |
| specs 161-166 | Each has 3 sections (Overview, Exit Criteria, Guardrails). None has a files-to-modify section. So column 6 had **no source** and was typed. Spec 157 for comparison has 5 (Requirement, TDD contract, Exit criteria, Guardrails, Milestone link). | C-run |
| "11 open question(s) for you" | **This is not the skill lane.** It comes from the tick (`37ff4aecb8b0`, `tick.sh:111-118`). All 11 are "found a leftover … committed it for review" notices from 10-13 to 10-15. **0 of 11 are review items.** | C-run |
| the skill lane (`e36a2afdb65f`) | Step 4 is hard-wired: "author the next spec from the ranked list in **`059-…` section 4**". 064 and 065 are invisible to every automated reader. | C-run (`jobs.json`) |
| `ctl.sh` | It lives at `/root/.hermes/scripts/ctl.sh`, and **`/root/.hermes` is not a git repository**. The script is unversioned and ungated. | C-run |

## Q1. Where the conversion lives

**Owner: a repo script, `scripts/autonomy/plan_import.py`.** It is stdlib Python like `state.py`, versioned and
selftested with the loop. **The tick invokes it, under the cycle lock.** `ctl.sh status` *reports* it as an owned
producer. It is not a `ctl.sh` verb.

Why the other two are wrong:
- **Not the skill lane's proposal step.**
  - The skill lane is an **agent**, and an agent converting prose to rows paraphrases. Paraphrase is exactly the
    measured defect class: 161 inverted, 162 drifted, 163 based on a false premise.
  - It runs every 6 h, not every 30 min, and it is hard-wired to 059 §4.
  - Its job stays judgement: it may *author* queue blocks (Q2) when it reviews, the same as I do here. It never
    writes `plan.tsv`.
- **Not `ctl.sh plan-import`.**
  - `ctl.sh` is outside the repo and unversioned (§0). A verb that writes `plan.tsv` and `specs/` would change
    the queue from a file nobody can diff or gate.
  - 158's contract covers what can *refuse, spend, start, stop or restart* work. The queue's content is none of
    those. Its producer belongs in `status`'s inventory, as an owned scheduler line, not in its verbs.
  - 063 Q4 froze `ctl.sh`'s mutating verbs.
- **Why the tick:**
  - It is the no-agent, 30-minute, deterministic scheduler.
  - It already takes the lock that `cycle.sh` holds, so an import cannot interleave with a cycle reading
    `plan.tsv`.
  - It already owns the one phone channel through which decisions are asked (`say_once`, `questions.md`).

**Proof:**
- `git log --format=%s -- scripts/autonomy/plan.tsv` shows rows added by commits whose subject begins `plan-import:`;
- over 7 days, **0 rows** are added by any other commit, measured as rows lacking a provenance line (Q2);
- `ctl.sh status` lists `plan_import.py (tick, owned)`;
- with the lock held, `plan_import.py import` exits 75 and writes nothing.

## Q2. A row's provenance

**The reviewer writes the row. The importer only copies and verifies it.** Each importable item in a review
document is a fenced block:

````
```queue
id: 065-Q1
rev: 9fd00d2
title: the in-flight barrier is a deadline on the oldest unflushed event
class: mechanical
spec_slug: the-barrier-is-a-deadline
files: crates/pii-graph/src/journal.rs; crates/pii-graph/src/sched/dispatch.rs; crates/pii-graph/tests/deadline_flush.rs
criteria: cargo test -p pii-graph --test write_amp; cargo test -p pii-graph --test deadline_flush
evidence: crates/pii-graph/tests/write_amp.rs:87 "a run costs {per_run}/100 commits; expected ~2"
evidence: crates/pii-graph/src/journal.rs:46 "pub const DEFAULT_BATCH: usize = 16;"
body: <the item's requirement text, quoted verbatim into the spec's Requirement section>
```
````

**What is recorded:** `scripts/autonomy/plan.provenance.tsv`, a sidecar keyed by row id. A sidecar, because
`cycle.sh:89` reads `plan.tsv` positionally, and adding a column there would shift `ATTEMPTS`. Each line holds:

```
A34  _docs/06-research/065-…md  065-Q1  block_sha256=<12>  crates/pii-graph/tests/write_amp.rs:87=<sha12 of the line>  …
```

**Rules that make invention impossible:**
- **No block, no row.** The importer reads only ` ```queue ` fences. Prose is never parsed.
- **The spec is generated, not written.**
  - `specs/<n>-<slug>.md` is rendered from the block: Requirement = `body`, Files to Modify = `files`,
    Exit Criteria = `criteria`, Source = the provenance line.
  - The row's column 5 is `criteria` byte-for-byte, and column 6 is `files`. The two columns that were wrong in
    `5bf0db7` are therefore never typed.
- **An evidence line is a content anchor, not a line number.** `path:line "exact text"`: the importer requires
  that `path` exists and that line `line` contains `exact text`, and it stores the hash.

**Rules that make a stale citation fail loudly:**
- **At import:** a missing file, a line past EOF, or text not on that line is refused. If the text is found on
  another line, the refusal names it, e.g. `moved: journal.rs:46 -> :46`.
- **At pick:** `state.py pick` re-checks the row's provenance before returning it. This is cheap: file reads and
  hashes. If the block's sha changed (the review was edited), an anchor broke, or the generated spec no longer
  equals its render, then `OUTCOME=plan-error`, which `cycle.sh:60-63` already treats as "NOT an empty queue".
  The row is not dispatched, and the phone line names the row and the broken anchor.

**Proof:**
- every row from A34 on has exactly one provenance line;
- changing one character at `write_amp.rs:87` in a scratch copy makes the next `pick` print
  `plan-error: A34 evidence write_amp.rs:87 no longer reads "…"`;
- `diff <(render 065-Q1) specs/<n>-the-barrier-is-a-deadline.md` is empty.

> **Measured while writing this section (C-run):**
> - 065 cites `write_amp.rs:85` for the budget's message. At HEAD it is at **:87**, and `:85` is the `assert!(` line,
>   so that citation was wrong from the start.
> - 064's `journal.rs:46` for `DEFAULT_BATCH` is right **at HEAD**. In the working tree it reads `:46`, because A28's
>   worker has uncommitted edits in `crates/pii-graph` (`git status`; A28 is `needs-review`, `last=gate-failed`).
>
> Two rules follow:
> - **Anchors resolve at a named revision.** The block records `rev:`, and the importer reads
>   `git show <rev>:<path>`, never the working tree, so an in-flight worker cannot break or fake a citation.
> - **At pick, anchors are re-checked at HEAD**, which is when a move is real.

## Q3. Mechanical versus a decision

**Queued automatically only when all of these hold:**
1. the block says `class: mechanical`;
2. every `files` entry is under `crates/`, `xtask/`, `tests/` or `scripts/checks/`;
3. no `files` entry is on the **decision list** below;
4. every `criteria` entry is a command (it parses as a shell word list starting with an executable), not prose;
5. at least one `evidence` anchor resolves (Q2);
6. no existing row has the same `id` or the same `spec_slug`.

**The decision list.** Any match escalates, whatever `class` says:
- `deploy/`, `/root/.hermes/`, `~/.pii/`, `*.service`: production and its control;
- `scripts/autonomy/`: the loop itself, which 062 froze;
- `specs/` other than the one being generated: another item's contract;
- **budget and limit files**: `crates/pii-graph/tests/write_amp.rs`, `coverage-floor.txt`, `gate-steps.txt`
  (removal only, see below), `crates/pii-server/src/daemon/config.rs` grant constants, any `*_budget*`/`*limit*`
  test;
- anything that widens grants or egress, touches secrets, deletes, or migrates a store.

**Escalation is one permission:**
- One `control.sh ask` line per block: `queue <id> "<title>"? touches <the matched path>; yes|no`.
- `control.sh answer yes` re-runs the import for exactly that `id` with `--approved <id>`, and records the act in
  `operator-acts.log`.
- `no` marks the block `declined` in the provenance file, so it is never asked twice.

**Near-miss controls, both ways (each a fixture in Q4):**

| item | looks like | is | why |
|---|---|---|---|
| 064 D1: switch one test to the stub runner (`tests/boss_watch.rs`) | a test change, so maybe a weakening? | **mechanical** | the file is not on the list, and the criterion runs the same test |
| 065 Q1 test 1: add `assert_eq!(deadline_flushes, 0)` **to `write_amp.rs`** | a stricter test, so harmless? | **decision** | the file is a budget file. The rule judges the file, not the claimed direction, because direction is exactly what a paraphrase gets wrong (161) |
| 161's exit criterion **runs** `--test write_amp` without editing it | it names the budget, so a decision? | **mechanical** | criteria may run a budget test; only `files` may not touch one |
| 065 Q2 R1: **add** `silent-noop-check` to `gate-steps.txt` | it edits the gate, so a decision? | **mechanical** | additions to `gate-steps.txt` are allowed. The importer diffs the rendered change: lines added only |
| removing a step from `gate-steps.txt` | the same file | **decision** | the diff removes a line |
| 065 rank 1: unbind the forum ingress **in `~/.pii/pii.toml`** | a config line | **decision** | production config |
| 165's code change in `channel_wiring.rs` (bind only when a thread is configured) | production behaviour | **mechanical** | code under `crates/`; it reaches production only through the deploy card |
| a block with `class: mechanical` whose `files` include `deploy/install-pii.sh` | mechanical by label | **decision** | the list overrides the label. This is the disguise the user asked about |
| 063 rank 1: "a supervisor in this container" | infrastructure | **decision** | it has no `crates/` file at all; it is a host act |

**Proof:**
- over the fixture set, the importer's classification equals the table's last-but-one column, 9 of 9;
- flipping the `class:` label on every fixture changes **no** decision-list outcome (label-independence);
- every escalated block produced exactly one `questions.md` line and zero `plan.tsv` lines.

## Q4. How it is tested

**The harness:** `scripts/autonomy/plan-import-selftest.sh`, called from `selftest.sh`. It never touches the real
queue.
- It copies a fixture tree to `mktemp -d`: a fixture review document, fixture source files carrying the anchored
  lines, a fixture `plan.tsv` with two existing rows, an empty provenance file and an empty `questions.md`.
- The paths are passed as `PII_PLAN`, `PII_PROVENANCE`, `PII_QUESTIONS` and `PII_SPECS`, the same override pattern
  spec 153 established for `PII_CONTROL`.
- Every case asserts **the exit code, the named reason on stderr, and `sha256sum` of `plan.tsv` and `specs/`
  unchanged** for a refusal, or **exactly N lines added** for an accept.

**It must REFUSE (exit 2, plan unchanged):**

| # | fed | expected reason |
|---|---|---|
| R1 | a block whose `id` is already in the provenance file | `duplicate: 065-Q1 already queued as A34` |
| R2 | a new `id` whose `spec_slug` equals an existing spec's | `duplicate spec slug: the-barrier-is-a-deadline (specs/167-…)` |
| R3 | a block with no `evidence:` line | `no evidence: 065-Q9` |
| R4 | an anchor whose file does not exist | `evidence missing: crates/x/y.rs` |
| R5 | an anchor whose text is now on another line (the fixture inserts one line above it) | `moved: journal.rs:46 -> :46` (the case from Q2, as it would read once A28's edit lands) |
| R6 | an anchor whose text is gone | `stale: write_amp.rs:87 no longer reads "…"` |
| R7 | `class: mechanical` with `files: deploy/install-pii.sh` | **escalate** (exit 3): one `questions.md` line, plan unchanged |
| R8 | `class: mechanical` with `files: crates/pii-graph/tests/write_amp.rs` | **escalate** (exit 3) |
| R9 | a `criteria:` entry that is prose ("the run is visible") | `criterion is not a command: 065-Q3` |
| R10 | **the replay of A28:** a block whose criteria are `write_amp; boss_watch` against an existing hand row with only `write_amp` | under `--check`: `A28 verify differs from specs/161 exit criteria` |
| R11 | **the replay of the paths defect:** `--check` over the real `plan.tsv` copied in | flags A28-A32: `commit paths do not cover the spec's files` (or `spec has no files section`, the true state today) |
| R12 | the lock held (`flock` on the fixture lock) | exit 75, nothing read |
| R13 | at pick, a generated spec hand-edited after import | `plan-error: specs/167 differs from its render of 065-Q1` |

**It must ACCEPT (the controls that keep the refusals honest):**

| # | fed | expected |
|---|---|---|
| A1 | one valid mechanical block | exactly 1 row, 1 provenance line, 1 generated spec, ids continue from the last row; `state.py rows()` loads the plan |
| A2 | A1's input again | exit 0, `already queued`, 0 lines added (idempotent) |
| A3 | a block whose criteria **run** the budget test without editing it | queued (the Q3 near-miss) |
| A4 | a block adding a line to `gate-steps.txt` | queued; the same block **removing** a line escalates |
| A5 | `--approved 065-R7` after R7 | queued, plus 1 `operator-acts.log` line |

**Mutation check of the harness itself:** disable the anchor check in a scratch copy of `plan_import.py`. R4-R6
must go red and A1-A5 must stay green. A test that stays green under that mutation proves nothing.

**Proof of the whole:** `bash scripts/autonomy/selftest.sh` reports the 13 + 5 rows as PASS, and the mutation run
reports R4-R6 FAIL.

## Q5. The smallest first version that removes the human tonight

No daemon, no new scheduler, and no cargo, so the gate's budget is untouched. Six pieces:

1. **`scripts/autonomy/plan_import.py`**, about 200 lines of stdlib Python, three verbs:
   - `import <doc>…`: blocks → generated spec, row and provenance line, or a refusal or escalation;
   - `check`: validates every existing row's verify column and paths against its spec, and every provenance
     anchor;
   - `--approved <id>`.
   It runs only file reads and hashes; it never builds or runs a criterion. The criterion's red-on-HEAD run stays
   where it already is, in the cycle's verify step. A pre-dispatch red check is a version-2 item.
2. **The tick, one line before its pick step:** `flock -n "$LOCK" python3 scripts/autonomy/plan_import.py import
   _docs/06-research/*.md`.
   - On new rows it commits exactly the files it wrote, with explicit paths and the neutral identity, under the
     subject `plan-import: <id> → A<n>`.
   - On an escalation it writes one `questions.md` line, which the tick already carries to the phone.
3. **`state.py pick`** calls `plan_import.check_row(id)` and refuses a row whose provenance broke
   (`OUTCOME=plan-error`, already handled by `cycle.sh:60-63`).
4. **The skill lane's step 4** changes from "author the next spec from 059 §4" to "if the newest review document
   has an item with no queue block, write the block into that document and commit it alone". The agent writes
   blocks, never rows.
5. **The tick's question report** separates kinds. `leftover` notices go to `notes.md`, and `questions.md` holds
   only items awaiting `yes|no`. "11 open question(s)" becomes "0 decisions pending", which is true tonight.
6. **The retro case:** the reviewer (this lane) appends ` ```queue ` blocks to 064 and 065 for their items. That is
   one commit per document, by whoever owns `_docs/`. **I have not done it, since this round is read-only.**
   - By Q3's rules, regenerating A28-A33 from those blocks would touch `specs/161-166` and existing rows. That
     makes it **a decision**: one permission, "replace A28-A33 with their regenerated rows".
   - The regenerated rows fix the paths, the dropped criterion and the three meaning drifts in one act.

**Measurement that it worked (the next morning):**
- `git log --since=<tonight> --format=%s -- scripts/autonomy/plan.tsv` shows only `plan-import:` subjects;
- `python3 scripts/autonomy/plan_import.py check` exits 0 over the whole plan, or names each row it cannot vouch
  for. Tonight that is A28-A32 (R11);
- `questions.md` holds exactly the escalated blocks and nothing else;
- `selftest.sh`'s VERDICT includes the 18 plan-import rows;
- the measure that matters, **hand-converted rows from a review document since tonight: 0**, counted as rows with
  no provenance line.

## Ranking

| rank | change | proves it |
|---|---|---|
| 1 | `plan_import.py check` over today's plan, alone | names A28-A32 (paths) and A28 (dropped criterion) before the loop dispatches them. It is the cheapest change and stops the five predicted `done-satisfied` closures |
| 2 | Generated specs and rows from ` ```queue ` blocks, with provenance and content anchors (Q2) | `diff <(render id) spec` is empty; a one-character change at an anchor turns pick into `plan-error` |
| 3 | The decision list plus one-permission escalation (Q3) | 9/9 fixtures classified; label flips change no outcome |
| 4 | The tick invokes the import; the skill lane writes blocks, not rows (Q1, Q5 items 2 and 4) | 0 hand rows in 7 days; only `plan-import:` subjects on `plan.tsv` |
| 5 | `questions.md` holds decisions only (Q5 item 5) | "open questions" equals pending decisions, 0 tonight |

## NOT VERIFIED

| item | why |
|---|---|
| That A28-A32 *will* close as `done-satisfied` | predicted from `cycle.sh:267-279`. A28 is dispatched: its worker's edits sit uncommitted in `crates/pii-graph`, outside its commit paths, and it is `needs-review` after `gate-failed`. It has not yet reached the commit step |
| The skill lane's other outputs over 7 days | I read the newest one only |
| Whether `state.py pick` can import a module cheaply on every cycle | not prototyped (read-only) |
| The exact `ctl.sh status` section the producer line would sit in | I read its verbs, not its full inventory code |

Nothing in the repo changed except this file, and nothing was committed. No scratch was created this round.
