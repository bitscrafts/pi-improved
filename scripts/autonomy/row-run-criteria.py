import subprocess, sys, pathlib

root = pathlib.Path("/projects/pi-improved")
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
