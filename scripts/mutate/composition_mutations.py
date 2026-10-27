"""Spec 286: the composition mutation table, and the applier that writes one mutation into a scratch tree.

Each row is one semantic mutation of the composition code: a single `dict` with an `id`, the `file` it edits (relative to
the tree root), `edits` (a list of `(old, new)` anchors, each of which must occur exactly once in the file before any
edit is made), the `test` that must turn red (`<crate>/<test-file-stem>`, or `""` for the root crate's own tests), and
`kind`: `required` (must turn its test red) or `probe` (reported, never counted as a pass).

Usage (run through `uv run --no-project python`; composition.sh does this):
    composition_mutations.py rows  --rows PATH                     # prints id, kind, file, test, tab-separated
    composition_mutations.py apply --rows PATH --scratch DIR --id ID
        # writes the mutated file under DIR; exits 3 (NO-APPLY) when an anchor is not unique, writing nothing
"""

import argparse
import importlib.util
import pathlib
import sys

KINDS = ("required", "probe")
NO_APPLY = 3

# The composition source is `crates/pii-graph/src/fragment/` (spec 255, 261, 269, 270) and the coder node's side of the
# run budget and session file in `crates/pii-server/src/node_kinds/nodes/coder/` (spec 271, 276).
FRAG = "crates/pii-graph/src/fragment"
CODER = "crates/pii-server/src/node_kinds/nodes/coder"

