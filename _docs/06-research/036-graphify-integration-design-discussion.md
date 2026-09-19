# Graphify integration design — discussion notes (2026-09-19)

Companion to `035-graphify-code-knowledge-graph-research.md`. That document
asked *whether* to use graphify; this one answers *how* it would be wired in.
**Every mechanic below was tested on this machine**, not read from the README.
No integration is committed yet — this is the discussion input.

The four questions, answered in order:

1. Self-contained via uv, with a gitignored `.graphify/`?
2. Tooling declared in **both** `CLAUDE.md` and `AGENTS.md`?
3. A hook that makes the agent read the graph first?
4. A skill in Hermes?

---

## 1. Self-contained with uv + a gitignored `.graphify/` — YES, verified

**Isolation (uv).** `uv tool install graphifyy` puts the whole thing in its own
environment at `/root/.local/share/uv/tools/graphifyy/` with shims in
`/root/.local/bin/`. It does not touch any project's `Cargo.toml`, lockfile, or
venv. Nothing to add to `deps-check`. Self-contained by construction.

**Output location — there is a first-class knob.** `GRAPHIFY_OUT` (and
`GRAPHIFY_OUT_NAME`) override the output directory. Defined once in
`graphify/paths.py` so the security/callflow path guards honour the same
override. **Tested:**

```bash
GRAPHIFY_OUT=/projects/pi-improved/.graphify graphify extract . --code-only
# -> wrote /projects/pi-improved/.graphify/graph.json: 4187 nodes, 9646 edges, 216 communities
# -> no graphify-out/ created (confirmed)
```

`graphify/paths.py:11` documents the intent verbatim: *"override with
GRAPHIFY_OUT env var for worktrees or shared-output setups."* So it was
designed for exactly this.

