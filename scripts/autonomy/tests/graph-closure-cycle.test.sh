#!/usr/bin/env bash
# Spec 397: a call cycle across graphs is refused by the closure check, and an acyclic set still passes.
# Two flows calling each other is exactly what a contributor to a downloadable graph library writes, and
# no per-graph check can see it: each file on its own is acyclic.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
CLOSURE="$ROOT/deploy/graph-closure.py"
W=$(mktemp -d)
trap 'mv "$W" "$ROOT/_trash/graph-closure-cycle-$(date +%s)" 2>/dev/null || true' EXIT
G="$W/graphs"
mkdir -p "$G"
ok=0
fail=0

check() { # name expected-rc expected-text
  local name=$1 want_rc=$2 want=$3 out rc
  out=$(python3 "$CLOSURE" "$G" a b 2>&1); rc=$?
  if [ "$rc" = "$want_rc" ] && printf '%s' "$out" | grep -qF "$want"; then
    echo "ok   $name"; ok=$((ok+1))
  else
    echo "FAIL $name (rc=$rc, want=$want_rc, out=$out)"; fail=$((fail+1))
  fi
}

digest() { python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"; }

# a calls b
printf 'name = "a"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "call"\ngraph = "b"\nsha256 = "%s"\n' "$(digest "$G/b.toml" 2>/dev/null || echo x)" > "$G/a.toml"
# b calls a  -> the cycle
printf 'name = "b"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "call"\ngraph = "a"\nsha256 = "%s"\n' "$(digest "$G/a.toml" 2>/dev/null || echo x)" > "$G/b.toml"
# re-write both now that each other's digest exists, so the only problem left is the cycle
printf 'name = "a"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "call"\ngraph = "b"\nsha256 = "%s"\n' "$(digest "$G/b.toml")" > "$G/a.toml"
printf 'name = "b"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "call"\ngraph = "a"\nsha256 = "%s"\n' "$(digest "$G/a.toml")" > "$G/b.toml"

check "two flows that call each other are refused, and the cycle is named" 1 "call cycle: a -> b -> a"

# control: b stops calling a. Same two files, same pins, no cycle.
printf 'name = "b"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "transform"\nvalue = 1\n' > "$G/b.toml"
printf 'name = "a"\nentry = "go"\nexit = "go"\n\n[[node]]\nid = "go"\nkind = "call"\ngraph = "b"\nsha256 = "%s"\n' "$(digest "$G/b.toml")" > "$G/a.toml"
out=$(python3 "$CLOSURE" "$G" a b 2>&1); rc=$?
if [ "$rc" = 0 ] && [ -z "$out" ]; then echo "ok   CONTROL an acyclic pair still passes"; ok=$((ok+1));
else echo "FAIL CONTROL an acyclic pair still passes (rc=$rc, out=$out)"; fail=$((fail+1)); fi

echo "----"
echo "$ok ok, $fail FAIL"
[ "$fail" = 0 ]
