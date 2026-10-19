# 076 — Review: skills, hooks, fragments, installed-graph verbs, graph categories

Reviewed 2026-10-19 against HEAD `ba809ff` (working tree clean). Read-only. Verdicts: ACCEPT / REJECT / AMEND.
Line refs are to the working tree unless marked `HEAD:`.

## 0. Stop first (two findings that change the order of work)

| claim | evidence | verdict |
|---|---|---|
| "Specified, committed, and NOT to be built until the review lands" | `scripts/autonomy/plan.tsv` queues A78–A81 (210–213) for the autonomy loop; HEAD `ba809ff` (titled "leftover from the previous item") already holds an unreviewed spec-210 build: `crates/pii-cli/src/cmd_skill.rs` (494 lines), `tests/skill_list_and_explain.rs`, `tests/skill_edit_keeps_the_pin.rs`, plus edits to `pii-memory/src/doc.rs` and `pii-store/src/mem/{read,write}.rs`, `tables.rs` (new `mem_evidence` table). `git show --stat ba809ff` lists 11 files, 940 insertions. | **REJECT.** The loop is building these specs right now. Pause A78–A81 in `plan.tsv` before anything else; otherwise this review lands after the code does. |
| That uncommitted build is "skills verbs only" | It also changes how **every** stored doc is decoded. `HEAD:doc.rs` `decode_kind` read `b.get(1)` and `created_of` read `b[3..11]`; `encode` writes `[kind_u8, created_u64_le, text]` (`doc.rs:159-167`). So at HEAD, kind came from the low byte of `created_ms`, and `created_ms` from shifted bytes. The working tree fixes both (`doc.rs:132-157`). Callers: `search.rs:139,143` (kind→decay λ, created→recency). | **AMEND.** This is a real, separate fix to ranking and decay for all 279 live docs. It is hidden inside a skills spec, with no test on the search path. Split it into its own spec with a regression test: a stored Skill doc decodes as Skill, and `created_ms` round-trips. Commit it first. |

## 1. Premises in the brief that are WRONG

Found WRONG (REJECT): P1, P2, P7, P8, P11, P12, plus the "19 repo graphs" count inside P6. Found partly wrong (AMEND): P3, P4, P5, P6, P9, P10. None found correct without qualification.

