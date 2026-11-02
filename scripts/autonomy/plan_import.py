#!/usr/bin/env python3
"""plan_import.py — copies a review's ```queue blocks into plan.tsv rows. It copies; it never invents (doc 066).

usage:
  plan_import.py import <doc>... [--approved <id>]   blocks -> generated spec + row + provenance line
  plan_import.py check                               every row's verify/paths against its spec, every anchor
  plan_import.py --self-test                         runs test_plan_import.py (fixtures only)

exit: 0 ok / already queued, 2 refused (named reason on stderr), 3 escalated (one questions.md line), 75 lock held.
Paths are overridable for tests: PII_ROOT PII_PLAN PII_PROVENANCE PII_QUESTIONS PII_SPECS PII_LOCK PII_ACTS.
A block: ```queue fence of `key: value` lines (id rev title class spec_slug files criteria evidence* body); `files` and
`criteria` are `;`-separated; an evidence line is `path:line "exact text"`, resolved at `rev` (git show), never the tree.
"""
import fcntl, fnmatch, hashlib, os, re, shlex, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("PII_ROOT") or os.path.dirname(os.path.dirname(HERE))
GATE = "quality-gate.sh"
ALLOWED = ("crates/", "xtask/", "tests/", "scripts/checks/")
DECISION = ("deploy/", "/root/.hermes/", "~/.pii/", "scripts/autonomy/", "specs/")
DECISION_NAMES = ("write_amp.rs", "coverage-floor.txt", "gate-steps.txt", "config.rs")
DECISION_GLOBS = ("*.service", "*_budget*", "*limit*", "*secret*")
BUILTIN = {"test", "cd", "echo", "true", "false", "diff", "grep", "[", "!"}


class Refuse(Exception):
    def __init__(self, msg, code=2):
        super().__init__(msg)
        self.code = code


def p(env, *default):
    return os.environ.get(env) or os.path.join(ROOT, *default)


def paths():
    return dict(plan=p("PII_PLAN", "scripts", "autonomy", "plan.tsv"),
                prov=p("PII_PROVENANCE", "scripts", "autonomy", "plan.provenance.tsv"),
                q=p("PII_QUESTIONS", "_tmp", "autonomy", "questions.md"),
                specs=p("PII_SPECS", "specs"),
                lock=p("PII_LOCK", "_tmp", "autonomy", "lock"),
                acts=p("PII_ACTS", "_tmp", "autonomy", "operator-acts.log"))


def sha(s):
    return hashlib.sha256(s.encode()).hexdigest()[:12]


def parse_blocks(text):
    out = []
    for m in re.finditer(r"^```queue[ \t]*\n(.*?)^```[ \t]*$", text, re.S | re.M):
        raw = m.group(1)
        b = {"evidence": [], "_sha": sha(raw)}
        for line in raw.splitlines():
            k, _, v = line.partition(":")
            k, v = k.strip(), v.strip()
            if not re.fullmatch(r"[a-z_]+", k):
                continue
            if k == "evidence":
                b["evidence"].append(v)
            else:
                b[k] = v
        out.append(b)
    return out


def split(v):
    return [x.strip() for x in v.split(";") if x.strip()]


def read_at(rev, path):
    """lines of path at rev (git show), or the file under ROOT when rev is empty/WORKTREE; None if absent."""
    if rev and rev != "WORKTREE":
        r = subprocess.run(["git", "-C", ROOT, "show", f"{rev}:{path}"], capture_output=True, text=True)
        return r.stdout.splitlines() if r.returncode == 0 else None
    f = os.path.join(ROOT, path)
    return open(f).read().splitlines() if os.path.isfile(f) else None


def anchor(rev, ev):
    """verify one `path:line "text"`; returns (path:line, sha of the line) or raises Refuse."""
    m = re.fullmatch(r'(\S+?):(\d+)\s+"(.*)"', ev)
    if not m:
        raise Refuse(f"evidence is not path:line \"text\": {ev}")
    path, n, text = m.group(1), int(m.group(2)), m.group(3)
    lines = read_at(rev, path)
    if lines is None:
        raise Refuse(f"evidence missing: {path}")
    base = os.path.basename(path)
    if n <= len(lines) and text in lines[n - 1]:
        return f"{path}:{n}", sha(lines[n - 1])
    for i, ln in enumerate(lines, 1):
        if text in ln:
            raise Refuse(f"moved: {base}:{n} -> :{i}")
    raise Refuse(f'stale: {base}:{n} no longer reads "{text}"')


def is_command(c):
    try:
        w = shlex.split(c)
    except ValueError:
        return False
    return bool(w) and (w[0] in BUILTIN or shutil.which(w[0]) is not None or w[0].startswith(("./", "/", "<(")))


