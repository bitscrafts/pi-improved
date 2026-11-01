#!/usr/bin/env python3
"""Spec 329: the config promotion behind deploy/install-pii.sh (sections 4 and 4a).

  check   FILE                        FILE parses as TOML; else name the error and exit 1
  merge   --template T --out O [--prior P] [--set TABLE.KEY=VALUE]... [--block FILE]...
                                      O = the operator's file P (the template when there is none), plus the template's
                                      tables and active keys that P lacks, the installer's own keys (--set), and each
                                      block whose [[name]] is not already present (a block holds one [[name]] entry)
  verify  ASIDE CANDIDATE             every table and key path of ASIDE is in CANDIDATE; else name each miss, exit 1

Only tomllib reads TOML. Edits are line-based and keep the operator's text: comments, order and values survive.
"""
import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

HEADER = re.compile(r"^\[(?!\[)\s*([^\]]+?)\s*\]")


def load(path):
    try:
        return tomllib.loads(Path(path).read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        print(f"ABORT: {path} does not parse as TOML: {e}", file=sys.stderr)
        sys.exit(1)


def header_index(lines, table):
    for i, line in enumerate(lines):
        m = HEADER.match(line)
        if m and m.group(1) == table:
            return i
    return None


def section_end(lines, start):
    for j in range(start + 1, len(lines)):
        if lines[j].startswith("["):
            return j
    return len(lines)


def set_key(text, table, key, rhs):
    """Replace the table's `key` line (active or commented out); else add it at the end of the table."""
    lines = text.split("\n")
    line = f"{key} = {rhs}"
    start = header_index(lines, table)
    if start is None:
        return text.rstrip("\n") + f"\n\n[{table}]\n{line}\n"
    end = section_end(lines, start)
    pat = re.compile(rf"^#?\s*{re.escape(key)}\s*=")
    for i in range(start + 1, end):
        if pat.match(lines[i]):
            lines[i] = line
            return "\n".join(lines)
    at = end
    while at > start + 1 and not lines[at - 1].strip():
        at -= 1
    lines.insert(at, line)
    return "\n".join(lines)


def template_section(tlines, table):
    start = header_index(tlines, table)
    return tlines[start:section_end(tlines, start)]


def template_rhs(tlines, table, key):
    start = header_index(tlines, table)
    for line in tlines[start + 1:section_end(tlines, start)]:
        m = re.match(rf"^{re.escape(key)}\s*=\s*(.*)$", line)
        if m:
            return m.group(1)
    return None


def cmd_check(a):
    load(a.file)
    return 0


def cmd_merge(a):
    template_text = Path(a.template).read_text(encoding="utf-8")
    tdoc = tomllib.loads(template_text)
    tlines = template_text.split("\n")
    text = Path(a.prior).read_text(encoding="utf-8") if a.prior else template_text

    # R2: a table the operator's file lacks comes from the template, whole.
    for table, value in tdoc.items():
        if isinstance(value, dict) and table not in tomllib.loads(text):
            text = text.rstrip("\n") + "\n\n" + "\n".join(template_section(tlines, table)).rstrip("\n") + "\n"

    # R2: an active template key that the operator's table lacks is added from the template.
    base = tomllib.loads(text)
    for table, value in tdoc.items():
        if not isinstance(value, dict) or not isinstance(base.get(table), dict):
            continue
        for key, tval in value.items():
            if isinstance(tval, dict) or key in base[table]:
                continue
            rhs = template_rhs(tlines, table, key)
            if rhs is not None:
                text = set_key(text, table, key, rhs)

    # R1: the keys the installer owns, edited in place when present and appended when absent.
    for spec in a.set or []:
        target, _, value = spec.partition("=")
        table, _, key = target.partition(".")
        text = set_key(text, table, key, json.dumps(value))

    # R3: a block is appended only when its [[name]] is not already present.
    for path in a.block or []:
        frag = Path(path).read_text(encoding="utf-8")
        fdoc = tomllib.loads(frag)
        have = tomllib.loads(text)
        wanted = False
        for name, items in fdoc.items():
            if isinstance(items, list):
                present = {x.get("name") for x in have.get(name, [])}
                wanted = wanted or any(x.get("name") not in present for x in items)
        if wanted:
            text = text.rstrip("\n") + "\n\n" + frag.strip("\n") + "\n"

    tomllib.loads(text)  # the candidate must parse; a broken edit stops here, before anything is written
    Path(a.out).write_text(text, encoding="utf-8")
    return 0


def paths(node, prefix=()):
    out = {prefix}
    if isinstance(node, dict):
        for k, v in node.items():
            out |= paths(v, prefix + (k,))
    elif isinstance(node, list):
        for i, v in enumerate(node):
            out |= paths(v, prefix + (i,))
    return out


def cmd_verify(a):
    old = paths(load(a.aside))
    new = paths(load(a.candidate))
    miss = old - new
    if not miss:
        print(f"VERIFIED: {len(old) - 1} key paths of {a.aside} are in {a.candidate}")
        return 0
    lost_tables = {p[0] for p in miss if len(p) == 1}
    for p in sorted(miss, key=str):
        if not p:
            continue
        if len(p) == 1:
            print(f"MISSING TABLE {p[0]}", file=sys.stderr)
        elif p[0] not in lost_tables:
            print(f"MISSING KEY {'.'.join(str(x) for x in p)}", file=sys.stderr)
    return 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("file")
    m = sub.add_parser("merge")
    m.add_argument("--template", required=True)
    m.add_argument("--prior")
    m.add_argument("--out", required=True)
    m.add_argument("--set", action="append")
    m.add_argument("--block", action="append")
    v = sub.add_parser("verify")
    v.add_argument("aside")
    v.add_argument("candidate")
    a = ap.parse_args(argv)
    return {"check": cmd_check, "merge": cmd_merge, "verify": cmd_verify}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
