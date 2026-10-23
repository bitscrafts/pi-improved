#!/usr/bin/env bash
# modules/devbox/check.sh — does the devbox module say what it claims? Exit 0 only when every row passes. NO build, NO container,
# NO network, NO key read: it checks the files this module ships, so it can run anywhere (the orchestrator has no podman).
# Each row prints PASS/FAIL and its reason. A control row proves the check can see a violation.
set -uo pipefail
D=$(cd "$(dirname "$0")" && pwd)
rc=0
row() { # name rc detail
  if [ "$2" -eq 0 ]; then printf 'PASS  %s\n' "$1"; else printf 'FAIL  %s  %s\n' "$1" "${3:-}"; rc=1; fi
}
C=$D/Containerfile; Y=$D/compose.yaml
[ -s "$C" ] && [ -s "$Y" ]; row "Containerfile and compose.yaml exist and are non-empty" $?

# the base is the official slim rust image
grep -qE '^FROM docker\.io/library/rust:[0-9]+\.[0-9]+\.[0-9]+-slim-' "$C"; row "base is the official rust slim image, version pinned" $?
# the toolchain the repo pins equals the image's
want=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$D/../../rust-toolchain.toml")
have=$(sed -n 's/^FROM docker.io\/library\/rust:\([0-9.]*\)-slim.*/\1/p' "$C")
[ -n "$want" ] && [ "$want" = "$have" ]; row "image rust ($have) equals rust-toolchain.toml ($want)" $? "the gate would download a different toolchain at run time"
grep -q "RUSTUP_TOOLCHAIN=$want" "$C"; row "RUSTUP_TOOLCHAIN is pinned to the same version" $?

# no secret value and no key NAME with a value anywhere in the module
! grep -nE '(API_KEY|TOKEN|SECRET|PASSWORD)[A-Z_]*=[^$ "]+' "$C" "$Y" >/dev/null; row "no credential value in Containerfile or compose.yaml" $?
# control: the same pattern MUST catch a planted value, or the row above proves nothing
printf 'ENV ANTHROPIC_API_KEY=sk-ant-plant\n' > "${TMPDIR:-/tmp}/devbox-check-plant.$$"
grep -qE '(API_KEY|TOKEN|SECRET|PASSWORD)[A-Z_]*=[^$ "]+' "${TMPDIR:-/tmp}/devbox-check-plant.$$"; row "control: the secret pattern catches a planted value" $?
rm -f "${TMPDIR:-/tmp}/devbox-check-plant.$$" 2>/dev/null

grep -q '^ *env_file:' "$Y"; row "credentials come from an env_file, not inline" $?
# Root inside + the default mapping: files written to the project keep the host owner. keep-id + root would give them to a sub-uid (measured).
no_keep_id() { ! grep -qE '^[[:space:]]*(userns_mode:.*keep-id|.*--userns[= ]keep-id)' "$1"; }
no_keep_id "$Y"; row "no keep-id with a root container (it would give the tree to a sub-uid)" $?
# control: the same predicate MUST reject a compose file that asks for keep-id, or the row above proves nothing
printf 'services:\n  x:\n    userns_mode: "keep-id"\n' > "${TMPDIR:-/tmp}/devbox-check-keepid.$$"
no_keep_id "${TMPDIR:-/tmp}/devbox-check-keepid.$$"; [ $? -ne 0 ]; row "control: the keep-id predicate rejects a planted keep-id" $?
rm -f "${TMPDIR:-/tmp}/devbox-check-keepid.$$" 2>/dev/null
grep -q 'io.pii: "1"' "$Y"; row "the container carries pii's io.pii label" $?
grep -q 'IS_SANDBOX' "$C"; row "IS_SANDBOX set so claude accepts bypass-permissions as root" $?
! grep -qE '^USER ' "$C"; row "root inside (no USER directive), by decision" $?
# minimality: nothing the evidence ruled out
for t in cmake openssh tmux rsync nodejs npm; do
  ! grep -E '^\s*(RUN|apt-get).*\b'"$t"'\b' "$C" >/dev/null; row "not installed: $t (no consumer measured)" $?
done
# the agents are copied from the build context, not downloaded at build time
grep -q '^COPY .*pi /usr/local/bin/pi' "$C" && grep -q '^COPY .*claude /usr/local/bin/claude' "$C"; row "pi and claude come from the build context" $?
! grep -qE 'curl .*\| *(ba)?sh|npm install' "$C"; row "no curl|sh and no npm install in the image build" $?
# Python is uv's only (operator, 2026-10-23): uv is in the image, no apt python3, and the smoke step asserts no bare python3.
grep -qE '^COPY --from=ghcr\.io/astral-sh/uv:[0-9]' "$C"; row "uv comes from its own pinned image" $?
grep -q '^RUN uv python install' "$C"; row "the interpreter is uv-managed" $?
no_apt_python() { ! grep -E 'apt-get install' "$1" | grep -qE '\bpython3?\b'; }
no_apt_python "$C"; row "no apt python in the image" $?
printf 'RUN apt-get install -y python3 git\n' > "${TMPDIR:-/tmp}/devbox-check-py.$$"
no_apt_python "${TMPDIR:-/tmp}/devbox-check-py.$$"; [ $? -ne 0 ]; row "control: the apt-python predicate rejects a planted apt python3" $?
rm -f "${TMPDIR:-/tmp}/devbox-check-py.$$" 2>/dev/null
grep -q 'command -v python3' "$C"; row "the smoke step asserts a bare python3 is absent" $?
# criteria run under a scrubbed env: rustup must find its toolchain from HOME alone
grep -q 'ln -s /usr/local/cargo /root/.cargo' "$C" && grep -q 'ln -s /usr/local/rustup /root/.rustup' "$C"; row "cargo and rustup resolve from HOME alone (scrubbed criterion env)" $?
grep -q 'devbox-target:/projects/pi-improved/target' "$Y"; row "the build volume is also at the project's default target dir" $?
exit $rc
