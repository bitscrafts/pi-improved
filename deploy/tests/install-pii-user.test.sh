#!/usr/bin/env bash
# Spec 338: the installer that ships refuses without a deploy card, and never overwrites a store.
# Drives deploy/install-pii-user.sh against fixtures. A fake ssh/scp run the "remote" side locally with HOME set to a
# fixture home, so nothing reaches the host. The control runs the installer as committed in b6e4f4e (the defect).
#
#   bash deploy/tests/install-pii-user.test.sh     one line per case, then RESULT: <ok> ok, <fail> FAIL
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INSTALLER="$REPO/deploy/install-pii-user.sh"
CONTROL_REV=b6e4f4e   # the commit whose installer copied dev stores over production with no card check
REV7="$(git -C "$REPO" rev-parse --short=7 HEAD)"
REV12="$(git -C "$REPO" rev-parse --short=12 HEAD)"

SCRATCH=""
cleanup() {
  # Removes only the scratch this test created (the guarded pattern below), never anything else.
  case "$SCRATCH" in
    "$REPO/_tmp/install-pii-user-test."*) find "$SCRATCH" -mindepth 1 -delete; rmdir "$SCRATCH" ;;
  esac
}
trap cleanup EXIT
mkdir -p "$REPO/_tmp"
SCRATCH="$(mktemp -d "$REPO/_tmp/install-pii-user-test.XXXXXX")"

# Test shim for ssh: runs the command locally with HOME = the fixture home. Host-side process and service commands are
# shadowed by no-ops, so only the file operations reach the fixture.
write_shims() {
  cat > "$1/bin/ssh" <<'SHIM'
#!/usr/bin/env bash
while [ "$1" = -o ]; do shift 2; done
shift
PRE='kill(){ :; }; pkill(){ :; }; sudo(){ :; }; podman(){ :; }; sleep(){ :; }; systemctl(){ echo active; }'
exec env HOME="$FAKE_HOME" bash -c "$PRE; $*"
SHIM
  cat > "$1/bin/scp" <<'SHIM'
#!/usr/bin/env bash
while [ "$1" = -q ]; do shift; done
src="$1"; dst="$2"
case "$dst" in
  *:*) rel="${dst#*:}"; dst="$FAKE_HOME/${rel#/}" ;;
esac
mkdir -p "$(dirname "$dst")" && cp "$src" "$dst"
SHIM
  chmod +x "$1/bin/ssh" "$1/bin/scp"
}

# One case directory: an empty fixture home, a card directory, and the local "container" sources.
fixture() {
  local c="$1"
  mkdir -p "$c/home" "$c/cards" "$c/tmp" "$c/bin" "$c/src/root/.local/bin" "$c/src/root/.config/pii" \
    "$c/src/root/.local/state/pii/graphs"
  printf '#!/bin/sh\necho "pii 0.0.0-fixture health ok"\n' > "$c/src/root/.local/bin/pii"
  chmod +x "$c/src/root/.local/bin/pii"
  printf '[core]\nruntime_dir = "/run/user/0/pii"\nlog_file = "/root/.local/state/pii/pii.log"\ntrajectory_dir = "/root/.local/state/pii/trajectory"\n' \
    > "$c/src/root/.config/pii/pii.toml"
  printf 'PII_DISCORD_TOKEN=fixture\nPII_DISCORD_CHANNEL_ID=1\n' > "$c/src/root/.config/pii/pii.env"
  printf 'DEV-STORE\n' > "$c/src/root/.local/state/pii/store.redb"
  printf 'dev graph\n' > "$c/src/root/.local/state/pii/graphs/g.toml"
  write_shims "$c"
}

plant_card() {  # the card the gate accepts: keyed by the 7-char revision, a non-empty JSON record
  printf '{"change":{"revision":"%s"}}\n' "$REV12" > "$1/cards/card-$REV7.json"
}

plant_store() {  # a production store already on the target
  mkdir -p "$1/home/.local/state/pii"
  printf 'PROD-STORE\n' > "$1/home/.local/state/pii/store.redb"
}

# Runs an installer in a case dir with that case's fixture environment; output in $1/last.out, rc returned.
run() {
  local c="$1" script="$2"; shift 2
  PATH="$c/bin:$PATH" FAKE_HOME="$c/home" TMPDIR="$c/tmp" DEPLOY_CARD_DIR="$c/cards" \
    BIN="$c/src/root/.local/bin/pii" SRC_CFG="$c/src/root/.config/pii/pii.toml" \
    SRC_ENV="$c/src/root/.config/pii/pii.env" SRC_STATE="$c/src/root/.local/state/pii" \
    bash "$script" "$@" > "$c/last.out" 2>&1
}

