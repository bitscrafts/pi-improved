#!/usr/bin/env bash
# wire-digest-to-topic.sh — point the digest at the `pii · status` TOPIC thread instead of the forum root.
#
# Why: a forum turns every message into a NEW thread, so posting a digest into the forum id created one post per
# day — tagged, but not grouped. A topic thread groups them: the digest appends to `pii · status` and the forum
# stays one entry per topic.
#
# Measured 2026-11-02: `pii · status` = 1566652720214188134 (tag status), `pii · alerts` = 1566652758432424018.
# The edit is done on a LOCAL copy and shipped back, because a nested heredoc inside ssh quoting mangles itself.
set -uo pipefail
HOST=mvcorrea@lnx
NEW_TO=${1:-1566652720214188134}
TMP=$(mktemp -d /tmp/digest-topic-XXXXXX)
trap 'rm -rf "$TMP"' EXIT

scp -q "$HOST:.local/pii/pii.toml" "$TMP/pii.toml" || { echo "could not fetch the config" >&2; exit 2; }
echo "digest target before: $(grep -A6 'graph = "discord-digest"' "$TMP/pii.toml" | grep -o 'to = "[0-9]*"' | head -1)"

python3 - "$TMP/pii.toml" "$NEW_TO" <<'PY'
import re, sys
path, new = sys.argv[1], sys.argv[2]
text = open(path).read()
i = text.index('graph = "discord-digest"')
head, tail = text[:i], text[i:]
tail, n = re.subn(r'to = "[0-9]+"', f'to = "{new}"', tail, count=1)
if n != 1:
    raise SystemExit("REFUSED: no `to = \"<id>\"` found in the digest block — nothing changed")
open(path, 'w').write(head + tail)
print(f"digest target after : {new}")
PY

# keep the previous config on the host, then place the new one with the same permissions
timeout 60 ssh -o BatchMode=yes "$HOST" "cp -p ~/.local/pii/pii.toml ~/.local/pii/pii.toml.topic-\$(date +%s)" || exit 3
scp -q "$TMP/pii.toml" "$HOST:.local/pii/pii.toml.new" && timeout 60 ssh -o BatchMode=yes "$HOST" \
  'install -m 600 ~/.local/pii/pii.toml.new ~/.local/pii/pii.toml && rm -f ~/.local/pii/pii.toml.new && systemctl --user restart pii.service && sleep 3
   export PII_RUNTIME_DIR=/run/user/1000/pii
   echo "unit   : $(systemctl --user is-active pii.service)"
   echo "digest : $(~/.local/bin/pii graph list 2>/dev/null | grep -i digest)"
   echo "health : $(~/.local/bin/pii health 2>&1 | head -1)"'
