# 073: The path, fifth reading. One change since 072, and it is in production's memory (Opus, 2026-10-19, 17:20Z)

This is a read-only review: the only repo file written is this one, and nothing was committed. Production was read
through `pii health`, `pii run ls`, `pii memory entity list` and file metadata only. Evidence labels:
- **C-run**: I ran it today, 17:20Z.
- **C-read**: I read the lines.

The brief is the Round-6 brief a sixth time. **069-072 answer all five points.** This file records only what moved
in the 35 minutes since 072, so that the series stays current without repeating it.

## 0. What did not change (C-run)

- **Revision:** `420fe8a7e946`, up 1.7 h, never signalled since 15:37:50Z.
- **Store:** 1,878 runs, **1 suspended** (the armed digest), 41 succeeded, 1,836 failed, and `cron_entries: 1`.
- **Posts:** `channel_posts 1`, refusals 0.
- **Config and graphs:** `pii.toml` mtime 10-19 15:37, and the graph dir is unchanged (`discord-digest`,
  `discord-triage`, `intervention-report`), with 1 `[[cron]]`.

**So 072's path, redeploy checks, a/b/c split, risk note and observability gap all stand unchanged.** The next
reading that moves the verdict is still **2026-10-20 12:00Z**, the first occurrence on `420fe8a`.

## 1. What did change: production's memory store (C-run)

| reading | 072 and before | now |
|---|---|---|
| `memory_entities` | 0 (068, 070) | **1,888** |
| `memory_docs` | 0 | **463** |
| entities of kind `device` | 0 | **0** |
| entities of kind `lesson` | 0 | **0** |

**Source: the builder loop, not a deploy.** 10 commits since 072 (16:47-17:13), among them:
- `2b21a41`: "pii indexes its own codebase: 1415 entities from 23 crates";
- `aa1a42c`: "286 structured documents covering every spec";
- `3560df2`: "every drained row's outcome is written to pii memory".

`scripts/autonomy/learn.sh:18` defaults to `PII_RUNTIME_DIR=/run/user/0/pii`, the **production** daemon's socket.
`scripts/pii-ingest.py:4` quotes the directive: "we do need all pii implementation inside pii memory!"

**What it means for the path:**
1. **The digest does not change.** It recalls kinds `device` and `lesson` (`discord-digest.toml`), and both are 0.
   Tomorrow's 12:00Z post will still say "0 device(s) known, 0 lesson(s) remembered" over a store holding 1,888
   entities. The one scheduled behaviour still reports nothing the store now knows.
2. **Production state now changes without a deploy, a card or a gate.** The builder loop writes into the running
   daemon's store on every drained row. That is the directive, so it is not a defect. But it puts a writer on the
   production store that 070 step 3 ("production is not the test bench") does not cover, and that no card records.
   The store is the soak's evidence source, so **it is a second writer to the soak's evidence**.
3. **Nothing has been read back.** I found no measurement that a `pii memory search` against production returns the
   indexed material, or that the ingest is idempotent across re-runs (490 lines of `ingest-ledger.json` suggest a
   ledger, and I did not test it). Until one exists, "1,888 entities" is a count, not a capability.

## 2. The one addition to 072's answers

- **§1 path, a new constraint on step 6 (the soak):** the soak's readings (runs, posts, armed) must be taken from
  run and channel records, which the memory ingest does not write. **Memory counters are excluded from soak
  evidence until the ingest is idempotent and attributed to its writer.**
- **§4 risk:** unchanged. The three sentences in 072 §3.4 stand, because the memory ingest does not touch the
  scheduler or the channel.
- **§5 gap:** unchanged in kind, with one more instance. The store now has writers (the daemon, `learn.sh`,
  `pii-ingest.py`, the code indexer) that `since_deploy` cannot tell apart. Its `unattributed` field counted
  1,850 inherited *runs*. Nothing counts unattributed *memory writes*.

## NOT VERIFIED

| item | why |
|---|---|
| that `pii memory entity list --kind device` lists by kind as I assumed | the command returned a JSON value I parsed as a count of 0; I did not cross-check with `pii memory tiers` |
| the ingest's idempotence | not tested (a re-run would write to production) |
| tomorrow's 12:00Z occurrence | future; commands in 072 §3.2 |

Nothing in the repo changed except this file. No scratch was created this round.
