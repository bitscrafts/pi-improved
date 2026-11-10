# Spec: a skill can be added to the store the daemon reads, through the daemon

## Overview
Spec 227 wired the live registry to `StoreSkills` (the coder node reads its skills from the daemon's own memory
store), and its six unit criteria are green on the committed tree. Its criterion 7 is live: on the host, a coder graph
that pins a stored skill journals `coder.spawned` with `skill=<name> sha256=<hex>`, and the same graph after the
skill is edited refuses with `skill.drift`. M0 now passes on the host (spec 230, run r258), so the container is no
longer what blocks it. Measured by the supervisor on 2026-10-20 against the the host daemon (pid 3539364, binary
installed 12:38:35, `PII_RUNTIME_DIR=/run/user/1000/pii`), it cannot be run, for three reasons:

1. **The store holds no skill, and the shipped code cannot write one.** `pii skill list --all` over a copy of
   `/run/user/1000/pii/memory.redb` prints nothing. Outside tests, `DocKind::Skill` is constructed only by
   `pii skill edit <id>` (`crates/pii-cli/src/cmd_skill.rs:444`), and that command replaces an id that already
   exists. `pii-learn` and `pii-evolve` never write the kind. The tests seed a skill through
   `MemStore::put(&rows_for(&Doc::new(L3, DocKind::Skill, ..)))`, which production has no equivalent of.
2. **`pii skill` cannot open the store while the daemon runs.** `pii skill list --all --dir /run/user/1000/pii`
   → `error: internal: memstore: open /run/user/1000/pii/memory.redb: Database already open. Cannot acquire
   lock.` Spec 220 fixed this for `run trace` by going through the daemon. `pii skill` still opens redb directly.
3. **The default dir is the wrong store.** With no `--dir`, `pii skill list` prints `error: no memory store at
   /run/user/1000/pii/default/memory.redb`, because `pii_uds::runtime_dir("default")` adds `/default`. The daemon's
   store is `/run/user/1000/pii/memory.redb` (`crates/pii-server/src/lib.rs:290`, `sibling("memory.redb")`).

So an operator cannot put a skill in front of a coder node. Spec 227's seam is real, but nothing in production can
use it yet.

## Architecture
No new crate. The seams already exist:
- `crates/pii-proto/src/method.rs`: two new methods, `skill.put` and `skill.list`, added the same way `RunTrace`
  is (enum variant, `as_str`, `from_str`).
- `crates/pii-server/src/engine/dispatch.rs`: both methods dispatch into a new `engine/skill_methods.rs`, shaped
  like `methods::memory_put`. They write and read `engine.memory`, the same `MemStore` that `StoreSkills` reads.
- `crates/pii-memory/src/skill.rs`: the one naming rule (`slug_of`, `live`, `pick`). Both methods use it. No
  second rule is added.
- `crates/pii-cli/src/cmd_skill.rs`: `list`, `show` and the new `add` first ask the daemon (`run_call_typed`, as in
  `cmd_trace.rs::read_via_daemon`). Only on `CallError::Down` do they open redb directly, and they print which
  path was used.

## Requirements
R1. `pii skill add --file <path> [--layer L1|L2|L3] [--dir D]` stores the file's text as one `DocKind::Skill`
    document (layer defaults to L3). The name is `pii_memory::skill::slug_of(text)`. The command refuses, and
    writes nothing, when the text has no `PII-SKILL` marker, or when a live skill already has that slug: a
    second document with the same name would make `pick` ambiguous, and changing a skill is what `edit` is for.
    On success it prints `added <id> <slug> sha256=<hex>`. `<hex>` is exactly the value a graph's
    `skills = [{ name, sha256 }]` pin must carry for `StoreSkills` to accept it: one hash, computed by the
    function the coder node uses, never a second implementation.