def decision_reason(b):
    """the path that forces a human, or None. The label is never consulted for the list (label-independent)."""
    if b.get("class", "") != "mechanical":
        return f"class is {b.get('class') or 'unset'}"
    files = split(b.get("files", ""))
    if not files:
        return "no files (a host act)"
    for f in files:
        add_only = f.endswith("(add)")
        path = f.replace("(add)", "").replace("(remove)", "").strip()
        base = os.path.basename(path)
        if any(path.startswith(d) or d in path for d in DECISION) or any(fnmatch.fnmatch(base, g) for g in DECISION_GLOBS):
            return path
        if base in DECISION_NAMES and not (base == "gate-steps.txt" and add_only):
            return path
        if not path.startswith(ALLOWED) and not (base == "gate-steps.txt" and add_only):
            return path
    return None


def clean_files(b):
    return [f.replace("(add)", "").replace("(remove)", "").strip() for f in split(b.get("files", ""))]


def render(b, doc, n):
    crit = "\n".join(f"- [ ] `{c}`" for c in split(b["criteria"]))
    files = "\n".join(f"- {f}" for f in clean_files(b))
    return (f"# Spec: {b['title']}\n\n## Overview\n\n{b['title']}\n\n## Requirement\n\n{b['body']}\n\n"
            f"## Files to Modify\n\n{files}\n\n## Exit Criteria\n\n{crit}\n\n"
            "## Guardrails\n\n- Do NOT weaken the gate, a budget or any existing control to make this pass.\n"
            "- Do NOT commit; the loop's orchestrator reviews the diff and commits.\n\n"
            f"## Source\n\n{doc} item {b['id']} (block {b['_sha']}); generated by plan_import.py, do not hand-edit.\n")


def load_prov(f):
    rows = []
    if os.path.isfile(f):
        for ln in open(f):
            if ln.strip():
                rows.append(ln.rstrip("\n").split("\t"))
    return rows


def plan_rows(f):
    out = []
    for ln in open(f) if os.path.isfile(f) else []:
        if ln.strip() and not ln.startswith("#"):
            out.append(ln.rstrip("\n").split("\t"))
    return out


def log_ask(P, b, why):
    line = f'- [ ] queue {b["id"]} "{b.get("title", "")}"? touches {why}; yes|no'
    cur = open(P["q"]).read() if os.path.isfile(P["q"]) else ""
    if f"queue {b['id']} " not in cur:
        os.makedirs(os.path.dirname(P["q"]), exist_ok=True)
        with open(P["q"], "a") as fh:
            fh.write(line + "\n")


def import_block(b, doc, P, approved):
    bid = b.get("id", "?")
    for k in ("id", "title", "spec_slug", "files", "criteria", "body"):
        if not b.get(k):
            raise Refuse(f"block missing {k}: {bid}")
    prov = load_prov(P["prov"])
    for r in prov:
        if len(r) > 2 and r[2] == bid:
            if r[0] == "DECLINED":
                return None
            if r[3] == f"block_sha256={b['_sha']}":
                print(f"already queued: {bid} as {r[0]}")
                return None
            raise Refuse(f"duplicate: {bid} already queued as {r[0]}")
    if not b["evidence"]:
        raise Refuse(f"no evidence: {bid}")
    anchors = [anchor(b.get("rev", ""), e) for e in b["evidence"]]
    for c in split(b["criteria"]):
        if not is_command(c):
            raise Refuse(f"criterion is not a command: {bid}")
    existing = [f for f in os.listdir(P["specs"])] if os.path.isdir(P["specs"]) else []
    slug = b["spec_slug"]
    for f in existing + [r[1] for r in plan_rows(P["plan"])]:
        if re.fullmatch(rf"\d+-{re.escape(slug)}\.md", f):
            raise Refuse(f"duplicate spec slug: {slug} (specs/{f})")
    why = decision_reason(b)
    if why and bid != approved:
        log_ask(P, b, why)
        raise Refuse(f"escalated: {bid} touches {why}", 3)
    rows = plan_rows(P["plan"])
    nums = [int(m.group(1)) for r in rows for m in [re.match(r"[A-Za-z]*(\d+)$", r[0])] if m]
    ids = [r[0] for r in rows]
    rid = f"A{max(nums, default=0) + 1}"
    specn = max([int(m.group(1)) for f in existing + [r[1] for r in rows] for m in [re.match(r"(\d+)-", f)] if m] or [0]) + 1
    spec = f"{specn}-{slug}.md"
    assert rid not in ids
    os.makedirs(P["specs"], exist_ok=True)
    with open(os.path.join(P["specs"], spec), "w") as fh:
        fh.write(render(b, doc, specn))
    row = [rid, spec, "-", b["title"], ";".join(split(b["criteria"])), ";".join(clean_files(b)), GATE]
    with open(P["plan"], "a") as fh:
        fh.write("\t".join(row) + "\n")
    with open(P["prov"], "a") as fh:
        fh.write("\t".join([rid, doc, bid, f"block_sha256={b['_sha']}", f"rev={b.get('rev', '')}"]
                           + [f"{a}={h}" for a, h in anchors] + [f"spec={spec}"]) + "\n")
    if approved == bid:
        os.makedirs(os.path.dirname(P["acts"]), exist_ok=True)
        with open(P["acts"], "a") as fh:
            fh.write(f"approved | human | queue {bid} -> {rid}\n")
    print(f"added\t{bid}\t{rid}\t{os.path.relpath(os.path.join(P['specs'], spec), ROOT)}")
    return rid


