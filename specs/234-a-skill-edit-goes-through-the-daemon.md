# Spec: a skill's stored text can be edited through the daemon

## Overview

Spec 227 criterion 7 has two live halves, and the first deploy that can measure them is blocked by the second:
the coder's drift refusal needs a way to change a skill's stored text while the daemon runs. Spec 233 (row A101,
committed 3d32079) wired `skill.put` / `skill.list` / `pii skill add` through the daemon, but `pii skill edit` still
opens redb directly (crates/pii-cli/src/cmd_skill.rs:423 `open(&o)`): against the live runtime dir it fails with
`Database already open. Cannot acquire lock.` — measured on the lnx daemon, pid 3539364. The only producer of a
changed `DocKind::Skill` payload on the live path would be a stopped daemon, and stopping the production daemon to
mutate a store is not an accepted move.

So criterion 7's drift half has the same shape 233 measured for the spawn half: the seam exists, the store holds the
text, and no production command can write through the running daemon. `pii_memory::skill::add` is now the shared
implementation `skill.put` uses; an edit needs the same discipline, one function, two callers.

## Architecture

No new crate. Three seams, all existing:

- `crates/pii-proto/src/method.rs`: one new method, `skill.edit`, added the same way `skill.put` is (enum variant,
  `as_str`, `from_str`).
- `crates/pii-server/src/engine/skill_methods.rs`: `skill_edit(engine, req)` shaped like `skill_put`. It writes
  `engine.memory`, the same `MemStore` the coder node's `StoreSkills` reads, so the next `run.start` sees the edit.
- `crates/pii-memory/src/skill.rs`: `edit(store, id, new_text, now_ms, source) -> Result<Edited, EditError>` — the
  one implementation of "change a live skill's text, keep its id, concepts and created_ms, refuse a dropped pin".
  `pii skill edit` calls it; `skill.edit` calls it. The pin-diff logic (`pins_of`, dropped-pin refusal) moves from
  `cmd_skill.rs` into it, because the rule must not live twice.

## Requirements

R1. `skill.edit {id, text, source?}` stores the text under the SAME document id: the doc's layer, concepts and
    created_ms are carried, the concept edges are not re-put (they are already in the graph — the same rule
    `pii skill edit` applies today by clearing `rows.edges`). The reply is `{id, slug, sha256, bytes_before,
    bytes_after}` where `sha256` is the hash of the NEW text — the value a graph's pin must be updated to.
R2. `skill.edit` refuses, and writes nothing, when: the id is not a live skill document (`no live skill with id`);
    the text has no `PII-SKILL` marker; the new text drops a pin the old text carried (an edit that would
    silently de-pin a script is the refusal `pii skill edit` already prints, moved whole into the shared
    function). A dropped pin needs a NEW skill id, which is what `pii skill add` is for.
R3. `pii skill edit <id> --file <path>` goes through `skill.edit` when the daemon serves the store
    (spec 233 R2's `daemon_serves`, unchanged); when no daemon is reachable it changes the store directly as
    today; when the daemon answers with an error that error is reported, never a silent fallback to the locked
    store. The source line (`via daemon` / `via store <path>`) is printed as `add` does.
R4. A skill edit is visible to the very next coder run: after `skill.edit`, a coder node pinning the OLD sha256
    refuses with `skill.drift` (spec 225 R4), and pinning the NEW one resolves and spawns, carried in
    `coder.spawned.skill=` / `sha256=`.
R5. `lint-observability` stays green; `cmd_skill.rs`'s budget line is amended with a reason only if the count
    moves. No print is added to the daemon side.

## TDD Contract

- RED first at HEAD 3d32079: `crates/pii-server/tests/skill_edit_through_the_daemons_store.rs` fails on
  `Method::SkillEdit` not existing (a wire method the daemon does not answer).
- The test, over the loopback transport (the harness `skill_put_through_the_daemon.rs` shapes):
  (a) `skill.put`, then `skill.edit` of that id: the reply carries the same id, the new slug, and the new sha256;
  (b) a coder leg pinning the OLD sha256 refuses with `skill.drift` naming the skill, and the fake podman journal
      holds no `run` line;
  (c) the same leg pinning the NEW sha256 runs, and `coder.spawned` names the skill with the new hash;
- Controls in the same file: an edit of an id no live skill holds is refused and the store's live skill count is
  unchanged; an edit whose text drops the body's `script:` pin is refused and the stored text is byte-equal to the
  old one; a text with no marker is refused and writes nothing.
- `pii skill edit`'s own semantics are covered by the existing `skill_edit_keeps_the_pin.rs` / `skill_list_and_explain.rs`
  and stay green UNCHANGED: the refactor moves the rule, it must not change the command's behaviour or output.

## Exit Criteria

- [ ] `cargo test -p pii-server --test skill_edit_through_the_daemons_store`
- [ ] `cargo test -p pii-server --test skill_put_through_the_daemon` (the daemon-put path over the shared `edit`-free code stays green)
- [ ] `cargo test -p pii-cli --test skill_edit_keeps_the_pin`
- [ ] `cargo test -p pii-cli --test skill_list_and_explain`
- [ ] `cargo test -p pii-proto --test wire_roundtrip`
- [ ] `CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-observability`
- [ ] `cargo fmt --all -- --check` and `cargo clippy -p pii-proto -p pii-server -p pii-cli -p pii-memory --all-targets` clean
- [ ] on lnx, after a deploy: `pii skill edit` on the live store reports `via daemon`, the coder graph pinned at the
      old sha256 refuses `skill.drift`, and the graph pinned at the new sha256 journals `coder.spawned` with the
      skill's name and new hash — together with the spawn half this measures spec 227 criterion 7 in full.

## Guardrails

- Do NOT archive-and-re-add: an edit keeps the document's id exactly (spec 233's R1 made the slug unique for the
  same reason — a second document makes `pick` ambiguous).
- Do NOT let the server-side edit skip the marker or pin-drop checks because "the client already checked": the
  daemon's store is writable by every client, and the rule must hold for a direct `skill.edit` caller too.
- Do NOT change `pii skill edit`'s output or exit codes: 233's rule moves the seam, it does not redesign the command.
- The guardrails of spec 233 carry over unchanged: one naming rule, never a direct redb write while a daemon
  serves the store, budget lines only with a checkable reason.

## Closure

(empty — nothing closed without measurement)
