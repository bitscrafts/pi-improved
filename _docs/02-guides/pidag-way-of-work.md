# Pidag — How We Implement (the SDD pipeline)

**Date**: 2026-09-07 · **Status**: VERIFIED against pidag source (`<workspace>/<project>`) and our live config

This is the authoritative description of how pi-improved (and every project in
this workspace) turns a spec into committed, verified code. It is grounded in
the pidag source, not folklore — every claim below was read from the code.

## 1. The core idea: a static graph, not an LLM orchestrator

pidag's orchestrator is a **static DAG**, planned up front by the planner
(Claude Code / opus, human in the loop) and executed by a **deterministic
scheduler that makes no judgements at runtime**. This is a deliberate,
research-backed choice: the dominant multi-agent failure mode is the *LLM
orchestrator* degrading under information pressure (ICML 2026). pidag sidesteps
it by not reasoning during the run. See `docs/ARCHITECTURE.md` in `<workspace>/<project>`.

**Consequence**: the plan (the spec) is where quality lives. 42% of failures
are specification ambiguity; the orchestrator's job is to write specs worth
executing, and pidag's job is to execute them faithfully and prove it.

## 2. The workflow: Spec → DAG → Implement → Validate → Commit

```
Spec → pidag sdd <spec> → DAG → pidag sdd <spec> --run → review → commit
```

1. **Write the spec** (`specs/NN-<feature>.md`) — planner work ONLY. The
   workhorse never touches `specs/`.
2. **Generate the DAG**: `pidag sdd specs/NN-<feature>.md` (prints JSON, no run).
3. **Execute**: `pidag sdd specs/NN-<feature>.md --run`.
4. **Review + commit** — the planner reads the diff against the spec, then commits.

## 3. How pidag drives the worker (verified, `src/worker/pi_print.rs`)

For each LLM node, `PiPrintWorker` shells out to the local `pi` CLI:

```
pi -p --provider <provider> --model <model> --thinking low \
   --session-dir _tmp/.pidag-worker-sessions/<node_id> [-c] <prompt>
```

- **`--provider` + `--model` are passed explicitly** (spec-13/P13). pidag splits
  `provider/model` from the model string, so it does NOT rely on pi's
  `PI_PROVIDER` env. This is what lets cross-provider models work.
- **`--thinking low` by default** (env `PI_THINKING` overrides). This prevents
  runaway extended-thinking on long implement prompts.
- **`--session-dir` per node** isolates parallel workers; **`-c` (continue)**
  on iter2/iter3 resumes the prior session with full context.
- **`stdin` is closed** (`Stdio::null()`) so `pi -p` is non-interactive.
- **`kill_on_drop(true)`** — no orphaned pi processes on timeout.

## 4. Model resolution ladder: free → account → topup (verified, `src/core/config.rs`)

Priority for the effective model (per spec R4/R7):

1. CLI `--model` flag (overrides the free chain)
2. `PIDAG_DEFAULT_MODEL` env var
3. `[models]` section of `.pidag/config.toml`
4. built-in default (last resort)

The `[models]` section has `free` and `paid` lists. **`free` is tried first
(SDD iterations 1-2); `paid` is appended as fallback on iteration 3+.**

**Our cost ladder maps onto this as TWO layers, not one chain** (per AGENTS.md):

- **ACCOUNT tier = Claude Code / opus** (the orchestrator). Anthropic is billed
  by subscription (OAuth). This is the planner: spec authoring, review, commit.
  pi has NO "claude-code subscription" provider — its `anthropic` provider calls
  the paid Messages API, so anthropic never appears in the worker chain.
- **WORKER chain = pi**, free first: `nvidia → empero → ollama → deepseek (topup, LAST)`.
  pi can never use the Anthropic subscription.

**Current worker (2026-09-07, user directive): `ollama-cloud/glm-5.3-flash:cloud`**
— the free tier, fully working (no 404). DeepSeek is the topup tier, the LAST
resort, used only when the free tiers are exhausted.

## 5. The SDD loop (verified, `src/cli/sdd.rs` + workflow template)

`pidag sdd <spec> --run` generates a DAG with, per iteration:

```
implement-iterN → quality-gate-N → validate-iterN
```

- **implement-iterN**: the worker (pi) reads the spec and writes the code.
- **quality-gate-N**: runs `cargo fmt/clippy/test` (the project's
  `quality-gate.sh`).
- **validate-iterN**: runs the spec's Exit Criteria via
  `validate-exit-criteria.sh` (the `- [ ]` checkbox commands).
- The loop re-implements up to `max_iterations` (our config: 2 iterations;
  iteration 2 swaps to paid, then stop) until the exit criteria pass.

## 6. Hard rules that govern every run

- **`specs/` are READ-ONLY for the workhorse.** Authoring/editing specs is
  EXCLUSIVE planner work. If a workhorse writes `specs/`, kill it, revert, and
  hand spec-creation back to the planner.
- **The workhorse never commits.** Only the planner commits, after reading the
  diff against the spec.
- **Spec filename is strictly `NN-<slug>.md`** — two digits, hyphen, lowercase
  slug, `.md`. No dots in the slug.
- **Exit Criteria must be `- [ ]` checkbox items**, each a runnable command in
  backticks. This grammar is what `validate-exit-criteria` and `split` consume.
- **Never `rm -rf .pidag/`** — move it aside. It is the only record of a run.
- **No AI-tool mentions in commits** (neutral messages only).

## 7. Known operational traps (learned the hard way)

- **Background shells don't inherit API keys.** `~/.bashrc` is sourced only by
  interactive/login shells. A `pidag sdd --run` launched in a background
  terminal loses `DEEPSEEK_API_KEY`/`OLLAMA_API_KEY` unless the launcher sources
  the key file first. Fix: a self-contained launcher script that sources
  `~/.hermes/.env` (or `~/.bashrc`) as its first line.
- **A "Done" implement node can be vacuous.** pi can exit 0 with no tree edits;
  validate then passes against the already-green unmodified tree. Always check
  `git status`/`git diff` after a run — a clean tree means nothing landed.
- **pi's anthropic OAuth token can block everything.** An expired token in
  `~/.pi/agent/auth.json` makes pi try to refresh it on every call (even with
  `--provider deepseek`), failing all nodes. pi should never hold anthropic
  creds (anthropic is planner-only). Keep `auth.json` free of anthropic.
- **Model ids must exist in pi's catalog** (`~/.pi/agent/models.json`). A model
  not in the catalog 404s. Add new models to the catalog before configuring them.