def cmd_import(args):
    P = paths()
    approved = None
    docs = []
    it = iter(args)
    for a in it:
        if a == "--approved":
            approved = next(it, None)
        else:
            docs.append(a)
    os.makedirs(os.path.dirname(P["lock"]), exist_ok=True)
    lk = open(P["lock"], "a")
    try:
        fcntl.flock(lk, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        print("lock held: nothing read", file=sys.stderr)
        return 75
    worst = 0
    for d in docs:
        if not os.path.isfile(d):
            continue
        for b in parse_blocks(open(d).read()):
            try:
                import_block(b, os.path.relpath(os.path.abspath(d), ROOT), P, approved)
            except Refuse as e:
                print(str(e), file=sys.stderr)
                worst = max(worst, e.code)
    return worst


def section(text, name):
    m = re.search(rf"^## {name}[^\n]*\n(.*?)(?=^## |\Z)", text, re.S | re.M)
    return m.group(1) if m else None


def check_row(rid, P=None):
    """None when the row's provenance holds, else the reason. Rows with no provenance line are not judged here."""
    P = P or paths()
    for r in load_prov(P["prov"]):
        if r[0] != rid:
            continue
        doc, bid = r[1], r[2]
        try:
            blocks = [b for b in parse_blocks(open(os.path.join(ROOT, doc)).read()) if b.get("id") == bid]
        except OSError:
            return f"{rid} review {doc} is gone"
        if not blocks or f"block_sha256={blocks[0]['_sha']}" != r[3]:
            return f"{rid} block {bid} in {doc} changed since import"
        b = blocks[0]
        for e in b["evidence"]:
            try:
                anchor("HEAD" if b.get("rev") else "", e)
            except Refuse as x:
                return f"{rid} evidence {e.split(' ')[0]} {x}"
        spec = r[-1].split("=", 1)[1]
        try:
            have = open(os.path.join(P["specs"], spec)).read()
        except OSError:
            return f"{rid} spec {spec} is gone"
        n = int(spec.split("-")[0])
        if have != render(b, doc, n):
            return f"{rid} specs/{n} differs from its render of {bid}"
    return None


def cmd_check():
    P = paths()
    bad = []
    for r in plan_rows(P["plan"]):
        rid, spec, verify, paths_ = r[0], r[1], r[4], r[5]
        n = spec.split("-")[0]
        try:
            text = open(os.path.join(P["specs"], spec)).read()
        except OSError:
            bad.append(f"{rid} spec {spec} is missing")
            continue
        ec = section(text, "Exit Criteria") or ""
        crit = [re.sub(r"^- \[.\] ", "", ln).strip().strip("`") for ln in ec.splitlines() if re.match(r"- \[.\] ", ln)]
        if verify != "-" and crit and ";".join(crit) != verify:
            bad.append(f"{rid} verify differs from specs/{n} exit criteria")
        fs = section(text, "Files to Modify")
        if fs is None:
            bad.append(f"{rid} commit paths do not cover the spec's files (spec has no files section)")
        elif paths_ != "-":
            cover = [x for x in paths_.split(";") if x]
            miss = [f for f in re.findall(r"^- (\S+)", fs, re.M) if not any(f == c or f.startswith(c.rstrip("/") + "/") for c in cover)]
            if miss:
                bad.append(f"{rid} commit paths do not cover the spec's files: {', '.join(miss)}")
        why = check_row(rid, P)
        if why:
            bad.append(f"plan-error: {why}")
    for b in bad:
        print(b, file=sys.stderr)
    return 2 if bad else 0


def main():
    a = sys.argv[1:]
    if a[:1] == ["--self-test"]:
        return subprocess.call([sys.executable, "-W", "ignore", "-m", "unittest", "-q", "test_plan_import"], cwd=HERE)
    if a[:1] == ["import"]:
        return cmd_import(a[1:])
    if a[:1] == ["check"]:
        return cmd_check()
    print(__doc__, file=sys.stderr)
    return 64


if __name__ == "__main__":
    sys.exit(main())
