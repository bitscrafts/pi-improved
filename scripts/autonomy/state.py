#!/usr/bin/env python3
"""state.py — the autonomy queue's memory (atomic, re-entrant).

One JSON file holds one row per unit of work. Every write goes to a temp file and is
os.rename'd into place, so a cycle killed mid-write cannot leave a truncated state
(that is the whole reason this is not `>>` on a log).

usage:
  state.py repair <id> <note...>          -> a park whose cause is repaired: status -> needs-review (or pending without\n                                             a session), last -> 'park-cause-repaired: <note>' (spec 323)\n  state.py pick [<state>]                 -> the top dispatchable item as TSV (parked/needs-review rows are skipped, except a needs-review row parked by a closed window, by a worker that stopped early (worker-rc=N), or by a refused commit (precommit-broken / precommit-timeout), or by a failed gate or criterion, with a session: spec 280, spec 312, spec 316, spec 318), or "NONE"
  state.py get <id> [<state>]             -> that row as JSON
  state.py annotate <id> [--spec <path>] [--commit <sha>]
                                          -> the row names its work and its commit; both are proven first (spec 326)
  state.py set <id> k=v [k=v ...]         -> update fields (status/attempts/evidence/session/last)
  state.py list                           -> every row, TSV, for reporting
A `plan.tsv` row is: id <tab> spec <tab> brief <tab> title <tab> verify(cmd;cmd) <tab> paths(a;b) <tab> gate
"""
import importlib, json, os, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
def _path(var, default):
    """The path a PII_* variable names, or the default if it is unset.

    A variable that is SET but blank is a mistake with a silent consequence: an empty
    PII_STATE used to fall through to the operator's live queue, so a test that forgot to
    set it wrote a fixture into real rows (row X1, 2026-11-04). Refuse instead of
    guessing -- but keep unset meaning the default, which every caller relies on.
    """
    if var not in os.environ:
        return default
    value = os.environ[var].strip()
    if not value:
        sys.stderr.write(f"{var} is set but empty; refusing to fall back to {default}\n")
        raise SystemExit(2)
    return value


STATE = _path("PII_STATE", os.path.join(ROOT, "_tmp", "autonomy", "state.json"))


SPECS = _path("PII_SPECS", os.path.join(ROOT, "specs"))
OPEN = ("pending", "in-progress", "needs-review")


def derived_verify(spec_name):
    """The Exit Criteria of a spec, in order, as one `;`-joined string — or None when unreadable/empty.

    Spec 327: for an OPEN row the verify column is a CACHE of this value, never an independent copy. A copy
    drifts (30 of 183 rows dropped a criterion outright, A154 losing its coder-demo grep), and the drift is
    invisible until the gate runs the wrong list. A finished row is the opposite case: its text is FROZEN
    history, because regenerating it from a spec that was amended later would claim the work was held to
    criteria that did not exist when it ran.
    """
    import re
    try:
        body = open(os.path.join(SPECS, spec_name)).read()
    except OSError:
        return None
    crit = re.findall(r"- \[ \] `(.+?)`", body)
    return ";".join(c.strip() for c in crit) if crit else None


def rows():
    plan = _path("PII_PLAN", os.path.join(ROOT, "scripts", "autonomy", "plan.tsv"))
    out = []
    for line in open(plan):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) < 7:
            raise SystemExit(f"plan.tsv row needs 7 tab-separated fields: {line!r}")
        row = dict(zip(("id", "spec", "brief", "title", "verify", "paths", "gate"), f))
        # spec 327: an open row's verify column is derived from its spec, so a row cannot be dispatched against
        # a criterion list that has drifted from the document it belongs to. The stored text is a cache; the
        # staleness check in sync-verify.sh is what keeps the file honest.
        if load().get(row["id"], {}).get("status", "pending") in OPEN:
            fresh = derived_verify(row["spec"])
            if fresh is not None:
                row["verify"] = fresh
        out.append(row)
    # The id IS the key the state is stored under, so two rows sharing one is not a cosmetic mistake: the second row
    # inherits the first's status and is never picked. Measured 2026-10-14: A20/151 (done) and A20/152 (queued, its
    # spec and brief already committed) both said A20, so every cycle for ~5 h reported "queue empty" with that work
    # sitting in the queue. Refuse loudly: an unreachable row must look broken, never idle.
    ids = [r["id"] for r in out]
    dup = sorted({i for i in ids if ids.count(i) > 1})
    if dup:
        raise SystemExit(
            "plan.tsv: duplicate row id(s): " + ", ".join(dup) + " — the state is keyed by the id, so a second row "
            "reusing one silently inherits the first row's status and is never dispatched. Give every row its own id."
        )
    return out


