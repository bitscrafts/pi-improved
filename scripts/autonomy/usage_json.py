#!/usr/bin/env python3
"""The account's own usage reading: Claude Code's OAuth usage endpoint (the one its /usage command calls).

Why a file of its own: the shell parser that read the endpoint's human-readable line mistook the "5" of
"5-hour" for the percentage twice (2026-10-13), and a syntax slip in that shell arm took the whole window guard
down. One small reader, one job, its output is JSON.
"""
import datetime, json, os, sys, urllib.request

URL = "https://api.anthropic.com/api/oauth/usage"
HDR = {"anthropic-beta": "oauth-2025-04-20"}


def token() -> str:
    creds = json.load(open(os.path.expanduser("~/.claude/.credentials.json")))
    return creds.get("claudeAiOauth", {}).get("accessToken") or creds.get("accessToken")


def read() -> dict:
    req = urllib.request.Request(URL, headers={**HDR, "Authorization": "Bearer " + token()})
    d = json.load(urllib.request.urlopen(req, timeout=25))
    fh = d.get("five_hour") or {}
    limits = d.get("limits") or [{}]
    return {
        "pct": fh.get("utilization"),
        "reset": fh.get("resets_at"),
        "severity": limits[0].get("severity"),
        "weekly_pct": (d.get("seven_day") or {}).get("utilization"),
    }


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[1] == "--epoch":
        s = sys.argv[2].strip().replace("Z", "+00:00")
        print(int(datetime.datetime.fromisoformat(s).timestamp()))
    else:
        try:
            print(json.dumps(read()))
        except Exception as e:  # a guard must never crash: say nothing, let the caller treat it as unknown
            print("", file=sys.stderr)
            sys.exit(3)
