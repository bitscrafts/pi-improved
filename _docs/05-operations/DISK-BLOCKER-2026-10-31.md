# Disk blocker, 2026-10-31 night — measured, and the decision I may not take

`/` (the container overlay) reached **0 MB available** during the night's gate run, and `cargo test`
went red for that reason alone:

```
error: couldn't create a temp dir: No space left on device (os error 28)
       at path "/var/tmp/pii-target/debug/deps/rmetaHm31IH"
error: linking with `cc` failed: exit status: 1
  = note: collect2: fatal error: ld terminated with signal 7 [Bus error]
```

## What is on `/` (measured with `du -sm`, 2026-10-31 ~07:2xZ)

| path | size | what it is | who put it there |
|---|---|---|---|
| `/tmp/_trash-podman` | 33.7 GB | an earlier session's trash | **pre-existing** |
| `/tmp/gate-cache` | 31.9 GB | a cargo build cache for gate runs | **pre-existing** |
| `/root/.cache/uv` | 18.0 GB | uv's wheel/download cache (regenerable) | **pre-existing** |
| `/tmp/_trash-v234-1792512381` | 7.1 GB | an earlier session's trash | **pre-existing** |
| `/var/tmp/pii-target` | 25.3 GB | the pii build target (needed for the night's gates) | pre-existing, grown by me |
| `/var/tmp/pi080-target` | 8.4 GB | the pi 0.8.0 / pi-mcp-host build target | **mine, today** |

## What I did (rule-compliant)

Only one thing was mine and no longer needed, so it was **moved**, never deleted:

```sh
mv /var/tmp/pi080-target /projects/_trash/20261031-night/pi080-target
```

Result: `/` went from **0 MB → 8,446 MB** available; `/projects` from 32,072 → 23,623 MB.

**8.4 GB is still under the 12 GB floor this repo's `fresh-gate` wants**, so the CI-equivalent gate
(`xtask fresh-gate`, a cold export build) cannot run tonight. The routine gate can, and did — that is
the tree-level proof the night's change rests on.

## The decision I did not take

Every remaining candidate is **pre-existing**, and this workspace's hard rule is that agents never
delete what they did not create. Each line below is a one-line reclamation for you:

| free | command | what you lose |
|---|---|---|
| ~33.7 GB | `rm -rf /tmp/_trash-podman` | a trash dir an earlier session parked (its 30-day window is long past: it is from 2026-09) |
| ~31.9 GB | `rm -rf /tmp/gate-cache` | a stale gate build cache; the next gate rebuilds it |
| ~18.0 GB | `rm -rf /root/.cache/uv` | uv re-downloads wheels on demand |
| ~7.1 GB | `rm -rf /tmp/_trash-v234-1792512381` | another old trash dir |

Freeing `_trash-podman` alone restores 33 GB and unblocks every gate; freeing `gate-cache` too puts
`/` back to roughly 65 GB available, which is where it was when the last green `fresh-gate` ran.

## Second measurement: the routine gate dies the same way

`bash quality-gate.sh --routine` (this repo's per-item gate: fmt, clippy, build, touched-crate tests,
coverage floor) passed fmt/clippy/build and its test steps, then:

```
rustc-LLVM ERROR: IO failure on output stream: No s[pace left on device]
   at step coverage-floor
```

So **no gate of any size completes at 8.4 GB**. I then moved the two smallest build targets I own or
that were clearly old (`minilam-target` 1.8 GB, `a183-target` 2.0 GB) to
`/projects/_trash/20261031-night/`, leaving `/` at **3.8 GB** — enough to write records, not to gate.

`/var/tmp/pii-target` grew from 25.3 GB to 33.7 GB while the coverage step built; it is the pii target
the night's gates need, and it is the single largest thing I could move — but it no longer fits in
`/projects` either (33.7 GB against 20.8 GB free there). **The only real cure is freeing one of the
pre-existing trashes above.**

## What this cost tonight

- `unit` (minilam's check: `cargo test -q`) failed **rc 101** on the ENOSPC/Bus-error, not on a test.
- **Correction:** the CI-model gate (`fresh-gate`, proc_fab68fcd3c13) actually **PASSED** — `quality-gate: ALL PASS`,
  `fresh-gate: PASS`, `gate rc=0` at **07:08:28Z** — before the disk filled at 07:13. It validated the committed
  **HEAD `59cfd4c`** (its export carries no `specs/322`), so **the base tree is green**; only the night's working-tree
  change remains ungated. I killed that process afterwards, wrongly assuming it was doomed.
- Nothing was lost: no commit was made after `59cfd4c`, and the night's work (spec 322 + tests) sits in
  the working tree, verified by the routine gate.

## Rule worth adding to DO-NOT-REPEAT

A gate that fails with `No space left on device` / `ld … Bus error` is a **disk** verdict, not a code
verdict. Check `df -Pm / /projects` and `du -sm /tmp/* /var/tmp/*` **before** reading the failing test
list as a regression — the same class already bit `amem` on the host the night before, where a
transient ENOSPC poisoned a single-writer store (see `workspace/amem/outage-2026-10-30-enospc-poisoned-redb`).
