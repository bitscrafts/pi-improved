#!/usr/bin/env python3
"""row-verify.py — a plan row's verify column must be its spec's ## Exit Criteria, command for command (spec 309). Read-only.

usage:
  row-verify.py criteria <spec.md>  -> the spec's criteria, one per line (exit 0); NO-CRITERIA (2), NOT-A-CRITERION (3)
  row-verify.py check <row-id>      -> OK <id> <n> (exit 0); COUNT / DIFF / SPLIT lines (exit 1); NO-CRITERIA (2), NO-ROW (2)
  row-verify.py check-all           -> OK or REFUSED <id>: <why> for each row (exit 1 if a dispatchable row is refused)
A row's column 5 is split on ';' (as cycle.sh does) and compared with the spec's criteria by exact string, after
trimming the edges only. PII_PLAN names the plan, PII_SPECS the specs directory.
"""
import os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DONE = ("done", "skipped", "revoked")
CRITERION = re.compile(r"^- \[ \] `([^`]+)`$")


def specs_dir():
    return os.environ.get("PII_SPECS") or os.path.join(ROOT, "specs")


def criteria(path):
    """(rc, commands): the ## Exit Criteria commands in order. rc 2 = no block (or unreadable), 3 = a line that is not one criterion."""
    commands, inside = [], False
    try:
        for line in open(path, encoding="utf-8"):
            line = line.strip()
            if line.startswith("## "):
                inside = line == "## Exit Criteria"
            elif inside and line.startswith("- ["):
                hit = CRITERION.match(line)
                if not hit or not hit.group(1).strip():
                    return 3, [line]
                commands.append(hit.group(1).strip())
    except OSError:
        return 2, []
    if not commands:
        return 2, []
    return 0, commands


def judge(row):
    """(rc, lines) for one plan row: 0 = OK, 1 = COUNT/DIFF/SPLIT lines, 2/3 = the spec declares no usable criteria."""
    rc, want = criteria(os.path.join(specs_dir(), row["spec"]))
    if rc == 2:
        return 2, [f"NO-CRITERIA {row['spec']}"]
    if rc == 3:
        return 3, [f"NOT-A-CRITERION {want[0]}"]
    # the column joins with ';', so a criterion holding one cannot round-trip: the split makes two, and the shell
    # discards the first one's failure (measured on A55/188)
    for i, c in enumerate(want, 1):
        if ";" in c:
            return 1, [f"SPLIT {i}: join the commands with && inside one criterion"]
    have = [c.strip() for c in row["verify"].split(";") if c.strip()]
    if len(have) != len(want):
        return 1, [f"COUNT spec={len(want)} row={len(have)}"]
    diffs = [f"DIFF {i} spec={w} row={h}" for i, (w, h) in enumerate(zip(want, have), 1) if w != h]
    if diffs:
        return 1, diffs
    return 0, [f"OK {row['id']} {len(want)}"]


def main():
    a = sys.argv[1:]
    if len(a) == 2 and a[0] == "criteria":
        path = a[1] if os.path.exists(a[1]) else os.path.join(specs_dir(), a[1])
        rc, cmds = criteria(path)
        if rc == 2:
            print(f"NO-CRITERIA {a[1]}")
            raise SystemExit(2)
        if rc == 3:
            print(f"NOT-A-CRITERION {cmds[0]}")
            raise SystemExit(3)
        print("\n".join(cmds))
        return
    if a[:1] == ["check"] or a == ["check-all"]:
        import state  # the plan reader is state.py's: one parser, one duplicate-id refusal
        rows = state.rows()
        if a == ["check-all"]:
            status = state.load()
            bad = 0
            for row in rows:
                rc, lines = judge(row)
                if rc:
                    # a closed row that differs is printed so the drift stays visible, but does not turn the check red
                    if status.get(row["id"], {}).get("status", "pending") not in DONE:
                        bad += 1
                    print(f"REFUSED {row['id']}: {', '.join(lines)}")
                else:
                    print(lines[0])
            raise SystemExit(1 if bad else 0)
        if len(a) == 2:
            hit = [r for r in rows if r["id"] == a[1]]
            if hit:
                rc, lines = judge(hit[0])
                print("\n".join(lines))
                raise SystemExit(rc)
            print(f"NO-ROW {a[1]}")
            raise SystemExit(2)
    raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
