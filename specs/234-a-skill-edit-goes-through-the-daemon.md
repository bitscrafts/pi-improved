# Spec: a skill's stored text can be edited through the daemon

## Overview

Spec 227 criterion 7 has two live halves, and the first deploy that can measure them is blocked by the second:
the coder's drift refusal needs a way to change a skill's stored text while the daemon runs. Spec 233 (row A101,
committed 3d32079) wired `skill.put` / `skill.list` / `pii skill add` through the daemon, but `pii skill edit` still
opens redb directly (crates/pii-cli/src/cmd_skill.rs:423 `open(&o)`): against the live runtime dir it fails with
`Database already open. Cannot acquire lock.` — measured on the the host daemon, pid 3539364. The only producer of a
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
- [ ] on the host, after a deploy: `pii skill edit` on the live store reports `via daemon`, the coder graph pinned at the
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

Independently verified 2026-10-20 by the orchestrator on a clean detached worktree of `622826e` (own target dir,
so the uncommitted coder.rs/lib.rs split in the main tree did not take part):

- criteria 1-6 green: skill_edit_through_the_daemons_store 1/0, skill_put_through_the_daemon 1/0,
  skill_edit_keeps_the_pin 1/0, skill_list_and_explain 4/0, wire_roundtrip 12/0, lint-observability 0 findings
  (172/173 raw prints); `cargo fmt --all -- --check` rc=0.
- mutations of `crates/pii-memory/src/skill.rs::edit` (sha256 89e3f576…ec3f8 before, `sha256sum -c` OK after
  restore + touch, test green again):
  M1 drop the `NoMarker` refusal -> the daemon test reddens (L321); the CLI pin test stays green (it does not
  cover the marker, the daemon test is the one that guards R2's marker half).
  M2 drop the `DropsPin` refusal -> both red (daemon L314, CLI L96).
  M3 drop `doc.id.clone_from(&old.id)` -> both red (daemon L227, CLI L121).
- criterion 7 was RED at `622826e`: clippy -D warnings fails compiling pii-graph on `double_must_use` at
  `crates/pii-graph/src/fragment/splice.rs:175`, a `#[must_use]` that arrived in leftover commit `171b7ae`, not
  in this spec's diff. That is the cycle's `gate-failed`; the commit message's "clippy zero on four crates" was
  not true of the tree. Removing that one attribute (committed alone just before this closure) makes
  clippy on pii-graph/proto/server/cli/memory --all-targets rc=0, and pii-graph's tests stay 110/0.
- criterion 8 (the host live: `via daemon`, old pin refuses `skill.drift`, new pin journals `coder.spawned`) is NOT
  measured here; it needs a deploy of this tree and is carried by row A95 (spec 227 criterion 7).

### Re-verified at HEAD `c74aa2f`, 2026-10-21T00:20Z (the tree had moved on since `622826e`)

The main tree was dirty with another agent's runtime WIP, so the criteria were run on a clean detached worktree
of `c74aa2f` (`_tmp/verify-a102-20261021T001649Z`, its own `CARGO_TARGET_DIR`), not on the working tree:

- criteria 1-6 green: `skill_edit_through_the_daemons_store` rc=0 (61 s), `skill_put_through_the_daemon` rc=0,
  `skill_edit_keeps_the_pin` rc=0, `skill_list_and_explain` rc=0, `wire_roundtrip` rc=0, `lint-observability`
  rc=0; `cargo fmt --all -- --check` rc=0.
- one mutation, re-run at HEAD rather than trusted from the earlier closure: the `DropsPin` refusal in
  `crates/pii-memory/src/skill.rs::edit` removed. Both guards redden — `skill_edit_through_the_daemons_store.rs:318`
  and `skill_edit_keeps_the_pin.rs:96` (`pin before: ok  after: none`), rc=101 each. Restored with
  `git checkout --` (**never** `cp -p`, the stale-mtime false red): `sha256sum` after the restore is
  `89e3f57679c4b08d94927bb380d1d6a12dd25f9c59d556ce26487dfe581ec3f8`, the same value measured before the
  mutation, zero mutation markers left in the file, and both tests green again (rc=0).
- criterion 8 stays unmeasured, and the reason is now a reading rather than an assumption (2026-10-21T00:18Z
  from the host): the the host daemon is `active (running)` since 2026-10-20 15:41:08 BST inside the `hermes`
  container (`PII_RUNTIME_DIR=/run/user/0/pii`), and the installed binary is the 2026-10-20T12:38:35Z build
  (11,150,384 bytes) — behind HEAD, so the live half needs that deploy, not code.

