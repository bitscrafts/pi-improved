#!/usr/bin/env python3
"""Stands in for jq on hosts without it (spec 226 M0). Supports exactly the
invocations the M0 script makes:
    m0-jq.py -r '<path> // empty' [file]
    m0-jq.py -r '<path>'          [file]
<path> is dot-separated; missing keys exit 5 (jq's missing-key behaviour is an
error too). With no file, stdin is read.
"""
import json
import sys


def main() -> int:
    args = sys.argv[1:]
    mode = False
    if args and args[0] == "-r":
        mode = True
        args = args[1:]
    # The filter is quoted by the caller; strip the quotes before treating
    # the argv tail as the file argument.
    path = args[0].strip('"').strip("'") if args else "."
    files = [a for a in args[1:]]
    path = path.replace(" // empty", "")
    if files:
        data = open(files[0]).read()
    else:
        data = sys.stdin.read()
    try:
        obj = json.loads(data)
    except Exception:
        return 4
    if path == ".":
        print(json.dumps(obj))
        return 0
    cur: object = obj
    for part in path.strip(".").split("."):
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        else:
            return 5
    if isinstance(cur, (dict, list)):
        print(json.dumps(cur))
    elif cur is None:
        print("" if mode else "null")
    else:
        print(cur if mode else json.dumps(cur))
    return 0


if __name__ == "__main__":
    sys.exit(main())