R2. With a daemon reachable, `skill add`, `skill list` and `skill show` go through `skill.put` / `skill.list`, and
    the daemon writes its own store. With no daemon, they open the store directly, as they do today. Either way
    the command prints the path it used (`via daemon` / `via store <path>`) on stderr. A daemon that answers with
    an error is reported as an error. It is never a reason to fall back to opening the store, because that store
    is locked.
R3. With no `--dir`, `pii skill` resolves the same store the daemon opens. The CLI and the daemon share one
    resolver, so `PII_RUNTIME_DIR=/run/user/1000/pii` names `/run/user/1000/pii/memory.redb` for both. Fix only
    `cmd_skill.rs`'s default here. The same `"default"` suffix in other commands (`cmd_backup`, `cmd_store`,
    `cmd_entity`, `cmd_learn`, `cmd_index`) is recorded in `## Closure` as a finding and left alone.
R4. `skill.put` runs on the daemon's `MemStore` handle, so the very next `run.start` of a coder node that pins the
    skill resolves it through `StoreSkills`. The skill reaches the node without a restart and without a file
    outside the store.
R5. `lint-observability` stays green. A new print in a CLI command takes a budget line with a reason in
    `xtask/observability-allowlist.txt`. A daemon-side report goes through pii-observe.

## TDD Contract
Write each test first, and record it RED before the fix.
- `crates/pii-server/tests/skill_put_through_the_daemon.rs`: spawn a scratch daemon (PII_CONFIG pinned to the
  scratch, as `995482f` does). Send `skill.put` a text with a `PII-SKILL` marker. Then, in the same daemon,
  `run.start` a one-node coder graph that pins that slug at the returned sha256, using the fake podman from
  `live_registry_supplies_the_skill_source.rs`. Assert that `coder.spawned` carries `skill=<slug>` and the same
  sha256. Control: a second `skill.put` with the same slug is refused and the store's skill count stays 1. Second
  control: a text with no marker is refused.
- `crates/pii-cli/tests/skill_cli_reads_through_the_daemon.rs`: while a scratch daemon holds the store,
  `pii skill add --file F` exits 0 and prints `via daemon`, and `pii skill list` lists the slug. With the daemon
  stopped, the same `list` over the same dir prints `via store` and the same slug. A test that only checks exit
  codes proves nothing here: assert the slug, the sha256 and the path line.
- The existing `skill_list_and_explain` and `skill_edit_keeps_the_pin` tests keep passing unchanged.

## Exit Criteria
- [ ] `cargo test -p pii-server --test skill_put_through_the_daemon`
- [ ] `cargo test -p pii-cli --test skill_cli_reads_through_the_daemon`
- [ ] `cargo test -p pii-cli --test skill_list_and_explain && cargo test -p pii-cli --test skill_edit_keeps_the_pin`
- [ ] `cargo test -p pii-server --test live_registry_supplies_the_skill_source && cargo test -p pii-server --test live_registry_refuses_a_drifted_skill`
- [ ] `CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-observability`
- [ ] on the host, after a deploy (orchestrator, not the worker): `pii skill add --file <skill>` prints `via daemon`.
      Then spec 227's criterion 7 runs as written: a coder graph that pins the skill journals
      `coder.spawned skill=<slug> sha256=<hex>`, and after `pii skill edit` changes the text, the same graph
      refuses `skill.drift`.

## Guardrails
- Do NOT add a second naming rule or a second pin hash: `slug_of` / `pick` and the coder's pin function are the
  only ones. If they disagree, fix the shared function.
- Do NOT let `skill add` overwrite an existing slug, and do NOT let the CLI fall back to opening the store when the
  daemon answered with an error (it would hit the lock, or worse, a stale copy).
- Do NOT give the daemon a skill source outside the store (no `$HOME`, no `.pi/skills`): spec 227's guardrail
  stands.
- Do NOT touch `xtask/src/lint_observability.rs` or loosen a budget without a checkable `# reason`.
- If `skill.put` cannot reach `engine.memory` without restructuring the engine, STOP and report the shape.
  Do not force it.

## Closure
(open)
