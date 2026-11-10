# Spec: installed graphs can be listed, edited and explained

## Overview
User directive 2026-10-19: "the pii also must be able to list/edit and explain its installed graphs". Today `pii graph` offers
`lint | show | run | resume | trace` — there is no way to LIST what is installed in the runtime graphs directory, no way to ask
what a graph actually does, and no controlled way to change one: editing means opening a TOML in an editor and restarting the
daemon, with no lint, no hash, no previous version and no record of what changed. The installed set on the host right now is
`discord-digest.toml · discord-triage.toml · intervention-report.toml`, while the repo carries nineteen — so "which graphs are
live, and do they match the repo?" is currently answered by `ls` and memory.

**Project**: <repo>

## Verified premises
- `pii graph` exists with five subcommands; `list` and `edit` are absent.
- installed graphs live in the runtime graphs dir (`~/.local/state/pii/graphs`), the repo copy in `graphs/`.
- the installer already copies a named subset (`GRAPH_NAMES=(discord-triage discord-digest)`), which is how the two sets diverged.
- spec 176/177 require a graph that cannot load to REFUSE loudly, and spec 212 requires woven nodes to be printed with origin.

## Requirements
### Functional
1. `pii graph list [--all]` — one line per INSTALLED graph: name · sha256 (first 12) · nodes · trigger (`timer 0 9 * * *` or
   `manual`) · armed runs · drift vs the repo copy (`same` | `DRIFT` | `repo-only` | `installed-only`).
2. `pii graph show <name>` — the installed TOML verbatim.
3. `pii graph explain <name>` — rendered FROM the file, never paraphrased:
   TRIGGER · the node chain in dependency order with each node's kind and ports · EFFECTS (what it writes: post, memory,
   store) · CAPABILITIES it needs (channel names, net grants, agent+model) · woven fragments (spec 212) with origin · the
   drift verdict and the hash of the file that was read.
4. `pii graph edit <name> --file <path> [--force]` — the only supported way to change an installed graph:
   a. LINT the new file first; a file that fails `pii graph lint` is refused and nothing is written
   b. REFUSE if the graph has an armed/suspended run unless `--force` is given, naming the run ids
   c. MOVE the previous file aside as `<name>.toml.prev-<epoch>` (never delete) and print both hashes
   d. write atomically (temp + rename), then re-read and re-lint the installed copy
   e. report `before-sha → after-sha` and the drift state against the repo
5. `pii graph explain` must state when the installed copy differs from the repo one, and name the differing node ids where it can.
6. Listing is read-only and works while the daemon runs (single writer is the daemon; the CLI reads files).

### Non-Functional
- Every edit prints what changed in hashes; no silent replace.
- No verb deletes a graph. Retirement is `pii graph list` → move aside, recorded.
- `list` output is stable and machine-readable with `--json`.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| list_reports_installed_only | repo has 19, installed 3 | three rows, the others absent unless --all |
| list_reports_drift | installed copy edited by hand | row says `DRIFT` |
| explain_names_trigger_and_effects | digest graph | trigger `timer`, effects include `post` and `memory` |
| edit_refuses_invalid_toml | broken TOML | refused, installed file unchanged, hashes identical |
| edit_moves_previous_aside | a valid edit | `.prev-<epoch>` exists, before-sha printed |
| edit_refuses_armed_run | graph with a suspended run | refused unless --force, names the run id |

## Exit Criteria
- [ ] `cargo test -p pii-cli --test graph_list_and_explain`
- [ ] `cargo test -p pii-cli --test graph_edit_refuses_and_keeps`
- [ ] on the host: `pii graph list` shows the three installed graphs with `same` or `DRIFT` against the repo
- [ ] `pii graph explain discord-digest` prints TRIGGER, the node chain, EFFECTS and the file hash

## Guardrails
- Do NOT edit a graph in place: lint first, move the old file aside, atomic write, re-lint the result.
- Do NOT delete a graph or its previous versions.
- Do NOT paraphrase in `explain`: render the file's own fields, or the explanation drifts from the thing described.
- Do NOT let `--force` skip the LINT step; force may only override the armed-run refusal.

## Depends on
- nothing. This is the audit surface for the graphs the daemon actually loads; 212's woven nodes appear here once built.
