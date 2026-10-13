#!/usr/bin/env python3
"""state.py — the autonomy queue's memory (atomic, re-entrant).

One JSON file holds one row per unit of work. Every write goes to a temp file and is
os.rename'd into place, so a cycle killed mid-write cannot leave a truncated state
(that is the whole reason this is not `>>` on a log).

usage:
  state.py pick [<state>]                 -> the top pending item as TSV, or "NONE"
  state.py get <id> [<state>]             -> that row as JSON
  state.py set <id> k=v [k=v ...]         -> update fields (status/attempts/evidence/session/last)
  state.py list                           -> every row, TSV, for reporting
A `plan.tsv` row is: id <tab> spec <tab> brief <tab> title <tab> verify(cmd;cmd) <tab> paths(a;b) <tab> gate
"""
import json, os, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
STATE = os.environ.get("PII_STATE") or os.path.join(ROOT, "_tmp", "autonomy", "state.json")


def rows():
    plan = os.environ.get("PII_PLAN") or os.path.join(ROOT, "scripts", "autonomy", "plan.tsv")
    out = []
    for line in open(plan):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) < 7:
            raise SystemExit(f"plan.tsv row needs 7 tab-separated fields: {line!r}")
        out.append(dict(zip(("id", "spec", "brief", "title", "verify", "paths", "gate"), f)))
    return out


def load():
    try:
        return json.load(open(STATE))
    except Exception:
        return {}


def save(d):
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(STATE), suffix=".json")
    with os.fdopen(fd, "w") as fh:
        json.dump(d, fh, indent=1, sort_keys=True)
    os.replace(tmp, STATE)


def main():
    a = sys.argv[1:]
    if not a:
        raise SystemExit(__doc__)
    cmd = a[0]
    if cmd == "list":
        for r in rows():
            st = load().get(r["id"], {})
            print("\t".join([r["id"], r["spec"], r["title"], st.get("status", "pending"),
                             str(st.get("attempts", 0)), st.get("last", "")]))
        return
    if cmd == "pick":
        st = load()
        for r in rows():
            s = st.get(r["id"], {}).get("status", "pending")
            if s in ("pending", "in-progress", "needs-review"):
                print("\t".join([r["id"], r["spec"], r["brief"], r["title"], r["verify"], r["paths"], r["gate"],
                                 str(st.get(r["id"], {}).get("attempts", 0)),
                                 st.get(r["id"], {}).get("session", "")]))
                return
        print("NONE")
        return
    if cmd == "get":
        print(json.dumps(load().get(a[1], {}), indent=1))
        return
    if cmd == "set":
        d = load()
        row = d.setdefault(a[1], {})
        for kv in a[2:]:
            k, v = kv.split("=", 1)
            row[k] = int(v) if v.isdigit() else v
        save(d)
        return
    raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
