#!/usr/bin/env python3
"""discord-forum-thread.py — create (or find) a topic thread in a Discord FORUM, and report its id.

Grouping by topic means one long-lived thread per tag: `status` carries status lines, `alert` carries incidents,
`chat` carries conversation. A forum post IS a thread, so the way to group is to post into the same thread each
time instead of creating a new one.

Usage: discord-forum-thread.py --forum <id> --name "pii · status" --tag status [--guild <id>] [--env <path>]
Prints `THREAD <id>` on success, or `EXISTS <id>` when a thread with that exact name is already there.
The token is read from the env file in-process: never printed, never an argument.
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
            if line.strip().startswith("PII_DISCORD_TOKEN="):
                return line.strip().split("=", 1)[1].strip().strip('"').strip("'")
    raise SystemExit(f"no PII_DISCORD_TOKEN in {path}")


def call(method, url, token, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bot {token}")
    req.add_header("User-Agent", "DiscordBot (https://github.com/pii, 0.1)")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            raw = resp.read().decode() or "{}"
            return resp.status, (json.loads(raw) if raw.strip().startswith(("{", "[")) else {})
    except urllib.error.HTTPError as e:
        return e.code, {"error": e.read().decode()[:300]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--forum", required=True)
    ap.add_argument("--name", required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--message", default="")
    ap.add_argument("--guild", default="1484208204627054704")
    ap.add_argument("--env", default="/home/mvcorrea/.local/pii/pii.env")
    args = ap.parse_args()

    token = load_token(args.env)

    # already there? (active threads of the guild, filtered to this forum)
    status, body = call("GET", f"{API}/guilds/{args.guild}/threads/active", token)
    existing = {}
    if status == 200 and isinstance(body, dict):
        for t in body.get("threads", []) or []:
            if isinstance(t, dict) and t.get("parent_id") == args.forum:
                existing[t.get("name", "")] = t.get("id")
    for t in (body.get("threads") if isinstance(body, dict) else None) or []:
        if isinstance(t, dict) and t.get("parent_id") == args.forum:
            print(f"  active: {t.get('name','')[:40]} -> {t.get('id')}  tags={t.get('applied_tags')}")
    if args.name in existing:
        print(f"EXISTS {existing[args.name]}")
        return 0

    # the tag id, by name
    status, chan = call("GET", f"{API}/channels/{args.forum}", token)
    if status != 200 or not isinstance(chan, dict):
        print(f"could not read the forum: HTTP {status} {chan.get('error','')[:120]}")
        return 1
    tag_id = next((t.get("id") for t in chan.get("available_tags") or [] if t.get("name") == args.tag), None)
    if not tag_id:
        names = [t.get("name") for t in chan.get("available_tags") or []]
        print(f"REFUSED: the forum has no tag named {args.tag!r}; it has {names}")
        return 2

    payload = {"name": args.name, "applied_tags": [tag_id], "message": {"content": args.message or f"thread for {args.tag}"}}
    status, created = call("POST", f"{API}/channels/{args.forum}/threads", token, payload)
    if status in (200, 201) and isinstance(created, dict) and created.get("id"):
        print(f"THREAD {created['id']}  tag={args.tag}")
        return 0
    print(f"REFUSED: HTTP {status} {str(created.get('error',''))[:200]}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