| # | premise (where) | evidence | verdict |
|---|---|---|---|
| P1 | "A skill is a `DocKind::Skill` memory document"; "the store and `DocKind::Skill` already exist" (210 Depends-on) | Production code never creates a Skill doc. `write_fact` hardcodes `DocKind::Atom` (`doc.rs:179`); `memory.put` goes through it (`engine/methods.rs:284`); the only `Doc::new(…Skill…)` outside tests is `skill edit` (`cmd_skill.rs:459`), and that needs a Skill doc to exist already. Before the decode fix, kind could not even be read back. | **REJECT.** The type exists but no instance does. 210's exit criterion "`pii skill list` on the live store shows `skill/manage-podman-containers`" can't be met honestly. That name looks like a *concept tag* on an Atom, so a worker will "pass" by listing Atoms tagged `skill/*`. |
| P2 | "The podman skill is now pii's main repair tool" (210 overview) | `scripts/pii-podman.sh` exists (`212d83f`). No graph names it, and no graph sets `container_image` (`grep -rln 'pii-podman\|container_image' graphs/` finds nothing). No `PII-SKILL` text exists anywhere in the repo outside cmd_skill and its tests. | **REJECT.** It is a script nothing calls. "Repair tool" is a goal, not a fact. |
| P3 | "pii installs nothing; podman is the tool (209, done)" (brief §4) | Coder `criteria` run **on the host** as `/bin/sh -c <string>` in the graph's `workdir` (`coder.rs:389-394`). The demo criteria are `cargo test …` (`graphs/coder-demo.toml`), which needs a host toolchain. No repo graph uses `container_image`, so every coder node takes the deprecated host path (209 req 1). | **AMEND.** 209 moved the *agent* into podman. The *verdict* still runs host shell taken from graph text. Say "agent spawn is podman-capable". Don't say "pii runs nothing on the host". |
| P4 | "Sop (spec 113) already proves the weaving primitive" (brief §1.2, 211, 212) | The SOP weave is a **source→source compiler**: pure text generation, no maps, no clock, same bytes on every run (`pii-graph/src/sop/compile.rs:1-8`). Its output is a graph TOML you can print (`cmd_sop.rs` `compile`), hash, lint and diff. The `sop` node itself weaves nothing (`sop.rs:1-10`). The specs propose a **load-time in-memory rewrite** that no file shows. | **AMEND.** Sop proves compile-time weaving with a printable artefact. Keep that property: weaving must produce a graph text that `pii graph show --woven` prints byte-for-byte, and the run must store that text. |
| P5 | "`pii graph` = lint\|show\|run\|resume\|trace" | `check` is a hidden alias of `lint` (`pii-cli/src/main.rs:229`). 211/212/A75 all use `graph check`. | AMEND (minor). Name `check` in USAGE, or switch the specs to `lint`. |
| P6 | "repo carries 19 → sets already diverged" | `find graphs -name '*.toml' \| wc -l` = **18** (6 top-level + 12 `starter/`), not 19. The 3 installed files are **byte-identical** to the repo copies (sha256 `ce7445f0962b`, `cff9e7983043`, `de1b1e430cce` both sides). The installer loop copies **every** top-level `graphs/*.toml` (`deploy/install-pii.sh:180-190`); `GRAPH_NAMES` (`:68`) is only the two the generated config names. So `intervention-report` is installer-shipped, not hand-copied (the earlier draft said otherwise). | **REJECT** the count of 19; **AMEND** the divergence claim. Membership differs; content does not. Real drift today is 0. The real gap is that an installed graph is not recorded as an installer or edit event, which is a provenance problem, not a hash problem. |
| P7 | User tier lives at `~/.local/state/pii/graphs` | That is the directory the **installer** writes and the daemon loads (`pii-server/src/lib.rs:80-89`, `install-pii.sh:68-69`). | **REJECT.** The "user" tier as placed is the install target, so a reinstall overwrites operator edits. The table puts system and user in the same directory. |
| P8 | "System graphs: `queue`, `code-index`, `heal`, triage" | None of them is installed on lnx (`ls /root/.local/state/pii/graphs` shows 3 discord/report graphs). The only graph that ships inside the binary today is `DEFAULT_TOML`, and a file **shadows** it: `load_spec` tries `graph_dir/<name>.toml` first and falls back to the built-in (`lib.rs:612-618`). | **REJECT.** The system tier is empty in production, and today's only built-in is overridable. That is the opposite of the proposed precedence. |
| P9 | "Today a name resolves to one file" | True within one dir, but there is already a silent dir-level precedence: `<runtime>/graphs` if it exists, else `<store-parent>/graphs` (`lib.rs:80-89`). | AMEND. That first-match fallback is the precedent the tier model would multiply. It should become a reading (§3 Q2). |
| P10 | 213: "REFUSE edit on an armed run unless `--force`" implies `--force` makes the edit take effect | Resume loads the **stored** graph body by hash (`pii-graph/src/run/resume.rs:113`). The body is stored at run start (`run/mod.rs:81-82`). | **AMEND.** An armed/suspended run never sees the edit, forced or not. `edit` must print `N armed runs continue on <before-hash> until cancelled`. Otherwise the operator believes the 1,846-run class of problem is fixed when it isn't. |
| P11 | "Hooks are documents … like skills" is cheap and safe | Consolidation archives **kind-blind**. It archives anything whose `last_used` is past the threshold (`pii-memory/src/consolidate.rs:63-66`), and it archives the lower-importance doc of any near-duplicate pair (`:81-85`). | **REJECT** for hooks (see Q4). A guard that rarely fires, or two versions of one guard, gets archived by a nightly consolidate, and the weave silently loses it. |
| P12 | 210 `edit` "keeps the id" is harmless | Ids are content hashes, and the store's invariant is "same text = same doc" (`doc.rs:1-2,84-97`). `put` inserts the new term postings but never deletes the old ones (`pii-store/src/mem/write.rs:29-35`). When the doc already exists it skips the `sum_dl` update (`:71-74`), and it re-bumps df (`:81`). The uncommitted `edit` re-puts under the old id (`cmd_skill.rs:466-471`). | **REJECT.** After an edit, search still finds the skill by its *old* words, BM25 length stats go stale, and `id_of(text) != id`. Fix: an edit makes a new content-addressed doc, archives the old one (archived, never deleted), and links `supersedes`. The *slug* is the stable handle. This also matches "previous versions move aside and are named". |