sum_of() { sha256sum "$1" | cut -d' ' -f1; }
tree_sum() { (cd "$1" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum) | sha256sum | cut -d' ' -f1; }

ok=0
fail=0
check() {  # $1 name, $2 rc of the case (0 = pass)
  if [ "$2" = 0 ]; then ok=$((ok + 1)); echo "PASS $1"; else fail=$((fail + 1)); echo "FAIL $1"; fi
}

case_no_card() {
  local c="$SCRATCH/no-card"; fixture "$c"
  local before after rc
  before="$(tree_sum "$c/home")"
  run "$c" "$INSTALLER" --apply; rc=$?
  after="$(tree_sum "$c/home")"
  [ "$rc" = 2 ] && grep -q REFUSED "$c/last.out" && [ "$before" = "$after" ]
}

case_card_existing_store() {
  local c="$SCRATCH/existing-store"; fixture "$c"; plant_card "$c"; plant_store "$c"
  local before rc=0
  before="$(sum_of "$c/home/.local/state/pii/store.redb")"
  # An UPGRADE: the binary, the config and the unit are installed, and no store is copied (nothing is passed
  # to copy one), so the live store must come out byte-identical.
  run "$c" "$INSTALLER" --apply; [ $? = 0 ] || rc=1
  grep -q "UPGRADE" "$c/last.out" || rc=1
  grep -q "kept:.*store.redb" "$c/last.out" || rc=1
  [ "$(sum_of "$c/home/.local/state/pii/store.redb")" = "$before" ] || rc=1
  # CONTROL: --seed-stores over a live store is still refused, because that is the operation that destroys it.
  run "$c" "$INSTALLER" --seed-stores --apply; [ $? = 2 ] || rc=1
  grep -q -- "--force-seed" "$c/last.out" || rc=1
  [ "$(sum_of "$c/home/.local/state/pii/store.redb")" = "$before" ] || rc=1
  return $rc
}

case_card_no_store_apply() {
  local c="$SCRATCH/apply"; fixture "$c"; plant_card "$c"
  local rc=0
  run "$c" "$INSTALLER" --apply; [ $? = 0 ] || rc=1
  [ -x "$c/home/.local/bin/pii" ] || rc=1
  # the remote home comes from the host, and the config carries it instead of /root
  grep -q "$c/home/.local/state/pii" "$c/home/.local/pii/pii.toml" || rc=1
  grep -q '/root/' "$c/home/.local/pii/pii.toml" && rc=1
  # no store is copied unless --seed-stores is passed
  [ ! -e "$c/home/.local/state/pii/store.redb" ] || rc=1
  return $rc
}

case_control_overwrites() {
  local c="$SCRATCH/control"; fixture "$c"; plant_store "$c"
  local before after
  before="$(sum_of "$c/home/.local/state/pii/store.redb")"
  git -C "$REPO" show "$CONTROL_REV:deploy/install-pii-user.sh" \
    | sed -e "s#/root/#$c/src/root/#g" -e "s#/tmp/#$c/tmp/#g" > "$c/old.sh"
  run "$c" "$c/old.sh"
  after="$(sum_of "$c/home/.local/state/pii/store.redb")"
  # the defect: the committed installer replaces the production store with the dev one
  [ "$before" != "$after" ]
}

case_dry_run() {
  local c="$SCRATCH/dry-run"; fixture "$c"; plant_card "$c"
  local before after rc=0
  before="$(tree_sum "$c/home")"
  run "$c" "$INSTALLER"; [ $? = 0 ] || rc=1
  after="$(tree_sum "$c/home")"
  [ "$before" = "$after" ] || rc=1
  grep -q "DRY RUN" "$c/last.out" || rc=1
  grep -q "$REV12" "$c/last.out" || rc=1
  grep -q "$c/home/.local/state/pii" "$c/last.out" || rc=1
  return $rc
}

case_no_card; check "no card, no install" $?
case_card_existing_store; check "a card, an existing store" $?
case_card_no_store_apply; check "a card, no store, --apply" $?
case_control_overwrites; check "control: the old installer overwrites" $?
case_dry_run; check "dry run writes nothing" $?

echo "RESULT: $ok ok, $fail FAIL"
[ "$fail" = 0 ]
