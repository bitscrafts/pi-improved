#!/usr/bin/env bash
# item-ids.sh [--check | --migrate] — one ledger: an item's id is its work's spec number (spec 328).
#
# Measured 2026-10-31: the queue's rows carry A-numbers and the flow's items carry A-numbers too, but they are
# two separate counters that only ever agreed because a person typed matching names. The queue side was fixed
# by spec 326 (a row's id IS its spec number from then on); this fixes the flow side and migrates what exists.
#
# The rule, per item file in the minilam store:
#   · `id` must equal the file's own name and be the spec number from `data.spec` (specs/320-….md -> 320);
#   · a legacy `A…` id is allowed only if the item DECLARES `data.alias` with that id — visible as history,
#     never as a second identity;
#   · `data.spec` must name a file that exists.
#
# --migrate writes the new file, records `data.alias`, MOVES the old file into /projects/_trash/<date>-item-ids/
# (never deletes anything) and, if the pointer names the old id, repoints it.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

MODE="${1:---check}"
case "$MODE" in --check|--migrate) ;; *) echo "usage: item-ids.sh [--check|--migrate]"; exit 2 ;; esac

python3 - "$MODE" <<'PY'
import json, os, re, shutil, sys, time

mode = sys.argv[1]
store = os.environ.get("MINILAM_DIR") or os.path.join(os.getcwd(), ".minilam")
items_dir = os.path.join(store, "items")
trash = os.environ.get("PII_ITEM_TRASH") or os.path.join("/projects/_trash", time.strftime("%Y%m%d") + "-item-ids")

if not os.path.isdir(items_dir):
    print(f"item-ids: no items directory at {items_dir} — nothing to check")
    sys.exit(0)

bad, ok, migrated = [], 0, []
for name in sorted(os.listdir(items_dir)):
    if not name.endswith(".json"):
        continue
    path = os.path.join(items_dir, name)
    stem = name[:-5]
    d = json.load(open(path))
    spec = (d.get("data") or {}).get("spec") or ""
    num = spec.split("/")[-1].split("-")[0] if spec else ""
    alias = (d.get("data") or {}).get("alias")
    if d.get("id") != stem:
        bad.append(f"{stem}: its id field says {d.get('id')!r} — an item whose name and identity disagree")
        continue
    if not spec:
        bad.append(f"{stem}: declares no data.spec, so it cannot be tied to the work it belongs to")
        continue
    if not os.path.isfile(os.path.join(os.path.dirname(store), spec)) and not os.path.isfile(spec):
        bad.append(f"{stem}: data.spec {spec} does not exist")
        continue
    if not num.isdigit():
        bad.append(f"{stem}: data.spec {spec} does not start with a number")
        continue
    if stem == num:
        ok += 1
        continue
    if alias == stem:
        ok += 1                       # declared history: an A-id that records the spec number as its identity
        continue
    if not re.fullmatch(r"A\d+", stem):
        bad.append(f"{stem}: neither the spec number {num} nor a legacy A-id")
        continue
    if mode == "--check":
        bad.append(f"{stem}: should be {num} (its spec's number), or declare data.alias")
        continue
    target = os.path.join(items_dir, num + ".json")
    if os.path.exists(target):
        bad.append(f"{stem}: cannot become {num} — that item already exists")
        continue
    d["id"] = num
    data = d.setdefault("data", {})
    data["alias"] = stem
    tmp = target + f".tmp.{os.getpid()}"
    with open(tmp, "w") as fh:
        json.dump(d, fh, indent=1, sort_keys=True)
    os.replace(tmp, target)
    os.makedirs(trash, exist_ok=True)
    shutil.move(path, os.path.join(trash, name))   # moved, never deleted
    migrated.append((stem, num))
    # the pointer must not name an id that no longer exists
    pointer = os.path.join(store, "state.json")
    if os.path.isfile(pointer):
        p = json.load(open(pointer))
        if p.get("current") == stem:
            p["current"] = num
            with open(pointer + ".tmp", "w") as fh:
                json.dump(p, fh, indent=1, sort_keys=True)
            os.replace(pointer + ".tmp", pointer)
            print(f"item-ids: pointer moved {stem} -> {num}")

for b in bad:
    print("ITEM-IDS FAIL: " + b)
for old, new in migrated:
    print(f"item-ids: {old} -> {new} (old file moved to {trash})")
print(f"item-ids: {ok} item(s) in step, {len(migrated)} migrated, {len(bad)} violation(s)")
sys.exit(1 if bad else 0)
PY
