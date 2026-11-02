# Spec 329: the Python 3.11+ (tomllib) that reads and verifies pii.toml. Sourced by install-pii.sh and
# install-config-keeps.test.sh; `config_python <repo>` prints an interpreter path and returns 1 when none can be had.
# Order: PII_PYTHON, then a python3.11+ on PATH, then uv provisioning one into <repo>/_tmp/ (scratch, no root needed).
config_python() {
  local repo="$1" c
  if [ -n "${PII_PYTHON:-}" ]; then echo "$PII_PYTHON"; return 0; fi
  for c in python3 python3.13 python3.12 python3.11; do
    if command -v "$c" >/dev/null 2>&1 && "$c" -I -c 'import tomllib' >/dev/null 2>&1; then
      command -v "$c"; return 0
    fi
  done
  if command -v uv >/dev/null 2>&1; then
    export UV_PYTHON_INSTALL_DIR="$repo/_tmp/uv-python" UV_CACHE_DIR="$repo/_tmp/uv-cache"
    if uv python install 3.12 >/dev/null 2>&1 && uv python find 3.12 2>/dev/null; then return 0; fi
  fi
  return 1
}

# Spec 197 / 338: the deploy-card check, shared by install-pii.sh and install-pii-user.sh. It lives in this tracked,
# already-sourced helper, not in a new file, so a committed-tree export carries it.
#   deploy_card_check DIR REV7 REV12
# Returns 0 when DIR/card-REV7.json or DIR/card-REV12.json exists and is non-empty, and sets DEPLOY_CARD to that file.
# Returns 1 and prints nothing: each installer words its own refusal.
deploy_card_check() {
  DEPLOY_CARD=""
  if [ -s "$1/card-$2.json" ]; then DEPLOY_CARD="$1/card-$2.json"; return 0; fi
  if [ -s "$1/card-$3.json" ]; then DEPLOY_CARD="$1/card-$3.json"; return 0; fi
  return 1
}