## 2. The seven questions

### Q1 — Three tiers (system immutable / user / project)?

VERDICT: AMEND — two tiers (`shipped`, `local`); project is a source, not a load tier.

**Verdict: AMEND → two tiers now (`shipped`, `local`), project is a *source* not a tier.**

| claim | evidence | verdict |
|---|---|---|
| "system" is a real, distinct category | The only graph that pii itself needs today is `DEFAULT_TOML`, embedded in the binary (`lib.rs:617`). Every "system" example in the brief is uninstalled on lnx (P8). | AMEND: a *shipped* tier is real but has one member. Make it `include_str!` graphs in the binary, which is how `default` already works. |
| system lives in "the install tree, hashed at build" | pii runs as root on lnx (`/root/.local/state/pii`). Root can rewrite both a file and its manifest hash in the same act. | REJECT the mechanism (see Q2): a hash next to a writable file is not verification. |
| user tier is per-operator | One daemon, one `graph_dir` (`lib.rs:56,80`). The operator is root. The installer writes the same dir (P7). | AMEND: `local` = the one graph_dir. The installer writes it **only through `pii graph edit`**, so each install becomes a recorded edit with `.prev-<epoch>`, never a raw `cp`. |
| project tier `<repo>/.pii/graphs` | The daemon has no notion of a project root. `queue.toml` "for THIS project" would need the daemon to load from N repos, which is a second registry by stealth. | REJECT as a load tier. A repo is where graph *bytes come from*. Install with `pii graph edit <name> --file <repo>/graphs/x.toml`, and record `source=<repo>@<commit>` as provenance. |

**What would settle it:** list the graphs that (a) pii needs in order to function and (b) would cause harm if an operator edited them. If, after a coder node and the queue graph have run in production on lnx (§Q7), that list is still just `{default}`, then tiers are bureaucracy. If `queue`/`heal` join it, they move into the `shipped` tier.

**Smallest spec change:** add §"Tiers" to 213. Two sources: `shipped:<name>` (in the binary) and `file:<graph_dir>/<name>.toml`. `list` gains a `source` column. Installer installs via `graph edit`. No third directory.

### Q2 — Resolution when a name exists twice; how list/explain PROVE which file ran

VERDICT: REJECT precedence — a duplicate name is a load refusal; provenance comes from the run journal, not the directory.

**Verdict: REJECT precedence; a duplicate name is a load REFUSAL. Proof comes from the journal, not the directory.**

| claim | evidence | verdict |
|---|---|---|
| "system > project > user" resolution | Any precedence order means *which one ran* depends on directory state at fire time. The 1,846-run incident is that failure class (`_docs/03-reference/skills-and-hooks-in-pii.md` §4, §8.3). | REJECT. A `local` file whose name equals a `shipped` name → `graph.name.shadowed`, refused at daemon start and by `lint`. To change a shipped graph, copy it under a new name. |
| `list` shows a `shadowed` marker | With refusal there is nothing to mark. | REJECT (made moot). |
| "immutable must mean verified, not unwritable" | Correct principle. A build-time hash compared to an on-disk file checks one root-writable thing against another. | ACCEPT the principle, AMEND the mechanism. Shipped graphs live **inside the binary**, so the binary's own version/hash is the verification. `explain` prints `source: shipped (pii <version>, sha256 <x>)`. |
| how to prove which file a run used | Already half-built. Every run stores the exact loaded spec JSON content-addressed and journals its hash (`pii-graph/src/run/mod.rs:81-99`; `RunEvent::Started`). Resume reloads that body (`resume.rs:113`). | AMEND: extend `RunEvent::Started` with `source` (`shipped:<name>@<ver>` \| `file:<abs path>`), `file_sha256` of the bytes read, and `woven` = list of `(attach-id, fragment, fragment_sha256)`. `pii graph explain --run <id>` renders from the journal + stored body, never from today's directory. |
| one hash identity | There are already **two** graph hashes: blake3 of canonical JSON (`engine/learn_methods.rs:49-57`, spec 139 promoter) and the store key `hash(spec_json)` (`pii-store/src/lib.rs:301-302`). 212/213 add a third: sha256 of file bytes. | AMEND: name them and print them with distinct labels. `file_sha256` = which bytes (pins, drift, `.prev`). `spec_hash` = which behaviour (store key, promoter). A cheap worker given only "sha256" will compare the wrong pair. |

