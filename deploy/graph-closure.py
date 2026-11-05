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

# Spec 397: a call cycle across graphs is an infinite recursion wearing composition's clothes. Per-graph
# validation cannot see it — each file on its own is acyclic — and the depth cap turns it into a run that
# recurses four levels and then fails, having spent four dispatches to learn nothing. A downloadable graph
# library makes this likely: two flows that call each other are exactly what a contributor writes.
edges: dict[str, list[str]] = {}
for name in names:
    path = graphs / f"{name}.toml"
    if not path.is_file():
        continue
    try:
        doc = tomllib.loads(path.read_text())
    except tomllib.TOMLDecodeError:
        continue
    edges[name] = [
        n.get("graph", "")
        for n in doc.get("node", [])
        if n.get("kind") == "call" and n.get("graph", "") in names
    ]

WHITE, GREY, BLACK = 0, 1, 2
colour = dict.fromkeys(edges, WHITE)
stack: list[str] = []


def walk(node: str) -> None:
    colour[node] = GREY
    stack.append(node)
    for nxt in edges.get(node, []):
        if colour.get(nxt) == GREY:
            loop = stack[stack.index(nxt) :] + [nxt]
            problems.append("call cycle: " + " -> ".join(loop))
        elif colour.get(nxt) == WHITE:
            walk(nxt)
    stack.pop()
    colour[node] = BLACK


for name in sorted(edges):
    if colour[name] == WHITE:
        walk(name)

for problem in problems:
    print(problem)
sys.exit(1 if problems else 0)
