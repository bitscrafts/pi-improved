import os, subprocess, sys, pathlib

# The root (spec 405): PII_ROOT when set, else the repository this file lives in. A blank PII_ROOT is refused, as env.sh does.
if "PII_ROOT" in os.environ and not os.environ["PII_ROOT"].strip():
    sys.exit("row-run-criteria: PII_ROOT is set but blank - refusing")
root = pathlib.Path(os.environ.get("PII_ROOT") or pathlib.Path(__file__).resolve().parents[2])
row_id = sys.argv[1]
plan = (root / "scripts/autonomy/plan.tsv").read_text().splitlines()
col = None
for line in plan:
    f = line.split("\t")
    if f and f[0] == row_id:
        col = f[4]
        break
if col is None:
    sys.exit(f"no row {row_id}")

crits = [c for c in col.split(";") if c.strip()]
print(f"{row_id}: {len(crits)} criteria, EXECUTED")
fail = 0
for c in crits:
    r = subprocess.run(["bash", "-c", c], cwd=str(root), capture_output=True, text=True)
    ok = r.returncode == 0
    if not ok:
        fail += 1
    print(f"  {'PASS' if ok else 'FAIL'} rc={r.returncode}  {c[:88]}")
print(f"RESULT: {len(crits) - fail} passed, {fail} failed")
sys.exit(1 if fail else 0)