m01 = dict(id="m01", file=f"{FRAG}/call.rs", edits=[
    ("if actual != sha {", "if actual != sha && false {"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m02 = dict(id="m02", file=f"{FRAG}/call.rs", edits=[
    ('let sha = required(call, &id, "sha256")?;',
     'let sha = call.get("sha256").and_then(Value::as_str).unwrap_or("").to_string();'),
    ("if actual != sha {", "if !sha.is_empty() && actual != sha {"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m03 = dict(id="m03", file=f"{FRAG}/call_needs.rs", edits=[
    ("if host_ids.iter().any(|h| h == *m)", "if false && host_ids.iter().any(|h| h == *m)"),
], test="pii-graph/call_needs", kind="required")

m04 = dict(id="m04", file=f"{FRAG}/call_needs.rs", edits=[
    ("if !declared.iter().any(|d| d == *key) {", "if false {"),
], test="pii-graph/call_needs", kind="required")

m05 = dict(id="m05", file=f"{FRAG}/call_needs.rs", edits=[
    (".filter(|s| !s.is_empty())", ".filter(|_| true)"),
], test="pii-graph/call_needs", kind="required")

m06 = dict(id="m06", file=f"{FRAG}/call.rs", edits=[
    ("const MAX_DEPTH: usize = 4;", "const MAX_DEPTH: usize = 5;"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m07 = dict(id="m07", file=f"{FRAG}/call.rs", edits=[
    ("if chain.contains(&graph) {", "if false && chain.contains(&graph) {"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m08 = dict(id="m08", file=f"{FRAG}/call.rs", edits=[
    ("if !widen.is_empty() {", "if false {"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m09 = dict(id="m09", file=f"{FRAG}/call.rs", edits=[
    ("call_needs::mark(&mut added, std::slice::from_ref(own))?;", "let _ = own;"),
], test="pii-graph/call_records", kind="required")

m10 = dict(id="m10", file="crates/pii-graph/src/run/call_events.rs", edits=[
    ('node.cfg.get("call-from") {', 'node.cfg.get("call-from").filter(|_| false) {'),
], test="pii-graph/run_journals_calls", kind="required")

m11 = dict(id="m11", file=f"{FRAG}/call.rs", edits=[
    ('Value::String(format!("{id}__{nid}"))', "Value::String(nid.clone())"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m12 = dict(id="m12", file=f"{FRAG}/call.rs", edits=[
    ('let input = match call.get("in") {', 'let input = match call.get("in").filter(|_| false) {'),
], test="pii-graph/call_inlines_a_graph", kind="required")

m13 = dict(id="m13", file=f"{FRAG}/call.rs", edits=[
    ('if e.get("from").and_then(Value::as_str) == Some(id) {',
     'if false && e.get("from").and_then(Value::as_str) == Some(id) {'),
], test="pii-graph/call_inlines_a_graph", kind="required")

m14 = dict(id="m14", file=f"{CODER}/budget.rs", edits=[
    ("let _ = ctx.budget().charge(BudgetKind::MicroUsd, micro as u64);", "let _ = micro;"),
], test="pii-server/coder_charges_run_budget", kind="required")

m15 = dict(id="m15", file=f"{CODER}/graph_node.rs", edits=[
    ("if let Some(reason) = start_gate(ctx) {", "if let Some(reason) = start_gate(ctx).filter(|_| false) {"),
], test="pii-server/coder_charges_run_budget", kind="required")

m16 = dict(id="m16", file=f"{CODER}/session_path.rs", edits=[
    ("let session = node_session_path(&spec.workdir, &spec.spec, run, node);",
     'let session = node_session_path(&spec.workdir, &spec.spec, run, "node");'),
], test="pii-server/coder_session_per_node", kind="required")

m17 = dict(id="m17", file="crates/pii-server/src/engine/runs.rs", edits=[
    ("pii_core::Budget::new(\n            spec.limits.max_tokens,\n            spec.limits.max_tool_calls,"
     "\n            spec.limits.max_micro_usd,\n            spec.limits.max_wall_ms,\n        )",
     "pii_core::Budget::unlimited()"),
], test="pii-server/composition_failure_matrix", kind="required")

m18 = dict(id="m18", file=f"{FRAG}/check.rs", edits=[
    ("if super::call::has_call(&doc) {", "if false && super::call::has_call(&doc) {"),
], test="pii-graph/call_inlines_a_graph", kind="required")

m19 = dict(id="m19", file=f"{FRAG}/call_needs.rs", edits=[
    ("if srcs.len() > 1 { srcs } else { Vec::new() }", "if !srcs.is_empty() { srcs } else { Vec::new() }"),
], test="pii-graph/call_needs", kind="probe")

m20 = dict(id="m20", file=f"{FRAG}/call_needs.rs", edits=[
    ('["input"] if scope.needs.is_some() =>', '["input"] if false =>'),
], test="pii-graph/call_needs", kind="probe")

ROWS = [m01, m02, m03, m04, m05, m06, m07, m08, m09, m10, m11, m12, m13, m14, m15, m16, m17, m18, m19, m20]


def load_rows(path):
    """The table at `path`: its `ROWS`, validated. A bad row stops the sweep before anything is written."""
    spec = importlib.util.spec_from_file_location("sweep_rows", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    rows = list(module.ROWS)
    seen = set()
    for row in rows:
        rid = row["id"]
        if rid in seen:
            raise SystemExit(f"table: duplicate id {rid}")
        seen.add(rid)
        if row["kind"] not in KINDS:
            raise SystemExit(f"table: {rid} has kind {row['kind']!r}, not one of {KINDS}")
        if not row["edits"]:
            raise SystemExit(f"table: {rid} has no edits")
        for old, new in row["edits"]:
            if old == new:
                raise SystemExit(f"table: {rid} has an edit that changes nothing")
    return rows


def apply(rows, scratch, rid):
    """Writes row `rid`'s edits into the scratch copy. Every anchor is counted on the untouched text first, so a
    NO-APPLY writes nothing."""
    row = next((r for r in rows if r["id"] == rid), None)
    if row is None:
        raise SystemExit(f"table: no row {rid}")
    target = pathlib.Path(scratch) / row["file"]
    text = target.read_text(encoding="utf-8")
    for old, _ in row["edits"]:
        count = text.count(old)
        if count != 1:
            print(f"NO-APPLY {rid}: anchor occurs {count} times in {row['file']}: {old!r}")
            return NO_APPLY
    for old, new in row["edits"]:
        text = text.replace(old, new, 1)
    target.write_text(text, encoding="utf-8")
    return 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    rows_p = sub.add_parser("rows")
    rows_p.add_argument("--rows", required=True)
    apply_p = sub.add_parser("apply")
    apply_p.add_argument("--rows", required=True)
    apply_p.add_argument("--scratch", required=True)
    apply_p.add_argument("--id", required=True)
    args = parser.parse_args(argv)
    rows = load_rows(args.rows)
    if args.cmd == "rows":
        for row in rows:
            print("\t".join((row["id"], row["kind"], row["file"], row["test"])))
        return 0
    return apply(rows, args.scratch, args.id)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
