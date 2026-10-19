#!/usr/bin/env -S uv run
"""pii-index-code.py — pii indexes its OWN codebase into its memory graph.

User directive 2026-10-19: "the graphify feature! initially indexing (and making indexed) its own codebase".

Measured before writing this: `grep -rln graphify` over the repo finds NOTHING outside _tmp — pii has entity/link/neighbours
verbs and a graph lane in search, but it had never indexed a single line of its own source. So "pii knows its code" was false.

What it emits, per Rust source file
    entity kind=module   name=<crate>::<module path>      source=<file>:1     attr lines=<n> crate=<c>
    entity kind=fn       name=<crate>::<mod>::<fn>        source=<file>:<line> attr pub=<bool>
    entity kind=struct|trait|enum   name=<crate>::<mod>::<name>  source=<file>:<line>

Why entities and not memory documents: the GRAPH lane (bm25 + vector + graph) expands a query through entity relationships.
A document is recallable; an entity is traversable. The code has to be the second kind or `neighbours` can never answer
"what calls journal.commit".

Usage
    uv run scripts/pii-index-code.py --dry-run --crate pii-graph
    uv run scripts/pii-index-code.py --crate pii-graph        # one crate
    uv run scripts/pii-index-code.py                          # every crate, capped at --max entities
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PII = os.environ.get("PII_BIN", "/root/.local/bin/pii")

RE_FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)")
RE_TYPE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(struct|trait|enum)\s+([A-Za-z_][A-Za-z0-9_]*)")


def module_path(crate: str, f: pathlib.Path) -> str:
    rel = f.relative_to(ROOT / "crates" / crate / "src")
    parts = list(rel.with_suffix("").parts)
    if parts and parts[-1] in ("mod", "lib", "main"):
        parts = parts[:-1]
    return "::".join([crate, *parts]) if parts else crate


def scan(crate: str, limit: int):
    out = []
    for f in sorted((ROOT / "crates" / crate / "src").rglob("*.rs")):
        text = f.read_text(errors="replace").splitlines()
        rel = f.relative_to(ROOT)
        mod = module_path(crate, f)
        out.append(("module", mod, f"{rel}:1", {"crate": crate, "lines": str(len(text))}))
        for i, line in enumerate(text, 1):
            m = RE_TYPE.match(line)
            if m:
                out.append((m.group(1), f"{mod}::{m.group(2)}", f"{rel}:{i}", {"crate": crate}))
                continue
            m = RE_FN.match(line)
            if m:
                out.append(("fn", f"{mod}::{m.group(1)}", f"{rel}:{i}",
                            {"crate": crate, "pub": str("pub" in line.split("fn")[0])}))
            if len(out) >= limit:
                return out
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--crate", default=None)
    ap.add_argument("--max", type=int, default=1500)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    crates = [a.crate] if a.crate else sorted(p.name for p in (ROOT / "crates").iterdir()
                                              if p.is_dir() and (p / "src").is_dir())
    made = failed = 0
    for crate in crates:
        items = scan(crate, a.max - made)
        for kind, name, src, attrs in items:
            if a.dry_run:
                print(f"[dry] {kind:8} {name:60} {src}")
                made += 1
                continue
            cmd = [PII, "memory", "entity", "add", "--kind", kind, "--name", name, "--source", f"file:{src}"]
            for k, v in attrs.items():
                cmd += ["--attr", f"{k}={v}"]
            r = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
            if r.returncode == 0:
                made += 1
            else:
                failed += 1
                if failed <= 3:
                    print(f"FAIL {kind} {name}: {(r.stderr or r.stdout).strip()[:140]}", file=sys.stderr)
            if made + failed >= a.max:
                break
        if made + failed >= a.max:
            break
    print(f"index-code: entities={made} failed={failed} crates={len(crates)}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