**Smallest spec change:** 213 req 1 gains `source` and both labelled hashes. A new 213 req 7: "`explain --run <id>` reads `Started.{source,file_sha256,woven}`; a run without them prints `provenance: not recorded (pre-213 run)`."

### Q3 — Weaving vs event bus; at what scale does weaving become unreadable, what breaks first?

VERDICT: AMEND — weave only `run.start` and `node.before`; `run.end` becomes an engine terminal step; `node.after` and `gate.refuse` are dropped. No event bus.

**Verdict: AMEND. Weave PRE surfaces only. Readability is not what breaks first; edge semantics break at N=1.**

What breaks first, in order. Each item is reachable with today's 3–6-node graphs (`grep -c '^\[\[node' graphs/*.toml` → max 6):

| # | breaks | why | smallest fix |
|---|---|---|---|
| 1 | **`run.end` on failure** | Woven as "the last node", it runs only on paths that reach that node. A failed or cancelled run never gets there, which is exactly when an end hook matters. | `run.end` is **not woven**. It becomes one engine-owned terminal step run on every terminal disposition (ok/failed/cancelled), inside the same run and budget, journalled `hook.fired`. Or drop it from v1. |
| 2 | **`node.after` port routing** | Nodes route on many ports (`ok/err/escalate/inconclusive/flake/detected/allow`, reference doc §1). An "observe node on the outgoing edge" must re-emit the *same* port to the *same* target. A worker will wire only `ok`, which silently breaks `err` routing. | Drop `node.after` from v1. The journal already records every node finish; an observer is a journal query, not a node. |
| 3 | **`node.before` on a join / multi-inbound node** | One woven node per inbound edge fires the hook k times. One merged node moves the node's `join` policy (`spec.rs:137`) onto the hook. | Spec it: one woven node per target node. It inherits the target's `join`, and the target becomes `join=any` from that single edge. Add a test with a 2-inbound `join=all` node. |
| 4 | **retry and loops** | A hook woven before a node with `retry`/`loop_budget` (`spec.rs:139-141`) fires once per activation, not once per attempt, and it spends loop steps. | State it: once per activation. The woven node counts against `max_steps` (`spec.rs:99`, default 1000). |
| 5 | readability | `on=["*"]` × a k-node fragment × N nodes = N·k woven nodes. | Lint refuses `on=["*"]` at `node.*` surfaces; `*` is allowed only at `run.start`. Lint refuses `woven > written` (`fragment.weave.dominates`). |

There is no honest node-count threshold. The limit is a **ratio**: once woven nodes outnumber written ones, the file no longer describes the run, and Q2's provenance becomes the only truth. Hence the `woven ≤ written` lint.

Event bus: REJECT for PRE (blocking needs to be in the graph). For POST, the bus already exists: it is the journal. No new mechanism is needed.

**Smallest spec change:** 211/212 surfaces shrink to `run.start` and `node.before` (woven), plus `run.end` (engine terminal step, if kept). `node.after` and `gate.refuse` are removed.

### Q4 — "A skill is prose OR a subgraph": one concept or two?

VERDICT: REJECT the single concept — two concepts (skill: recalled prose; fragment: pinned, explicitly attached graph). A hook is a fragment attached at a PRE surface, not a third kind.

**Verdict: TWO. And a hook is not a third: it is the second one attached at a PRE surface.**

| name | is | selected by | lives in | executes? | for |
|---|---|---|---|---|---|
| **skill** | `DocKind::Skill` memory doc: instructions, templates, snippets, pins to scripts | **recall** (bm25/vector/graph lanes): fuzzy, decaying | memory store | never by pii's loader. A coder agent may *read* it. | telling an agent how to do something |
| **fragment** | graph TOML under `graphs/fragments/`, `[fragment]` header, pinned | **explicit `[[attach]]`** by name+pin: deterministic | repo → installed via `graph edit` | yes, as woven nodes in the host run | doing something the same way every time |

Why they cannot be one: recall is non-deterministic, and memory consolidates kind-blind (P11). Anything the loader executes must be chosen by a pinned name, never by search. A skill may *describe* a fragment (`fragment: <name> sha256=<hex>` line, shown by `skill explain`), but it never causes one to run.

**Hooks:** REJECT `DocKind::Hook`. A "hook" is an `[[attach]]` of a fragment with `may_block=true` at `run.start`/`node.before`. 211's audit verbs come from 213 (`graph explain` lists attaches) plus a `pii fragment list|show|explain` that reuses 213's code. This removes a second store for control-plane, and it removes 211's two internal contradictions (§Q6).

**Smallest spec change:** 210 drops `body: subgraph`. 211 is rewritten as "hooks are fragment attaches" and merged into 212. The reference doc §8.2 recommendation is withdrawn.

### Q5 — What must never be allowed in a user-tier (local) graph?

VERDICT: AMEND — a graph may request capabilities, never grant them; grant-set changes come from daemon config, and `graph edit` refuses widening.

**Verdict: a graph may REQUEST capabilities; it may never GRANT them. Today it grants itself.**

Measured: `GraphSpec.caps` is declared in the graph file itself (`spec.rs:153-155`; `coder-demo.toml: caps = ["spawn","proc"]`). V2 only checks that node caps ⊆ graph caps (`validate.rs:77-92`), i.e. internal consistency. The same file also carries `agent`, `model`, `workdir`, `env_pass`, network grant, `container_image`, and `criteria`, which are **host shell strings** (`coder.rs:389-394`).

| field in a graph | configuration (allowed to edit) | escalation (must be refused unless the daemon config already grants it) |
|---|---|---|
| `criteria` | choosing among commands in a config allow-list | **any free-form shell string.** Today this is arbitrary host command execution from a TOML edit. |
| `caps` | subset of a ceiling held in daemon config | any bit outside that ceiling |
| `env_pass` | names on a config allow-list | any other name (e.g. a provider API key handed to an agent = exfiltration) |
| network grant | `none`, or a named grant from config | free-form `--network` values; `host` |
| `container_image` | digest-pinned image on a config allow-list | tag-only / unknown image |
| `workdir` / mounts | paths under a config allow-root | anything else; `forbidden_mount` (`coder.rs:196`) is a deny-list, so invert it |
| `agent` / `model` | entries in config | anything else |
| memory writes | own layer/namespace | writing `Skill` docs. That plants instructions that *other* graphs' agents will recall: prompt injection across runs. |
| timers | cadence ≥ a config floor | sub-floor cadence; >1 armed run per graph without `max_armed` |
| attaches | fragments installed via `graph edit` | a fragment that adds caps the host lacks (V2 must run on the **woven** spec; the weaver must never widen `spec.caps`) |

**The line:** anything that changes the *grant set* is privilege, and it must come from a file the graph author cannot change in the same act: the daemon config, edited separately and journalled. `graph edit` diffs CAPABILITIES before→after and refuses widening (`graph.edit.escalation`). `--force` does not override it.

**Smallest spec change:** 213 req 4 gains "f. refuse if the new file's effective grant set (caps ∪ env_pass ∪ net ∪ image ∪ criteria ∪ workdir) exceeds the old one's, naming each addition". A new spec, before any of 210–213 ships: "criteria are names from config, not shell".

### Q6 — What a cheap worker would implement wrongly from specs 210–213 alone

VERDICT: AMEND — 210–213 contain two self-contradictions (211:40 vs 211:69-77; 211:37-38 vs 211:77) and several unstated choices; fold the fixes in before any build.

| spec | gap / contradiction | evidence | what the worker will do | fix |
|---|---|---|---|---|
| 210 | no production writer of Skill docs | P1 | fake the live criterion by listing Atoms tagged `skill/*` | add req: `pii skill put --file <f> --slug <s>` writes `DocKind::Skill`, L3; exit criterion uses it, then `list` |
| 210 | "keeps the id" vs content-addressed ids | P12; `write.rs:29-35,71-74` | re-put under old id → stale postings, df/dl drift | edit = new doc + archive old + `supersedes` link; slug is the handle |
| 210 | tests seed the store directly with `DocKind::Skill` | `tests/skill_edit_keeps_the_pin.rs:30` | green tests that never touch the decode path that was broken at HEAD | one test must write via the CLI, reopen the store, and read back kind=Skill |
| 210 | consolidation archives skills | `consolidate.rs:63-85` | skills vanish from `list` after a nightly consolidate; "variants as separate docs" (ref §7.3) are near-duplicates → merged | req: consolidate skips `Skill` (or lists what it archived by kind); test it |
| 210 | pin path is relative to what? | `cmd_skill.rs` `current_root()` (`:139`) = CWD | `ok` from repo root, `MISSING FILE` from anywhere else | pin paths are repo-relative to a configured root; `explain` prints the root used |
| 210 | "refuses drop of a pin" compares paths only | `cmd_skill.rs:446-450` | an edit may keep the path but change `sha256=` to match a tampered file — a silent re-pin | print old→new hash per path; changing a hash is allowed only with `--repin` and is journalled |
| 210/211 | "a DRIFT skill may not be used by a node" — no node uses skills | ref doc §1 | invent a consumer, or ship an unenforced rule | strike until a consumer exists; Q7 |
| 211 | **contradiction**: "Loading hooks never mutates the graph … not edges" (`211:40`) vs "woven into the graph at load time as nodes" (`211:69-77`) | spec text | implements whichever section it read last | delete `211:40` (or all of 211, per Q4) |
| 211 | **contradiction**: drift → "inert … refused at fire time" (`211:37-38`) vs "fails at LOAD, not at fire" (`211:77`); 212 says graph refused (`212:26-27`) | spec text | three behaviours across two specs | one rule: drift refuses the **graph** at load (`fragment.pin.drift`), never a silent skip |
| 211 | which node kind *runs* the pinned script? its caps, timeout, network? | `sop` runs nothing (`sop.rs:1-10`); V10 requires a process-spawning node to bound itself (`validate.rs:95+`) | a new `/bin/sh -c` on the host — the exact thing ref §4 forbids | name it: the woven node is a `coder`-family container run (`--network none`, digest image, timeout) or the fragment's own declared nodes; never host shell |
| 211 | `when: <predicate>` grammar unstated | `pii-graph/src/expr.rs` exists | a new ad-hoc parser | `when` uses `expr.rs` grammar, evaluated at load where static, else refused |
| 212 | weaver vs V2 | `validate.rs:77-92` | "fix" a V2 failure by OR-ing fragment caps into `spec.caps` | req: weaving never alters `spec.caps`; V2 runs on the woven spec |
| 212 | which hash is `pin` | Q2 | canonical-JSON hash (spec 139) vs file bytes | `pin` = sha256 of fragment file bytes, stated |
| 212 | `on = [ids]` naming a nonexistent node | — | silently matches nothing | `fragment.target.unknown` refusal |
| 212 | namespaced ids `<attach>.<node>` may collide with an operator node literally named `guard.x` | — | duplicate-id panic or overwrite | lint refuses written ids containing `.` or the collision |
| 212 | where fragments are installed | `resolve_graph_dir` has no fragments subdir | load from repo `graphs/fragments/` at runtime (repo ≠ installed) | `<graph_dir>/fragments/`, installed via `graph edit`, listed by 213 |
| 212 | the user's sentence was a question (brief §1.3) but A80 is queued | `plan.tsv` | builds it | mark A80 blocked pending decision |
| 213 | edit vs armed runs | P10 | promise `--force` makes the edit live | print "N armed runs continue on `<before>`" |
| 213 | installer bypasses `edit` | `install-pii.sh:68-69` | `edit` audits; `install` still `cp`s | installer calls `pii graph edit` |
| 213 | "drift vs the repo copy" — which repo, which commit, on a host with no checkout? | daemon has no repo root | hard-code `/projects/pi-improved/graphs` | `--repo <dir>` explicit; absent → `drift: unknown`, never `same` |
| 213 | edit while daemon runs: does the daemon re-read? | `load_spec` reads per fire (`lib.rs:612`) | assume restart needed (or not) | state: next *new* run reads the new file; journal `graph.edited{before,after}` from the CLI |
| all | `--json` only promised in 213 | — | four different text formats | one `--json` row schema for list across skill/fragment/graph |

### Q7 — What must NOT be claimed until a coder node has run in production on lnx

VERDICT: ACCEPT — each claim below stays unmade until the named lnx journal reading exists.

| do not claim | because | the reading that would permit it |
|---|---|---|
| "pii can use a skill" / "skills work" | no Skill doc exists in production (P1); no node consumes one | a production journal where a coder run's brief names a skill id + `PIN ok` |
| "pii installs nothing / runs nothing on the host" | criteria are host `/bin/sh -c` (P3); no graph sets `container_image` | a lnx run journal with `podman run` argv, image digest, and criteria executed in-container |
| "the podman skill is the repair tool" | no graph invokes `scripts/pii-podman.sh` (P2) | a heal run on lnx whose journal shows the probe verb fired |
| "hooks can block" / "guards run on every graph" | no hook runner exists; v1 surfaces unspecified (Q3, Q6) | `run.refused{hook=…}` from a real armed run |
| "the queue is a graph that dispatches work" | `queue.toml` not installed on lnx (P8); 14 green tests are local | one queue item selected → coder → judged → closed, on lnx, in the journal |
| "system graphs are immutable" | no system tier exists; the only built-in is file-shadowable (`lib.rs:612-618`) | a refused shadow attempt (`graph.name.shadowed`) on lnx |
| "installed graphs have drifted" | 0 content drift today (P6) | a `pii graph list` row with `DRIFT` |
| "edit stops a runaway schedule" | armed runs keep the stored body (P10) | an edit + cancel sequence on lnx with the journal showing the old hash stop |
| memory/search quality numbers taken before the decode fix | kind and `created_ms` were mis-decoded at HEAD (§0) | re-measure after the fix lands |
| 0.1.1 | gated on five production readings, none exist (brief §2) | those readings |

## 3. Invariant check (one run · one budget · one store)

| proposal in 210–213 / brief | respects invariant? | note |
|---|---|---|
| `DocKind::Hook` in memory | **no** — a second control-plane store inside the knowledge store, subject to decay/merge | REJECT (Q4) |
| three load directories | **no** — a registry per tier | AMEND to one dir + binary (Q1) |
| fragments woven into the host run | yes, if budget/caps are host's and V2 runs on the woven spec | ACCEPT with Q6 fixes |
| `run.end` as an engine terminal step | yes — same run, same budget, same journal | ACCEPT (Q3) |
| `mem_evidence` new table (in `ba809ff`) | borderline — evidence already exists as `Doc.evidence` (`doc.rs:51-52`) but is not encoded by `encode` (`doc.rs:159-167`) | AMEND: persist `Doc.evidence` in the doc codec or say why a side table; don't leave two evidence fields |

## 4. Recommended order (replaces A78→A81)

1. Pause A78–A81 in `plan.tsv`; keep the 210 build at `ba809ff` as-is for review (do not revert or discard).
2. **Decode fix** as its own spec + regression test (§0).
3. **Criteria are named, not shell** + grant ceiling in daemon config (Q5). Highest risk, smallest change.
4. **213** with Q2 provenance (`source`, two labelled hashes, `explain --run`), armed-run wording (P10), escalation refusal (Q5), installer via `edit`.
5. **210** amended: `skill put`, new-id-on-edit, consolidation exemption, `--repin`.
6. **212** (only if the user answers its question yes) with surfaces `run.start`, `node.before`; hooks = attaches; 211 withdrawn.
7. Production readings (Q7) before any claim or 0.1.1.
