#!/usr/bin/env python3
# Spec 384 R4: the shipped graph set is closed and pinned. Usage: graph-closure.py <graphs-dir> <name>...
# Every named graph must parse, every `call` target must be named too, and every pin must be the sha256 of the file on disk.
# Prints one problem per line and exits 1 when any is found; exits 0 silently when the set is closed.
import hashlib
import pathlib
import sys
import tomllib

graphs = pathlib.Path(sys.argv[1])
names = sys.argv[2:]
problems = []


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


for name in names:
    path = graphs / f"{name}.toml"
    if not path.is_file():
        problems.append(f"{name}: {path} is missing")
        continue
    try:
        doc = tomllib.loads(path.read_text())
    except tomllib.TOMLDecodeError as e:
        problems.append(f"{name}: does not parse: {e}")
        continue
    for node in doc.get("node", []):
        if node.get("kind") != "call":
            continue
        target = node.get("graph", "")
        if target not in names:
            problems.append(f"{name} calls '{target}', which the manifest does not name")
            continue
        actual = digest(graphs / f"{target}.toml")
        if node.get("sha256", "") != actual:
            problems.append(f"{name} pins '{target}' at {node.get('sha256', '')}, but the file is {actual}")

for problem in problems:
    print(problem)
sys.exit(1 if problems else 0)
