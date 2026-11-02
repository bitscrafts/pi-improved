#!/usr/bin/env python3
"""row-paths.py — a plan row's commit paths must equal the `## Files` block of its spec (spec 283). Read-only.

usage:
  row-paths.py files <spec.md>   -> the spec's ## Files entries, one per line (exit 0); NO-FILES (2), NOT-A-FILE (3)
  row-paths.py check <row-id>    -> OK <id> <n> (exit 0); MISSING <path> / EXTRA <path> (exit 1); NO-FILES (2), NOT-A-FILE (3)
  row-paths.py check-all         -> OK or REFUSED <id>: <why> for each dispatchable row (exit 1 if any refused)
A row's column 6 is split on ';' (as cycle.sh does). PII_PLAN names the plan, PII_SPECS the specs directory.
"""
import os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DONE = ("done", "skipped", "revoked")


def specs_dir():
    return os.environ.get("PII_SPECS") or os.path.join(ROOT, "specs")


def declared(path):
    """(rc, entries): the ## Files entries in order. rc 2 = no block (or unreadable), 3 = an entry names a directory."""
    entries, inside = [], False
    try:
        for line in open(path, encoding="utf-8"):
            if line.startswith("## "):
                inside = line.strip() == "## Files"
            elif inside and line.count("`") >= 2:
                entries.append(line.split("`")[1].strip())
    except OSError:
        return 2, []
    entries = [e for e in entries if e]
    if not entries:
        return 2, []
    for e in entries:
        if e.endswith("/") or (os.path.exists(os.path.join(ROOT, e)) and not os.path.isfile(os.path.join(ROOT, e))):
            return 3, [e]
    return 0, entries


def judge(row):
    """(rc, lines) for one plan row: 0 = OK, 1 = MISSING/EXTRA lines, 2/3 = the spec declares no usable list."""
    rc, want = declared(os.path.join(specs_dir(), row["spec"]))
    if rc == 2:
        return 2, [f"NO-FILES {row['spec']}"]
    if rc == 3:
        return 3, [f"NOT-A-FILE {want[0]}"]
    have = [p.strip() for p in row["paths"].split(";") if p.strip() not in ("", "-")]
    missing = [p for p in want if p not in have]
    extra = [p for p in have if p not in want]
    if missing or extra:
        return 1, [f"MISSING {p}" for p in missing] + [f"EXTRA {p}" for p in extra]
    return 0, [f"OK {row['id']} {len(want)}"]


def main():
    a = sys.argv[1:]
    if len(a) == 2 and a[0] == "files":
        path = a[1] if os.path.exists(a[1]) else os.path.join(specs_dir(), a[1])
        rc, entries = declared(path)
        if rc == 2:
            print(f"NO-FILES {a[1]}")
            raise SystemExit(2)
        if rc == 3:
            print(f"NOT-A-FILE {entries[0]}")
            raise SystemExit(3)
        print("\n".join(entries))
        return
    if a[:1] == ["check"] or a == ["check-all"]:
        import state  # the plan reader is state.py's: one parser, one duplicate-id refusal
        rows = state.rows()
        if a == ["check-all"]:
            status = state.load()
            bad = 0
            for row in rows:
                if status.get(row["id"], {}).get("status", "pending") in DONE:
                    continue
                rc, lines = judge(row)
                if rc:
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
