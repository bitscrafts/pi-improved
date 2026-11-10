# Spec: skills can be listed, edited and explained

## Overview
User directive 2026-10-19: "pii must be able to list/edit and explain its skills" — and the same day fixed the meaning: a skill is `DocKind::Skill`, a memory document (`knowledge/skill-means-dockind-skill`). Today such a document can only be reached by `pii memory search` and replaced by another `put`; there is no way to enumerate skills, read one whole, change one deliberately, or ask what it MEANS and whether its pinned script still matches. A skill nobody can list is a skill nobody can audit — and the podman skill is now pii's main repair tool, so auditing it is not optional.

**Project**: <repo>

## Verified premises
- `Doc` carries `id · layer · kind · text · concepts · importance · created_ms · last_used_ms · evidence · archived`; `DocKind = Atom | Scene | Persona | Skill`.
- A skill's executable half is a repo file pinned in the body: `script: <path> sha256=<hex>` (skills-and-hooks-in-pii.md §7).
- `pii memory search` returns documents but does not filter by kind, and has no notion of the pin.

## Requirements
### Functional
1. `pii skill list [--all] [--layer L]` — one line per skill: short id · the skill's slug (first line after `PII-SKILL`) · layer · concepts · last_used · pin status (`ok` | `DRIFT` | `none`). `--all` includes archived.
2. `pii skill show <id|slug>` — the full body, verbatim, plus its metadata; refuses ambiguity with the list of candidates if the slug matches more than one.
3. `pii skill explain <id|slug>` — a rendered summary built from the body, never a paraphrase of a paraphrase: WHEN TO USE, the CONTRACT lines, the VERBS, the PIN and its verification, and the last time it was used.
4. `pii skill edit <id> --file <path> [--note <text>]` — replaces the body from a file, records `evidence = (commit-or-file, ms)`, keeps the id, and REFUSES when the new body would drop the pin line for a script the old body pinned (a silent de-pinning is the dangerous edit).
5. Pin verification is a REAL check: parse `script: <path> sha256=<hex>` from the body, hash the file on disk, and report `ok` / `DRIFT` / `MISSING FILE`. `explain` and `list` both surface it; a `DRIFT` skill may not be used by a node (refusal event `skill.pin.drift`).
6. Everything is read-through the store; no separate skill registry, no filesystem of skills.

### Non-Functional
- `explain` must work with only the store: if the pinned script is absent, it says `MISSING FILE` and still explains the skill.
- No verb silently writes: `edit` prints what changed (bytes, pin before → pin after) and exits non-zero on refusal.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| list_shows_only_skills | documents of kinds Atom/Scene/Persona/Skill | only Skill rows listed |
| list_reports_pin_status | body pinning a real file with a wrong hash | row says `DRIFT` |
| explain_names_the_pin | a skill with a valid pin | output contains `PIN ok <sha256>` |
| edit_keeps_the_pin | edit that removes the pin line | refused, non-zero, body unchanged |
| explain_without_the_script | script deleted | `MISSING FILE`, skill still explained |

## Exit Criteria
- [ ] `cargo test -p pii-cli --test skill_list_and_explain`
- [ ] `cargo test -p pii-cli --test skill_edit_keeps_the_pin`
- [ ] `pii skill list` on the live store shows `skill/manage-podman-containers` with `ok`
- [ ] `pii skill explain manage-podman-containers` prints WHEN TO USE, CONTRACT, VERBS and `PIN ok <sha256>`

## Guardrails
- Do NOT add a skill registry outside the store.
- Do NOT let `edit` weaken or delete a pin; de-pinning requires a NEW skill id.
- Do NOT execute anything: these verbs read and write documents, nothing else (execution stays with the coder node, spec 209).
- Do NOT paraphrase the body in `explain` — render its own lines, or the explanation drifts from the skill.

## Depends on
- nothing: the store and `DocKind::Skill` already exist. This is the audit surface for the podman skill.
