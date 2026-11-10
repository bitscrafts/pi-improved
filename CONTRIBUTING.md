# Contributing

1. Open an issue with the **Task** form. Say what changes, give exit criteria as commands that exit 0
   when it is done (and non-zero before), and list the files it may touch.
2. A maintainer marks it `ready` (`scripts/mark-ready.sh <number>`). It refuses an issue written by
   anyone but the owner unless the owner approves it with `--owner-approved`. Work starts from
   `ready` issues only.
3. Open a pull request that closes the issue. Paste the criteria output unsummarised, and name the
   test that fails without the change.

Before opening it:

```bash
bash quality-gate.sh --routine   # the gate, on this checkout and its warm build cache
```

A fresh-tree build of the same gate is `cargo run -q -p xtask -- fresh-gate --worktree`; it is slow
and is the check to run before a release.

Rules that review holds to:

- A test is written first and must fail for the right reason before the change.
- A check that always passes does not count. A rule that skips something needs a near-miss case that
  still takes the normal path.
- A source file stays under 450 lines; split into a folder module instead of raising the limit.
- Commit messages describe the change. Identity is your own.
- Do not weaken a check, or lower a floor, to make your own change pass.

Larger work gets a spec in [`specs/`](specs/) first. A spec carries a card line under its title
(`#: depends= status= block=`) and its exit criteria are runnable commands.
