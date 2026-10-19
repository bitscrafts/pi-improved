#!/usr/bin/env -S uv run
"""pii-ingest.py — feed the implementation record INTO pii's own memory, structured.

User directive 2026-10-19: "we do need all pii implementation inside pii memory! read the implementation pipeline and feed
everything to pii! specs, handoff all in a structured way".

What it ingests
    specs/*.md          one document per spec: number, slug, title, status (from the queue's state), overview, exit criteria
    HANDOFF.md          one document per section (split on '## ')
    _docs/05-operations one document per operational doc (interim headers only — the first paragraph)

Shape of every document (so recall is useful, not a wall of text)
    "PII-<KIND> <id> | status=<s> | <title>\n<fields...>\n<first 600 chars of prose>"

De-duplication
    The daemon's put has no key, so identity is client-side: a sha256 of the exact text, kept in
    scripts/autonomy/ingest-ledger.json. Re-running after nothing changed writes NOTHING. Changed text writes once.

Usage
    uv run scripts/pii-ingest.py --dry-run          # what would be written, no calls
    uv run scripts/pii-ingest.py                    # specs + handoff + ops docs
    uv run scripts/pii-ingest.py --only specs       # one source
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LEDGER = ROOT / "scripts" / "autonomy" / "ingest-ledger.json"
PII = os.environ.get("PII_BIN", "/root/.local/bin/pii")


def state() -> dict:
    p = ROOT / "_tmp" / "autonomy" / "state.json"
    try:
        return json.loads(p.read_text())
    except Exception:
        return {}


def row_for_spec(spec_name: str, st: dict, plan: list[list[str]]) -> tuple[str, str]:
    for f in plan:
        if len(f) > 1 and f[1] == spec_name:
            v = st.get(f[0]) if isinstance(st.get(f[0]), dict) else {}
            return f[0], v.get("status", "pending")
    return "-", "no-row"


def plan_rows() -> list[list[str]]:
    p = ROOT / "scripts" / "autonomy" / "plan.tsv"
    return [l.rstrip("\n").split("\t") for l in p.read_text().splitlines() if l.startswith("A")]


def spec_docs(st: dict, plan: list[list[str]]):
    for spec in sorted((ROOT / "specs").glob("*.md")):
        text = spec.read_text(errors="replace")
        title = next((l.lstrip("# ").strip() for l in text.splitlines() if l.startswith("# ")), spec.stem)
        m = re.match(r"(\d+)-", spec.name)
        num = m.group(1) if m else "?"
        row, status = row_for_spec(spec.name, st, plan)
        crit = [l.strip("- [ ] ").strip() for l in text.splitlines() if l.strip().startswith("- [ ]")]
        guard = [l.strip() for l in text.splitlines() if l.strip().startswith("- Do NOT")]
        body = re.sub(r"\s+", " ", text.split("## Overview", 1)[-1].split("##", 1)[0]).strip()[:600]
        doc = (
            f"PII-SPEC {num} | status={status} | row={row} | {title}\n"
            f"file=specs/{spec.name} criteria={len(crit)}\n"
            + "".join(f"criterion: {c[:160]}\n" for c in crit[:6])
            + "".join(f"guardrail: {g[:120]}\n" for g in guard[:4])
            + f"body: {body}"
        )
        # key by FILE, not by number: specs 03-* exist three times, and a numeric key made the ledger hold only the
        # last hash, so every re-run rewrote the earlier ones.
        yield f"spec-{spec.stem}", "spec", doc


def handoff_docs():
    p = ROOT / "HANDOFF.md"
    if not p.exists():
        return
    parts = re.split(r"(?m)^## ", p.read_text(errors="replace"))
    for part in parts[1:]:
        head, _, rest = part.partition("\n")
        body = re.sub(r"\s+", " ", rest).strip()[:600]
        if not body:
            continue
        yield f"handoff-{hashlib.sha256(head.encode()).hexdigest()[:8]}", "handoff", (
            f"PII-HANDOFF | {head.strip()}\nbody: {body}"
        )


def ops_docs():
    d = ROOT / "_docs" / "05-operations"
    if not d.exists():
        return
    for f in sorted(d.glob("*.md")):
        text = f.read_text(errors="replace")
        head = next((l.lstrip("# ").strip() for l in text.splitlines() if l.startswith("# ")), f.stem)
        body = re.sub(r"\s+", " ", text).strip()[:600]
        yield f"ops-{f.stem[:40]}", "ops", f"PII-OPS | {head}\nfile=_docs/05-operations/{f.name}\nbody: {body}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["specs", "handoff", "ops"], default=None)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else {}
    st = state()
    plan = plan_rows()

    sources = {"specs": lambda: spec_docs(st, plan), "handoff": handoff_docs, "ops": ops_docs}
    picked = [a.only] if a.only else list(sources)

    written = skipped = failed = 0
    for name in picked:
        for key, kind, doc in sources[name]():
            h = hashlib.sha256(doc.encode()).hexdigest()
            if ledger.get(key) == h:
                skipped += 1
                continue
            if a.dry_run:
                print(f"[dry] {kind:8} {key:34} {doc.splitlines()[0][:90]}")
                written += 1
                continue
            r = subprocess.run([PII, "memory", "put", doc, "--kind", kind],
                               capture_output=True, text=True, cwd=ROOT)
            if r.returncode != 0:
                failed += 1
                print(f"FAIL {kind} {key}: {(r.stderr or r.stdout).strip()[:160]}", file=sys.stderr)
                continue
            ledger[key] = h
            written += 1

    if not a.dry_run:
        LEDGER.parent.mkdir(parents=True, exist_ok=True)
        LEDGER.write_text(json.dumps(ledger, indent=0, sort_keys=True))
    print(f"ingest: written={written} unchanged={skipped} failed={failed} ledger={LEDGER.relative_to(ROOT)}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
