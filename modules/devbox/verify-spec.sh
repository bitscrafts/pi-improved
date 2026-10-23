#!/usr/bin/env bash
# Independent verification of a worker's result against the spec's WHOLE Exit Criteria list (the worker's node ran only the first,
# named one), plus a mutation control: revert the production change and the new test must go RED. Inside the devbox, in a copy of the
# tree's changes only is NOT possible (shared mount), so the mutation is applied to the working tree and restored byte-identically
# (sha256 before/after), with no worker active.
# usage: verify-246-247.sh 247
set -uo pipefail
SPEC=${1:?247}
case "$SPEC" in
  247) F=crates/pii-graph/src/validate.rs; T=crates/pii-graph/tests/coder_requires_timeout_lint.rs; PKG=pii-graph; TST=coder_requires_timeout_lint ;;
  *) echo unknown; exit 2 ;;
esac
cd /projects/pi-improved || exit 2
if ps -eo args | grep -q '[c]laude -p'; then echo "worker active: not mutating the tree"; exit 3; fi
ssh -o BatchMode=yes mvcorrea@10.10.10.217 "F=$F TST=$TST PKG=$PKG SPEC=$SPEC bash -s" <<'REMOTE'
X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }
echo "== criterion 1: the spec's tests"
X sh -c "cargo test -q -p $PKG --test $TST 2>&1 | grep -E '^test result|FAILED'"
echo "== criterion 2: lint of the four shipped graphs prints no V10"
X sh -c 'for g in claude-probe coder-selffix coder-demo coder-podman; do echo "$g: $(cargo run -q -p pii-cli --bin pii -- graph lint graphs/$g.toml 2>&1 | grep -c V10) V10 lines"; done'
echo "== criterion 3: every shipped coder graph carries timeout_ms"
X sh -c 'grep -c timeout_ms graphs/coder-demo.toml graphs/coder-podman.toml graphs/coder-selffix.toml graphs/claude-probe.toml'
echo "== criterion 4: the whole pii-graph test suite (no test was edited)"
X sh -c "cargo test -q -p pii-graph 2>&1 | grep -E '^test result' | awk '{p+=\$4; f+=\$6} END{print \"passed=\" p \" failed=\" f}'"
echo "== criterion 5: clippy -D warnings, pii-graph"
X sh -c "cargo clippy -q -p pii-graph --all-targets -- -D warnings 2>&1 | head -5; echo clippy rc=\$?"
echo "== criterion 6: file-size lint"
X sh -c "cargo run -q -p xtask -- lint-files 2>&1 | tail -3; echo lint-files rc=\$?"
echo "== MUTATION CONTROL: take the production change out; the new tests must go red"
X sh -c "sha256sum $F > /tmp/before.sha; cp $F /tmp/validate.orig; git show HEAD:$F > $F; cargo test -q -p $PKG --test $TST 2>&1 | grep -E '^test result|^test .*FAILED' | head -6; cp /tmp/validate.orig $F; sha256sum -c /tmp/before.sha"
REMOTE
