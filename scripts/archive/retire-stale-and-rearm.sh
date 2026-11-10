#!/usr/bin/env bash
# retire-stale-and-rearm.sh — cancel the stale duplicate digests, restore the cron, and prove the pair.
#
# Context: four suspended digest runs for ONE cron entry (070 §0.1). 187 stops NEW arming; 189 gives the verb to retire the
# old ones as LOGGED EVENTS. The cron was removed so they could not fire at the next occurrence; this puts it back once
# exactly one armed run remains.
#
# The proof is one equation:   suspended == cron_entries
set -uo pipefail
cd /projects/pi-improved || exit 1
export PII_RUNTIME_DIR=/run/user/0/pii
HOST=mvcorrea@10.10.10.217
PII=/root/.local/bin/pii

armed() { "$PII" run ls --status suspended 2>/dev/null | python3 -c 'import sys,json; d=json.load(sys.stdin); r=d if isinstance(d,list) else d.get("runs",[]); print(len(r))' 2>/dev/null || echo "?"; }
ids()   { "$PII" run ls --status suspended 2>/dev/null | python3 -c 'import sys,json; d=json.load(sys.stdin); r=d if isinstance(d,list) else d.get("runs",[]); [print(x.get("id") or x.get("run_id") or "") for x in r]' 2>/dev/null; }
cronn() { "$PII" health --json 2>/dev/null | python3 -c 'import sys,json; print((json.load(sys.stdin).get("counts") or {}).get("cron_entries"))' 2>/dev/null || echo "?"; }
rev()   { "$PII" health --json 2>/dev/null | python3 -c 'import sys,json; print((json.load(sys.stdin).get("build") or {}).get("revision"))' 2>/dev/null; }

echo "=== BEFORE ==="
echo "  armed: $(armed)   cron_entries: $(cronn)   revision: $(rev)"

echo "=== retire all but one, each a logged event ==="
mapfile -t RUNS < <(ids)
echo "  found ${#RUNS[@]} suspended"
keep="${RUNS[0]:-}"
for r in "${RUNS[@]}"; do
  if [ "$r" = "$keep" ]; then
    echo "  KEEP   $r"
    continue
  fi
  out=$("$PII" run cancel "$r" --reason "stale duplicate of the digest cron, retired after 187 stopped new arming" 2>&1)
  rc=$?
  echo "  CANCEL $r -> rc=$rc  ${out:0:110}"
done

echo "=== restore the cron: 0 12 * * * UTC = 09:00 local ==="
python3 - <<'PY'
import pathlib
env = pathlib.Path('/root/.config/pii/pii.env')
cid = next((l.split('=', 1)[1].strip() for l in env.read_text().splitlines()
            if l.startswith('PII_DISCORD_CHANNEL_ID=')), '') if env.exists() else ''
p = pathlib.Path('/root/.config/pii/pii.toml')
t = p.read_text()
if not [l for l in t.splitlines() if l.strip() == '[[cron]]']:
    p.write_text(t.rstrip() + '\n\n[[cron]]\nname = "digest"\ngraph = "discord-digest"\n'
                 'schedule = "0 12 * * *"\ninput = { to = "' + cid + '" }\n')
    print('  cron restored')
else:
    print('  cron already present')
PY
ssh -o BatchMode=yes "$HOST" 'systemctl --user restart pii && sleep 10 && systemctl --user is-active pii' 2>&1 | tail -1

sleep 6
echo "=== AFTER — the only pair that matters ==="
A=$(armed); C=$(cronn)
echo "  armed: $A   cron_entries: $C   revision: $(rev)"
if [ "$A" = "$C" ]; then
  echo "  VERDICT: BALANCED — suspended == cron_entries == $A"
else
  echo "  VERDICT: UNBALANCED (suspended $A vs cron $C) — do not leave it here"
fi
