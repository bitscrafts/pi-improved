#!/usr/bin/env python3
"""register-157 — put spec 157 in the loop's queue (A24), so the pipeline has an item again.

Why a script and not an inline heredoc: HARD RULE 0d — a dispatch-path change must be a named, re-runnable
artefact, and the queue row is exactly that. Idempotent: running it twice appends nothing.
"""
import pathlib

SPEC = "157-the-deployed-daemon-posts-to-the-channel.md"
ROW = "\t".join([
    "A24",
    SPEC,
    "-",
    "the deployed daemon posts to the operator's channel (M5's second half)",
    "cargo test -p pii-channels --test channel_wiring;bash scripts/checks/no-secret-in-tree.sh",
    "crates/pii-channels;crates/pii-server;deploy;specs",
    "quality-gate.sh",
])

plan = pathlib.Path("scripts/autonomy/plan.tsv")
rows = plan.read_text().rstrip("\n").split("\n")
if any(r.startswith("A24\t") for r in rows):
    print("A24 already present — nothing appended")
else:
    rows.append(ROW)
    plan.write_text("\n".join(rows) + "\n")
    print("appended A24 ->", SPEC)
