#!/usr/bin/env bash
# Rehearse scripts/deploy-upgrade-host.sh on the host in a SCRATCH ROOT: no systemd, no live daemon, no live store, no credential.
# Two runs: (A) a successful upgrade; (B) a forced bad-health upgrade that must roll back and restore every backed-up file
# byte-for-byte. Output is the evidence the deploy card quotes. Read-only on the live install (it only copies an OLD binary,
# the config and the graphs, all non-secret; the stores are fake files).
set -uo pipefail
cd /projects/pi-improved || exit 1
REV=${1:?usage: rehearse-deploy-upgrade.sh <rev>}
LOG=_tmp/deploy/rehearsal-${REV:0:7}.log
mkdir -p _tmp/deploy; : > "$LOG"
exec > >(tee -a "$LOG") 2>&1
HOST=mvcorrea@10.10.10.217
sshx(){ timeout 120 ssh -o BatchMode=yes "$HOST" "$@"; }
setup(){ # $1 = scratch root on the host
  sshx "set -e; R='$1'; mkdir -p \$R/.local/bin \$R/.local/pii \$R/.local/state/pii/graphs
    cp -p ~/.local/bin/pii.old-1793594912 \$R/.local/bin/pii
    cp -p ~/.local/pii/pii.toml \$R/.local/pii/pii.toml
    cp -a ~/.local/state/pii/graphs/. \$R/.local/state/pii/graphs/
    mkdir -p \$R/run/pii/graphs && cp -a /run/user/1000/pii/graphs/. \$R/run/pii/graphs/
    for f in store memory approval heal queue; do head -c 4096 /dev/urandom > \$R/.local/state/pii/\$f.redb; done
    cd \$R && find .local run -type f ! -name '*.prev-*' -print0 | sort -z | xargs -0 sha256sum > \$R/BEFORE.sha256; wc -l < \$R/BEFORE.sha256"
}
TS=$(date +%s)
echo "######## REHEARSAL of deploy-upgrade-host.sh for $REV at $(date -u +%FT%TZ)"
echo "######## A. successful upgrade in a scratch root"
RA=/home/mvcorrea/pii-rehearsal-A-$TS
echo "files in scratch: $(setup "$RA")"
REMOTE_ROOT=$RA NO_UNIT=1 bash scripts/deploy-upgrade-host.sh "$REV" --apply 2>&1 | tail -22
echo "-- after A: installed binary label / models block / graphs:"
sshx "R='$RA'; strings -n 12 \$R/.local/bin/pii | grep -oE '${REV:0:7}[0-9a-f]{5}' | sort -u | head -2; grep -c '^\[models\.' \$R/.local/pii/pii.toml; ls \$R/.local/state/pii/graphs/*.toml | wc -l; ls \$R/.local/state/pii/graphs/.by-sha256 2>/dev/null | wc -l"
echo
echo "######## B. forced bad health: must roll back to the BEFORE state byte-for-byte"
RB=/home/mvcorrea/pii-rehearsal-B-$TS
echo "files in scratch: $(setup "$RB")"
FORCE_BAD_HEALTH=1 REMOTE_ROOT=$RB NO_UNIT=1 bash scripts/deploy-upgrade-host.sh "$REV" --apply 2>&1 | tail -16
echo "-- after B: every file that existed before must hash the same (the .by-sha256 dir and the backup are additions, not changes):"
sshx "cd '$RB' && sha256sum -c BEFORE.sha256 2>&1 | grep -vc ': OK$'; echo '(number above = files that DIFFER from before; must be 0)'; sha256sum -c BEFORE.sha256 2>&1 | grep -c ': OK$'"
echo
echo "######## C. control: the same check must be able to FAIL (alter one file in B's scratch, re-check)"
sshx "cd '$RB' && echo tamper >> .local/pii/pii.toml && sha256sum -c BEFORE.sha256 2>&1 | grep -c 'FAILED'; echo '(number above must be >= 1)'"
echo
echo "######## scratch roots left for inspection (never deleted by this script): $RA $RB"
echo "######## END $(date -u +%FT%TZ)"
