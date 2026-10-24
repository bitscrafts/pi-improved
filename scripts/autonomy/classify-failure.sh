#!/usr/bin/env bash
# classify-failure.sh — what kind of failure a worker's log shows (spec 254): auth | quota | work.
# stdin: the worker log. stdout: exactly one word. Pure text matching: no network, no credential read.
# Auth wins over quota: a dead login is the lane's fault, and it must not be read as a window that reopens.
LOG=$(cat)
if printf '%s\n' "$LOG" | grep -qF -e 'OAuth token revoked' -e 'Failed to authenticate' -e 'Please log in again' -e 'api_error_status":401' -e 'api_error_status\":401' -e 'claude_api_error_status":"401"'; then
  echo auth
elif printf '%s\n' "$LOG" | grep -qiE 'spend limit|usage limit|hit your.*limit'; then
  echo quota
else
  echo work
fi
exit 0
