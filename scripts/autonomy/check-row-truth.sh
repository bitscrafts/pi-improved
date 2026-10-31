#!/usr/bin/env bash
# check-row-truth.sh — every row that claims a commit is checked against its sources (spec 326 R4).
#
# A denormalized field with no validator is a second opinion. So for every plan row carrying a commit:
#   · the sha exists in git                       (else the row points at nothing)
#   · the spec file exists                       (else the row names a document that is gone)
#   · the row's verify column == the spec's Exit Criteria, command for command, in order (spec 309's rule,
#     applied to the whole plan instead of only to the row being dispatched)
# Rows with no commit yet are counted, not failed: they are simply not yet done.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

python3 - <<'PY'
import importlib, os, re, subprocess, sys

SPECS = os.environ.get("PII_SPECS") or "specs"
sys.path.insert(0, "scripts/autonomy")
state = importlib.import_module("state")
plan_path = os.environ.get("PII_PLAN") or os.path.join(state.ROOT, "scripts", "autonomy", "plan.tsv")


def stored_rows(path):
    """The plan as the FILE says it, not as the dispatcher would derive it (spec 327).

    `state.rows()` derives an open row's verify from its spec, so auditing through it would compare the spec
    with itself and a stale file could never be seen. This checker exists to catch exactly that.
    """
    out = []
    for line in open(path):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) < 7:
            continue
        out.append(dict(zip(("id", "spec", "brief", "title", "verify", "paths", "gate"), f)))
    return out


plan = stored_rows(plan_path)
st = state.load()

bad, audited, historical, unjudged = [], 0, 0, 0
for r in plan:
    row = st.get(r["id"], {})
    sha = row.get("commit")
    status = row.get("status", "pending")
    # WHICH spec is the row judged against? The one the work was actually held to, when we can know it:
    # a spec amended AFTER its item ran must not be reported as the row having dropped a criterion (they look
    # identical when compared to today's file). A finished row with no recorded commit cannot be judged at all.
    if sha:
        audited += 1
        if subprocess.run(["git", "cat-file", "-e", sha + "^{commit}"], capture_output=True).returncode != 0:
            bad.append(f'{r["id"]}: commit {sha} is not in git')
            continue
        show = subprocess.run(["git", "show", f'{sha}:specs/{r["spec"]}'], capture_output=True, text=True)
        if show.returncode != 0:
            bad.append(f'{r["id"]}: specs/{r["spec"]} did not exist at its own commit {sha[:10]}')
            continue
        body, where_from = show.stdout, f"specs/{r['spec']} at {sha[:10]}"
    elif status in ("pending", "in-progress", "needs-review"):
        audited += 1
        path = os.path.join(SPECS, r["spec"])
        if not os.path.isfile(path):
            bad.append(f'{r["id"]}: spec {path} does not exist')
            continue
        body, where_from = open(path).read(), path
    else:
        # done/skipped/revoked without a commit: the spec may have moved since, so no judgement is possible.
        # Counted, never silently counted as a pass.
        historical += 1
        continue
    crit = re.findall(r"- \[ \] `(.+?)`", body)
    cmds = [c.strip() for c in r["verify"].split(";") if c.strip()]
    if crit != cmds:
        where = next((i for i, (a, b) in enumerate(zip(cmds, crit), 1) if a != b), min(len(cmds), len(crit)) + 1)
        bad.append(f'{r["id"]}: criterion {where} differs from {where_from} '
                   f'(row {len(cmds)} commands, spec {len(crit)} criteria)')

for b in bad:
    print("ROW-TRUTH FAIL: " + b)
print(f"row-truth: {audited} row(s) audited, {historical} historical (no commit: not judgeable), {len(bad)} violation(s)")
sys.exit(1 if bad else 0)
PY
