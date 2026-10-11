#!/usr/bin/env bash
# The loop's environment, set here and not inherited.
#
# Measured 2026-10-11: a cycle started by the cron tick answered
#   UNDECIDED rc=127: timeout: failed to run command 'claude': No such file or directory
# because a scheduled job does not inherit the interactive PATH (claude lives in /root/.local/bin, cargo in
# /usr/local/cargo/bin). This is the same reason HARD RULE 0d exists: a dispatch that worked only because the
# calling shell happened to have the right PATH is not reproducible. Every script in this directory sources
# this file before it does anything.
# APPENDED, not prepended: a caller that injects its own PATH (the selftest's fake CLI, a scrubber, a
# wrapper) must keep priority, while a minimal scheduled environment still finds the tools it needs.
export PATH="${PATH:-}:/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin"
case ":$PATH:" in *":/root/.local/bin:"*) ;; *) export PATH="$PATH:/root/.local/bin" ;; esac
export PII_TMP_ROOT="${PII_TMP_ROOT:-/projects/_tmp}"

# root + bypassPermissions also needs the sandbox marker, or the worker dies in one second with
# "--dangerously-skip-permissions cannot be used with root/sudo privileges".
export IS_SANDBOX="${IS_SANDBOX:-1}"
