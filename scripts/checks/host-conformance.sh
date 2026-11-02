#!/usr/bin/env bash
# The probe's entry point: the checks live in host-conformance.py (json is easier to read there than in bash).
exec python3 "$(dirname "$0")/host-conformance.py" "$@"
