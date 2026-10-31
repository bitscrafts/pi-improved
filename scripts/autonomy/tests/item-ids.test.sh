#!/usr/bin/env bash
# item-ids.test.sh — an item's id is its work's spec number, and a legacy id must declare itself (spec 328).
# Measured 2026-10-31: the queue's A-numbers and the flow's A-numbers were two counters that agreed only
# because a person typed matching names (item A184 -> spec 320, item A186 -> spec 322, and A186 had no queue
# row at all).
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0
FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T=_tmp/item-ids.$$
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
mkdir -p "$T/.minilam/items" "$T/specs"
printf '%s\n' '# scratch' '## Exit Criteria' '- [ ] `true`' > "$T/specs/900-scratch.md"
item(){ printf '%s' "$2" > "$T/.minilam/items/$1.json"; }
run(){ MINILAM_DIR="$T/.minilam" PII_ITEM_TRASH="$T/trash" bash scripts/autonomy/item-ids.sh "$@"; }
rc(){ run "$@" >/dev/null 2>&1; echo $?; }

# (a) a legacy A-id that does not declare itself is a violation
item A5 '{"id":"A5","state":"done","data":{"spec":"specs/900-scratch.md"},"history":[{"a":1}],"stack":null,"flow":"flow"}'
check "(a) --check is red on a legacy id" "$(rc --check)" "1"
check "(a) and names the number it should be" "$(run --check 2>&1 | grep -c 'should be 900')" "1"

# (b) --migrate renames it, records the alias, keeps the history, and MOVES the old file out of the store
printf '%s' '{"current":"A5","items":{},"schema":1}' > "$T/.minilam/state.json"
check "(b) --migrate exits 0" "$(rc --migrate)" "0"
check "(b) the new file exists" "$(test -f "$T/.minilam/items/900.json" && echo yes)" "yes"
check "(b) the old file left the store" "$(test -f "$T/.minilam/items/A5.json" && echo present || echo gone)" "gone"
check "(b) it was MOVED, not deleted" "$(test -f "$T/trash/A5.json" && echo yes)" "yes"
check "(b) the history came across" "$(python3 -c "import json;print(len(json.load(open('$T/.minilam/items/900.json')).get('history') or []))")" "1"
check "(b) the alias is declared" "$(python3 -c "import json;print((json.load(open('$T/.minilam/items/900.json')).get('data') or {}).get('alias'))")" "A5"
check "(b) the id field follows the name" "$(python3 -c "import json;print(json.load(open('$T/.minilam/items/900.json')).get('id'))")" "900"
check "(b) the pointer was repointed" "$(python3 -c "import json;print(json.load(open('$T/.minilam/state.json')).get('current'))")" "900"

# (c) the store is in step afterwards, and a declared alias stays legal
check "(c) --check is green after the migration" "$(rc --check)" "0"

# (d) an item whose name and id disagree is a violation — it has two identities
item 901 '{"id":"A7","state":"done","data":{"spec":"specs/900-scratch.md"}}'
check "(d) a name/id disagreement is red" "$(rc --check)" "1"
rm -f "$T/.minilam/items/901.json"

# (e) an item with no data.spec cannot be tied to its work
item 902 '{"id":"902","state":"done","data":{}}'
check "(e) a missing spec is red" "$(rc --check)" "1"
rm -f "$T/.minilam/items/902.json"

# (f) a migration that would collide is refused, and nothing moves
item A9 '{"id":"A9","state":"done","data":{"spec":"specs/900-scratch.md"}}'
item 900 '{"id":"900","state":"done","data":{"spec":"specs/900-scratch.md"},"history":[]}'
check "(f) a collision is red" "$(rc --migrate)" "1"
check "(f) and the legacy file is still in place" "$(test -f "$T/.minilam/items/A9.json" && echo present)" "present"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