**Git hooks honour it too.** `.git/hooks/post-commit` line 45:
`_GFY_OUT="${GRAPHIFY_OUT:-graphify-out}"`, with a comment noting a literal
`graphify-out/` there *"let a [worktree] ..."* (the #1423 fix). So the
auto-rebuild follows `GRAPHIFY_OUT` correctly.

**But the always-on text does NOT — this is the one real gotcha.**
`graphify/always_on/agents-md.md` **hardcodes `graphify-out/` five times**, and
the generated `CLAUDE.md` section hardcodes it too. Worse, the PreToolUse hook
message is a fixed string: *"MANDATORY: graphify-out/graph.json exists."* So if
we relocate the graph to `.graphify/`, **the instructions tell the agent to
query a path that does not exist, while the hooks (correctly) use the real one.**

Three ways to resolve it, with honest trade-offs:

| Option | How | Trade-off |
|---|---|---|
| **A. Default name, gitignored** | keep `graphify-out/`, add to `.gitignore` | Zero conflict — text and hooks agree. Directory name is not dot-prefixed, so it is visible in the root listing (our clean-root rule tolerates it if gitignored, but it is not hidden). |
| **B. `.graphify/` + patch the text** | set `GRAPHIFY_OUT`, then edit the `graphify-out/` strings in `CLAUDE.md`/`AGENTS.md` to `.graphify/` | Hidden/self-contained as you wanted, **but the patch is overwritten by the next `graphify install` or upgrade** — a standing maintenance cost, easy to forget, and the failure is silent (agent reads a stale path). |
| **C. `.graphify/` + compat symlink** | real dir `.graphify/`, plus `graphify-out -> .graphify` symlink, both gitignored | Both the hardcoded text and the env var resolve. Self-contained and hidden-ish (one symlink in root). Slight complexity; path guards honour `GRAPHIFY_OUT` so they follow it. |

**My recommendation: A for now, C if the hidden dir matters.** Option B is the
one to avoid — a silent, upgrade-reset divergence between the instructions and
reality is exactly the "check that looks rigorous while testing the wrong
surface" failure mode we have already documented twice in pidag.

**Gitignore lines for option A** (also prevents the Claude Code prompt-cache
invalidation the tool's own README warns about):

```
graphify-out/
```

---

## 2. Both `CLAUDE.md` and `AGENTS.md` — YES, and they need two different commands

Correct that both are needed: Claude Code (and **its subagents**) read
`CLAUDE.md`; other agents — Codex, Cursor, Hermes, pi — read `AGENTS.md`. The
installer splits them accordingly, verified:

| Command | Writes | Hook? |
|---|---|---|
| `graphify install --project` | **`CLAUDE.md`** (root) + `.claude/CLAUDE.md` + `.claude/settings.json` (PreToolUse) + `.claude/skills/graphify/` | **Yes** — `Bash\|Grep` → `graphify hook-guard search`; `Read\|Glob` → `graphify hook-guard read` |
| `graphify agents install` | **`AGENTS.md`** (appends a `## graphify` section, preserving existing content) | **No** — its own output says: *"unlike Claude Code, there is no PreToolUse hook equivalent for Agents — the AGENTS.md rules are the always-on mechanism."* |

Tested against a scratch `AGENTS.md`: it appended cleanly and **left the
existing content intact** — important, because our repos have substantial
`AGENTS.md` files. The section is marker-delimited (`## graphify`), so it is
idempotent and removable (`graphify agents uninstall`).

**The always-on rules it injects are the right rules** — query before reading,
`path`/`explain` for relationships, `GRAPH_REPORT.md` only for broad review,
and `graphify update .` after code changes (AST-only, no API cost).

**One caveat worth knowing:** the `Bash|Grep` matcher fires on a `grep` too, so
with the hook live, *every* grep in the project emits a "query the graph first"
nudge. That is the intended nudge behaviour, but on our workflow (which greps
constantly) it will be noisy. `--strict` is worse: it *blocks* the first raw
source read per session (`GRAPHIFY_HOOK_STRICT=1/0`). Given how much we rely on
direct file reading for verification, I would run **soft nudge, not strict**.

---

## 3. A hook to read the graph first — YES, two distinct hooks

They are **different mechanisms** and both were tested:

**(a) PreToolUse hook (agent-level, Claude Code only).** Registered in
`.claude/settings.json`. Verified output on a `Read` of
`crates/pii-worker/src/control.rs`:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":
 "MANDATORY: graphify-out/graph.json exists. You MUST run graphify before reading
  source files. ... This rule applies to subagents too — include it in every
  subagent prompt involving code exploration."}}
```

Note two things: it explicitly covers **subagents** (which is why the same
guidance must also live in `CLAUDE.md` — subagents get the file, not the
session), and it is **conditional** — it emitted *nothing* until a graph existed
at the path it checks. My first test was silent precisely because the graph was
at `.graphify/` and it was probing `graphify-out/`. That is the gotcha from §1
showing up as a real behaviour, not a theory.

**(b) Git hooks (repository-level, all agents).** `graphify hook install`
writes `post-commit` and `post-checkout` (13 KB each) and registers a
`graphify-out/graph.json merge=graphify` merge driver in `.gitattributes`. They
auto-rebuild the graph after commits and branch switches — AST-only, no API
cost. Per §1 they correctly honour `GRAPHIFY_OUT`. I installed and then cleanly
uninstalled both, and removed the `.gitattributes` line; repo verified clean.

**Honest warning about the post-commit hook in *our* workflow.** Our rule is
*workers never commit; the orchestrator commits.* A post-commit hook firing a
graph rebuild on the orchestrator's commit is fine — but it changes what a
"clean tree" means, and our pidag effect-verification seam depends on tree
state. The tool's own docs already flag the sibling problem: graph output
invalidating the Claude Code prompt cache. I would **start without the git
hooks**, using explicit `graphify update .` at deliberate moments, and add them
later only if the manual step proves to be the thing we forget.

---

## 4. A skill in Hermes — YES, already installed and loading

`graphify install --platform hermes` wrote `/root/.hermes/skills/graphify/`
(SKILL.md + 8 `references/`). `--project` variant targets
`<project>/.hermes/skills/graphify/`. I loaded it with `skill_view('graphify')`
and it is valid and available in this session.

What is actually in it (worth knowing before adopting): the skill is not a thin
wrapper — it is the **full pipeline runbook**, with Steps 0-9, an interpreter
guard, and its own honesty rules ("Never invent an edge", "Always show token
cost"). Its design intent is that the *host agent* is the LLM for semantic
extraction when no `GEMINI_API_KEY` is set — it explicitly refuses to read
`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`. Relevant to our budget policy: for a
**code-only corpus it needs no LLM at all**, which is the mode we would use.

It also has a documented fast path: if `graphify-out/graph.json` exists and the
question is a codebase question, **skip the entire rebuild and go straight to
`graphify query`**. That is the behaviour the PreToolUse hook enforces, and it
is why the skill and the hook are complementary rather than duplicate.

Note for our stack: besides `hermes` it also supports **`--platform pi`**
(`graphify pi install`) — directly relevant, since pi is our workhorse.

---

## What I would actually do (proposal for your call)

Sequenced, reversible, nothing committed yet:

1. **Keep `graphify-out/` as the name; gitignore it.** (§1 option A — no
   text/hook divergence.)
2. `graphify install --project` for **CLAUDE.md + `.claude/`** (hooks and the
   Claude/subagent path) **+ `graphify agents install` for AGENTS.md** (the
   Codex/Cursor/Hermes/pi path). Commit both — they are the shared contract.
3. `graphify install --platform hermes` + **`--platform pi`** so the skill
   travels with the harnesses we actually run.
4. **Soft nudge, not `--strict`.** Build the graph once with
   `graphify extract . --code-only`; use `graphify update .` deliberately rather
   than installing git hooks at the start.
5. **Do not wire it into the quality gate or any exit criterion.** It is a
   reader/orientation tool. §B of document 035 covers the one place it *would*
   earn gate status — upgrading `xtask reachability-check` to a real call graph.

### Decisions I need from you

- **§1: option A (default name, gitignored) or option C (`.graphify/` + compat
  symlink)?** I recommend A. B is the one I would avoid.
- **§2: accept the grep-noise of the soft PreToolUse nudge?** It will fire on
  every `grep`/`Read` in the project. If that is too loud for our
  verify-by-reading workflow, we install the skill and the always-on rules but
  skip the hook.
- **§3: git hooks on or off to start?** I recommend off, explicit
  `graphify update .` instead — because our clean-tree/effect-verification
  discipline depends on knowing exactly what writes to the repo.
- **Scope:** apply to **pii only** first, or also **pidag**? pidag is a much
  smaller tree (one crate) where the payoff is likely lower.

---

## 5. Must graphify be called at the START of a session? — NO (measured)

**Short answer: no. Nothing needs to run at session begin.** The graph is
persistent on disk; the design is *build once, query many, refresh
deliberately.* Measured on pidag:

| what | evidence |
|---|---|
| The graph persists across sessions | `graphify-out/graph.json` (3.6 MB) sits on disk; nothing rebuilds it on open |
| The hook points at a **query**, not a rebuild | fires only when `graph.json` **exists**, and instructs *"run `graphify query …` before reading source files"* — it never asks for a rebuild |
| The skill has an explicit **fast path** | *"Before doing anything else, check whether `graphify-out/graph.json` exists … **skip Steps 1–5 entirely and jump straight to `## For /graphify query`**. Do not run detect. Do not check corpus size. The graph is already built — use it."* |
| A refresh is cheap **if** you want one | `graphify update .` → **2.9 s** as a no-op (*"No code-graph topology changes detected"*), **4.8 s** with real changes |

So the correct session-begin behaviour is **do nothing** — query on demand. The
nudge exists precisely so the agent reaches for the graph *instead of* a
grep-and-read sweep, not so it rebuilds anything.

### The right moment to refresh is AFTER code changes, not at session start

That is what the tool's own design assumes: it offers a `post-commit`/
`post-checkout` hook (which we declined) **and** an explicit `graphify update .`
for the manual case. The natural trigger is "I changed code", not "I opened a
session". `update` is idempotent and cheap, so running it at session start is
*harmless* — but it is not required, and if nothing changed it is 3 s of
nothing.

### CAVEAT I hit while measuring this — `update` WIDENS the corpus

`graphify extract . --code-only` on pidag produced **3,101 nodes**. A later
`graphify update .` produced **5,098** — because update re-scanned more than
code:

```
.rs 2142   .md 1997   (none) 835   .sh 122   .toml 2
top dirs: src 1191, specs 1156, tests 1126, deploy 275, .claude 140, .orchestration 133
```

So **`update` ≠ `extract --code-only`** — it is not corpus-narrow. It pulled in
`specs/` (1156 nodes) and agent-config dirs (`.claude/` 140,
`.orchestration/` 133). Two consequences worth a decision later:

- **Good:** connecting `specs/*.md` to code in one graph is arguably *valuable*
  for us — pidag's specs are its contract.
- **Noise:** `.claude/` and `.orchestration/` (our own agent config) are not
  code and should probably be excluded.
- **Cost:** the graph grew 3,101 → 5,098 nodes; more nodes means a larger
  `graph.json` and noisier traversal.

**No LLM spend occurred** — the AST path is deterministic, and `cost.json`
records no tokens. (A `GEMINI_API_KEY` is present in the environment; the
code/structural path did not use it.)

**Recommendation:** leave session start alone; refresh deliberately after code
changes; and if we adopt the broader corpus, add an exclude for
`.claude/`/`.orchestration/` so the graph stays "the codebase", not "everything
in the tree".

### For pidag specifically

pidag's graph is **already built and persists**. Nothing runs at session open.
The first `Read`/`Grep` in the project fires the soft nudge pointing at
`graphify query` — that is the entire per-session cost.

One interaction to keep in mind: pidag's hard rule is *workers never commit;
the orchestrator commits*, and the effect-verification seam depends on tree
state. `graphify update .` writes only inside the **gitignored**
`graphify-out/`, so it cannot dirty the tracked tree or disturb that seam —
which is another reason the git-hook route was declined and the explicit
command is the right one.

**One caveat on the nudge for pidag:** the hook matches `Bash|Grep`, so it
fires on the many greps our pidag workflow already runs (spec audits, drift
checks). If that proves too loud, install the skill + always-on rules and drop
the hook — the AGENTS.md/CLAUDE.md rules still carry the guidance.
