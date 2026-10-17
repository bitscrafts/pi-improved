#!/usr/bin/env bash
# verify-179-180.sh — redeploy, put the cron back, and read the counters TWICE, a minute apart.
#
# 179 claims a timer does not re-fire; 180 claims a post survives the day's byte budget.
# Both are measured here IN PRODUCTION, as a DELTA rather than a snapshot: the last single read looked
# healthy ("cron 1, runs climbing") while the graph was firing three times a second.
#
# Run:  bash scripts/verify-179-180.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
export XDG_RUNTIME_DIR=/run/user/0

echo "=== 1. commit, so the release build runs on a clean tree ==="
git add _docs specs scripts crates graphs deploy 2>/dev/null
git commit -q -m "179 and 180 and the day's documents, committed so the release build runs on a clean tree" 2>/dev/null || true
echo "HEAD: $(git log --oneline -1)"

echo "=== 2. install ==="
bash deploy/install-pii.sh 2>&1 | grep -E 'INSTALLED|MOVED ASIDE|Finished|CHANNEL' || true

echo "=== 3. put the cron back — THIS is the test of 179 and 180 ==="
python3 - <<'PY'
import pathlib
env = pathlib.Path('/root/.config/pii/pii.env')
cid = next((l.split('=', 1)[1].strip() for l in env.read_text().splitlines()
            if l.startswith('PII_DISCORD_CHANNEL_ID=')), '') if env.exists() else ''
p = pathlib.Path('/root/.config/pii/pii.toml')
t = p.read_text()
if not [l for l in t.splitlines() if l.strip() == '[[cron]]']:
    p.write_text(t.rstrip() + '\n\n[[cron]]\nname = "digest"\ngraph = "discord-digest"\n'
                 'schedule = "0 9 * * *"\ninput = { to = "' + cid + '" }\n')
    print('cron back (forum id read from pii.env, not printed)')
else:
    print('cron already present')
PY

echo "=== 4. restart from the host ==="
ssh -o BatchMode=yes mvcorrea@10.10.10.217 \
  'systemctl --user restart pii && sleep 10 && systemctl --user is-active pii; echo -n "restarts: "; systemctl --user show pii -p NRestarts --value' \
  2>&1 | tail -2

echo "=== 5. two reads, sixty seconds apart ==="
sleep 25
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health --json 2>/dev/null > /tmp/t1.json
sleep 60
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health --json 2>/dev/null > /tmp/t2.json

python3 - <<'PY'
import json
try:
    a = json.load(open('/tmp/t1.json')); b = json.load(open('/tmp/t2.json'))
except Exception as e:
    print('could not read health:', e); raise SystemExit(1)
ca, cb = a.get('counts', {}) or {}, b.get('counts', {}) or {}
d = (cb.get('runs') or 0) - (ca.get('runs') or 0)
print('revision    :', b.get('build', {}).get('revision'))
print('T1          : runs', ca.get('runs'), '| posts', ca.get('channel_posts'), '| cron', ca.get('cron_entries'))
print('T2 (+60s)   : runs', cb.get('runs'), '| posts', cb.get('channel_posts'), '| cron', cb.get('cron_entries'))
print('179 VERDICT :', ('HELD (+%d runs in 60s)' % d) if d <= 2 else ('STILL FIRING (+%d runs in 60s)' % d))
print('180 VERDICT :',
      ('POSTED (posts %s -> %s)' % (ca.get('channel_posts'), cb.get('channel_posts')))
      if (cb.get('channel_posts') or 0) > (ca.get('channel_posts') or 0)
      else 'no new post yet — the digest fires at 09:00 UTC')
PY

echo "=== newest egress denial (should be OLD or absent) ==="
grep -a 'egress.denied' /root/.local/state/pii/pii.log 2>/dev/null | tail -1 | cut -c1-170