def resumable(status, row):
    # one rule for "a session can pick this row up again" (specs 280, 312, 316, 318): a needs-review row that has a session and
    # was parked by a closed window, a worker that stopped early, a refused commit, or a failed gate or criterion. Every other
    # row waits for a person. The gate's marker form (worker-ok; gate-failed: ...) is offered too: it is what the cycle writes.
    last = row.get("last", "")
    # spec 323: a park whose cause was repaired is offered again. The marker is written by `repair`, which also
    # moves the status to needs-review -- a row still `parked` with the marker is NOT offered, so the record never
    # says "parked" about a row the queue is working.
    return (status == "needs-review" and bool(row.get("session"))
            and (last == "window-closed" or last.startswith(("worker-rc=", "precommit-broken", "precommit-timeout",
                                                             "gate-failed", "verify-failed", "worker-ok; gate-failed",
                                                             "park-cause-repaired"))))


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
        # spec 326 R3: the row names its work and its work's commit. The two new columns are APPENDED, so a
        # reader that takes the first six fields positionally keeps working; a missing field is "-", never a
        # blank, because a blank reads as "not applicable" and it is not.
        for r in rows():
            st = load().get(r["id"], {})
            num = r["spec"].split("-")[0] if r["spec"][:1].isdigit() else "-"
            sha = str(st.get("commit") or "-")[:10]
            print("\t".join([r["id"], r["spec"], r["title"], st.get("status", "pending"),
                             str(st.get("attempts", 0)), st.get("last", ""), num, sha]))
        return
    if cmd == "pick":
        st = load()
        skipped = []
        for r in rows():
            s = st.get(r["id"], {}).get("status", "pending")
            # a needs-review row parked by a closed window waits for the clock, not a person: once the window guard lets
            # a cycle start, it is offered again with its session (spec 280). A worker that stopped early (worker-rc=N) left
            # its session for the next cycle to resume, so it is offered again too (spec 312)
            window = resumable(s, st.get(r["id"], {}))
            # a parked / needs-review row waits for a person and must not hold the queue behind it (spec 169):
            # skip it for dispatch order only - its state is never touched - and name it on stderr
            if s in ("parked", "needs-review") and not window:
                skipped.append(f'{r["id"]}({s})')
                continue
            if s in ("pending", "in-progress") or window:
                # spec 283: a row whose column 6 is not its spec's ## Files set is refused before a worker is bought
                sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
                rowpaths = importlib.import_module("row-paths")  # hyphenated file: import_module, not `import`
                rc, lines = rowpaths.judge(r)
                if rc:
                    print(f"refused {r['id']}: {', '.join(lines)}", file=sys.stderr)
                    continue
                # spec 309: a row whose column 5 is not its spec's ## Exit Criteria, command for command, is refused the same way
                rowverify = importlib.import_module("row-verify")  # hyphenated file: import_module, not `import`
                rc, lines = rowverify.judge(r)
                if rc:
                    print(f"refused {r['id']}: {', '.join(lines)}", file=sys.stderr)
                    continue
                if skipped:
                    print("pick: skip parked " + ", ".join(skipped) + " -> dispatching " + r["id"], file=sys.stderr)
                # a row imported from a review is re-verified at pick (doc 066 Q2): a broken anchor is a plan-error
                sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
                import plan_import
                why = plan_import.check_row(r["id"])
                if why:
                    raise SystemExit("plan-error: " + why)
                print("\t".join([r["id"], r["spec"], r["brief"], r["title"], r["verify"], r["paths"], r["gate"],
                                 str(st.get(r["id"], {}).get("attempts", 0)),
                                 st.get(r["id"], {}).get("session", "")]))
                return
        if skipped:
            print("pick: every open row is parked: " + ", ".join(skipped) + " - waiting for a person", file=sys.stderr)
        print("NONE")
        return
    if cmd == "get":
        print(json.dumps(load().get(a[1], {}), indent=1))
        return
    if cmd == "repair":
        # R1: the repair is an operation, not a note. Attempts are untouched: the 3-strikes cap is a safety
        # property, so a repaired row that fails again parks again on its own merits.
        if len(a) < 3:
            raise SystemExit("usage: state.py repair <id> <note...>")
        d = load()
        row = d.get(a[1])
        if row is None:
            raise SystemExit(f"repair: no such row {a[1]}")
        row["last"] = "park-cause-repaired: " + " ".join(a[2:])
        row["status"] = "needs-review" if row.get("session") else "pending"
        save(d)
        print(json.dumps(row, indent=1))
        return
    if cmd == "annotate":
        # spec 326 R2: the two facts only the commit step knows. A field that can hold a lie is worse than a
        # missing one, so both are proven before they are written: the spec file exists, and git knows the sha.
        if len(a) < 4:
            raise SystemExit("usage: state.py annotate <id> [--spec <path>] [--commit <sha>]")
        ident, spec, commit, rest = a[1], None, None, a[2:]
        while rest:
            if rest[0] == "--spec" and len(rest) >= 2:
                spec, rest = rest[1], rest[2:]
            elif rest[0] == "--commit" and len(rest) >= 2:
                commit, rest = rest[1], rest[2:]
            else:
                raise SystemExit(f"annotate: unknown argument {rest[0]!r}")
        d = load()
        row = d.setdefault(ident, {})
        plan_row = next((r for r in rows() if r["id"] == ident), None)
        if spec is not None:
            if not os.path.isfile(os.path.join(ROOT, spec)):
                raise SystemExit(f"annotate: no such spec {spec} — nothing written")
            if plan_row and os.path.basename(spec) != plan_row["spec"]:
                # a row annotated with somebody else's spec is a reference that points at the wrong document
                raise SystemExit(f"annotate: {ident} owns {plan_row['spec']}, not {spec} — nothing written")
            row["spec"] = spec
        if commit is not None:
            ok = os.system(f"git -C {ROOT} cat-file -e {commit}^{{commit}} 2>/dev/null") == 0
            if not ok:
                raise SystemExit(f"annotate: git does not know {commit} — nothing written")
            row["commit"] = commit
        save(d)
        print(json.dumps(row, indent=1, sort_keys=True))
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
