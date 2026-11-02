#!/usr/bin/env python3
"""discord-forum-empty.py — list and optionally delete every thread (post) in a Discord FORUM channel.

A forum has no messages: each post is a thread, so emptying it means deleting threads. The bot must hold
Manage Threads; a refusal is reported per thread and the exit code is non-zero, never swallowed.

Usage: discord-forum-empty.py --forum <id> [--guild <id>] [--yes] [--env <path>]
  without --yes it only lists (safe default)
The token is read from the env file inside this process, never printed, never passed as an argument.
"""
import argparse
import json
import sys
import urllib.error
import urllib.request

API = "https://discord.com/api/v10"


def load_token(path):
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line.startswith("PII_DISCORD_TOKEN="):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit(f"no PII_DISCORD_TOKEN in {path}")


def call(method, url, token, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bot {token}")
    # Cloudflare answers 403 / error 1010 to the default Python-urllib agent — curl gets through, and so does a
    # named one. Measured 2026-11-02.
    req.add_header("User-Agent", "DiscordBot (https://github.com/pii, 0.1)")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            raw = resp.read().decode() or "{}"
            return resp.status, (json.loads(raw) if raw.strip().startswith(("{", "[")) else {})
    except urllib.error.HTTPError as e:
        return e.code, {"error": e.read().decode()[:200]}
    except Exception as e:  # noqa: BLE001 - reported, not raised, so one failure cannot stop the sweep
        return 0, {"error": str(e)[:200]}


def threads_in(forum, guild, token):
    found, seen = [], set()
    if guild:
        status, body = call("GET", f"{API}/guilds/{guild}/threads/active", token)
        if status != 200:
            print(f"active threads: HTTP {status} {body.get('error', '')[:120]}")
        for t in (body.get("threads", []) if isinstance(body, dict) else []):
            if t.get("parent_id") == forum and t["id"] not in seen:
                seen.add(t["id"])
                found.append(t)
    before = None
    for _ in range(20):  # archived posts, paginated
        url = f"{API}/channels/{forum}/threads/archived/public?limit=100" + (f"&before={before}" if before else "")
        status, body = call("GET", url, token)
        if status != 200:
            print(f"archived threads: HTTP {status} {body.get('error', '')[:120]}")
            break
        batch = body.get("threads", []) if isinstance(body, dict) else []
        batch = [t for t in batch if isinstance(t, dict) and "id" in t]
        for t in batch:
            if t["id"] not in seen:
                seen.add(t["id"])
                found.append(t)
        if not batch or not body.get("has_more"):
            break
        meta = batch[-1].get("thread_metadata") or {}
        before = meta.get("archive_timestamp") or batch[-1]["id"]
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--forum", required=True)
    ap.add_argument("--guild", default="1484208204627054704")
    ap.add_argument("--yes", action="store_true")
    ap.add_argument("--env", default="/home/mvcorrea/.local/pii/pii.env")
    args = ap.parse_args()

    token = load_token(args.env)
    found = threads_in(args.forum, args.guild, token)
    print(f"{len(found)} post(s) in forum {args.forum}")
    for t in found:
        print(f"  {t['id']}  {t.get('name', '')[:70]}")

    if not args.yes:
        print("nothing deleted (pass --yes to delete)")
        return 0

    failures = 0
    for t in found:
        status, body = call("DELETE", f"{API}/channels/{t['id']}", token)
        if status in (200, 204):
            print(f"deleted {t['id']}  {t.get('name', '')[:60]}")
        else:
            failures += 1
            print(f"REFUSED {t['id']}  HTTP {status} {body.get('error', '')[:100]}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
