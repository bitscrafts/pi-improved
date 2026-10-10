#!/usr/bin/env python3
"""Spec 116 R1: the F2 leak bisect harness. Re-runnable, fresh store per variant.

For each variant it starts `target/debug/pii daemon` on a scratch runtime dir,
drives N runs over ONE persistent connection (or one connection per run), and
samples the daemon's VmRSS (/proc) and `sys.status` (rss_kb, store bytes, run
count) every --every runs. It prints one table per variant and the
least-squares slope in KB per 1000 runs. Nothing is written outside --scratch;
the scratch dir of a previous run is moved aside, never deleted.

  scripts/leak_bisect.py --runs 20000 --every 2000 --variants base,obs-off
  long series in foreground chunks: add --keep, then repeat with --resume
  (the last chunk without --keep stops the daemons)
  variants: base obs-off audit-off both-off traj-on conn-per-run idle-conn
"""
import argparse, json, os, socket, subprocess, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PII = os.path.join(ROOT, "target/debug/pii")
GRAPH = """name = "starter/lean"
entry = "step"
caps = []
[limits]
max_steps = 100
max_wall_ms = 30000
[[node]]
id = "step"
kind = "transform"
value = "ok"
"""
AGENT = """name = "starter/loadagent"
entry = "agent"
caps = ["fs_read", "fs_write", "proc"]
[limits]
max_steps = 100
max_wall_ms = 30000
[[node]]
id = "agent"
kind = "agent"
caps = ["fs_read", "fs_write", "proc"]
tools = []
max_turns = 1
[[node]]
id = "reply"
kind = "transform"
expr = "$.payload"
[[edge]]
from = "agent"
port = "ok"
to = "reply"
kind = "data"
"""
# variant -> (extra daemon flags, extra env, driver mode)
VARIANTS = {
    "base": ([], {}, "persist"),
    "obs-off": (["--observer", "off"], {}, "persist"),
    "audit-off": (["--audit", "off"], {}, "persist"),
    "both-off": (["--observer", "off", "--audit", "off"], {}, "persist"),
    "traj-on": ([], {"PII_TRAJECTORY_DIR": "{scratch}/traj"}, "persist"),
    "conn-per-run": ([], {}, "per-run"),
    "idle-conn": ([], {}, "idle"),
}


class Conn:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(path)
        self.f = self.s.makefile("rwb", buffering=0)
        self.id = 0
        self.call("sys.hello", {"protocol": "pii.v2", "client": {"name": "leak116", "version": "0", "build": "py"}, "encodings": ["json"], "features": []})

    def call(self, method, params):
        self.id += 1
        self.f.write((json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n").encode())
        while True:
            msg = json.loads(self.f.readline())
            if msg.get("id") == self.id:
                break
        if msg.get("error"):
            raise RuntimeError(msg["error"])
        return msg["result"]

    def close(self):
        self.s.close()


def rss_kb(pid):
    """(VmRSS, RssAnon, RssFile) in KiB: anonymous heap vs file-backed mappings."""
    got = {}
    for line in open(f"/proc/{pid}/status"):
        k = line.split(":")[0]
        if k in ("VmRSS", "RssAnon", "RssFile"):
            got[k] = int(line.split()[1])
    return got["VmRSS"], got["RssAnon"], got["RssFile"]


def io_bytes(pid):
    d = dict(l.split(": ") for l in open(f"/proc/{pid}/io").read().splitlines())
    return int(d["write_bytes"]), int(d["wchar"])


def cpu_s(pid):
    """Daemon CPU seconds (user+system) from /proc/<pid>/stat."""
    f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")


def one_run(c, graph):
    rid = c.call("run.start", {"graph": graph, "input": {"prompt": "hi"}})["run_id"]
    while True:
        st = c.call("run.get", {"run_id": rid}).get("status")
        if st in ("succeeded", "failed"):
            return st == "succeeded"
        time.sleep(0.0005)


def slope(xs, ys):
    n = len(xs)
    mx, my = sum(xs) / n, sum(ys) / n
    den = sum((x - mx) ** 2 for x in xs)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den if den else 0.0


def start_daemon(name, a, scratch):
    flags, env, _ = VARIANTS[name]
    rt = os.path.join(scratch, "rt")
    os.makedirs(os.path.join(rt, "graphs/starter"))
    open(os.path.join(rt, "graphs/starter/lean.toml"), "w").write(GRAPH)
    open(os.path.join(rt, "graphs/starter/loadagent.toml"), "w").write(AGENT)
    open(os.path.join(scratch, "empty.toml"), "w").close()
    model = os.path.join(scratch, "model.json")
    json.dump({"turns": [{"text": "ok"}] * (100 if a.graph.endswith("lean") else a.total_runs * 2 + 10)}, open(model, "w"))
    e = dict(os.environ, PII_RUNTIME_DIR=rt, PII_MODEL="scripted:" + model)
    e.update({k: v.format(scratch=scratch) for k, v in env.items()})
    d = subprocess.Popen([PII, "daemon", "--sandbox", "off", "--store", os.path.join(scratch, "store.redb"),
                          "--config", os.path.join(scratch, "empty.toml")] + flags,
                         env=e, stdout=subprocess.DEVNULL, stderr=open(os.path.join(scratch, "daemon.err"), "w"))
    sock = os.path.join(rt, "control.sock")
    for _ in range(300):
        if os.path.exists(sock):
            break
        time.sleep(0.1)
    return d, sock


def run_variant(name, a):
    """Runs `a.runs` more runs against the variant's daemon. With --keep the
    daemon (and its sampled rows) survive for the next invocation, so a long
    series can be taken in foreground chunks."""
    mode = VARIANTS[name][2]
    scratch = os.path.join(a.scratch, name)
    state_path = os.path.join(scratch, "state.json")
    st = None
    if a.resume and os.path.exists(state_path):
        st = json.load(open(state_path))
        if not os.path.exists(f"/proc/{st['pid']}"):
            raise SystemExit(f"{name}: recorded daemon {st['pid']} is gone; start a fresh series")
        pid, sock = st["pid"], os.path.join(scratch, "rt/control.sock")
    else:
        if os.path.exists(scratch):
            os.rename(scratch, f"{scratch}.old-{int(time.time())}")
        os.makedirs(scratch)
        d, sock = start_daemon(name, a, scratch)
        pid = d.pid
    rows = st["rows"] if st else []
    done = rows[-1][0] if rows else 0
    c = Conn(sock)
    w0 = tuple(st["w0"]) if st else (0, 0)
    if not st:
        time.sleep(1)
        w0 = io_bytes(pid)

    def sample():
        sd = c.call("sys.status", {})
        main = next(s for s in sd["stores"] if s["name"] == "main")
        wb, wc = io_bytes(pid)
        rss, anon, file = rss_kb(pid)
        rows.append([done, rss, anon, file, main["bytes"], wb - w0[0], wc - w0[1], cpu_s(pid), time.time()])
        json.dump({"pid": pid, "rows": rows, "w0": list(w0)}, open(state_path, "w"))

    if not rows:
        sample()
    target = done + a.runs
    while done < target:
        n = min(a.every, target - done)
        if mode == "idle":
            for _ in range(n):
                t = Conn(sock); t.call("sys.status", {}); t.close()
        elif mode == "per-run":
            for _ in range(n):
                t = Conn(sock); one_run(t, a.graph); t.close()
        else:
            for _ in range(n):
                one_run(c, a.graph)
        done += n
        sample()
    c.close()
    json.dump({"pid": pid, "rows": rows, "w0": list(w0)}, open(state_path, "w"))
    if not a.keep:
        os.kill(pid, 15)
        for _ in range(300):
            if not os.path.exists(f"/proc/{pid}"):
                break
            time.sleep(0.1)
    return rows


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--runs", type=int, default=10000)
    p.add_argument("--every", type=int, default=1000)
    p.add_argument("--variants", default="base")
    p.add_argument("--parallel", action="store_true", help="one daemon per variant at once (shared disk: timing is not comparable, RSS and bytes are)")
    p.add_argument("--keep", action="store_true", help="leave the daemon running for a later --resume")
    p.add_argument("--resume", action="store_true", help="continue the daemon/series left by --keep")
    p.add_argument("--total-runs", type=int, default=0, help="size the scripted model for a graph that uses it")
    p.add_argument("--graph", default="starter/lean")
    p.add_argument("--scratch", default="/projects/_tmp/leak-bisect")
    a = p.parse_args()
    os.makedirs(a.scratch, exist_ok=True)
    a.total_runs = a.total_runs or a.runs
    names = a.variants.split(",")
    if a.parallel:
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(len(names)) as ex:
            results = list(ex.map(lambda n: run_variant(n, a), names))
    else:
        results = [run_variant(n, a) for n in names]
    for name, rows in zip(names, results):
        print(f"== {name}  graph={a.graph}  (\"runs\" = {'connections' if name == 'idle-conn' else 'runs'})")
        print("runs   rss_kb  anon_kb file_kb store_bytes  write_bytes  wchar")
        for r in rows:
            print("%-6d %-7d %-7d %-7d %-12d %-12d %d" % tuple(r[:7]))
        # slope over the second half: past warm-up
        h = rows[len(rows) // 2:] if len(rows) > 3 else rows
        s = slope([r[0] for r in h], [r[1] for r in h]) * 1000
        per = (rows[-1][5] / rows[-1][0]) if rows[-1][0] else 0
        sa = slope([r[0] for r in h], [r[2] for r in h]) * 1000
        print(f"slope_kb_per_1000_runs(second half): rss={s:.1f} anon={sa:.1f}  write_bytes_per_run={per:.0f}  wchar_per_run={rows[-1][6] / max(rows[-1][0], 1):.0f}")
        dr = rows[-1][0] - rows[0][0]
        if dr and len(rows[0]) > 8:
            print(f"cpu_ms_per_run={(rows[-1][7] - rows[0][7]) * 1000 / dr:.2f}  runs_per_s={dr / (rows[-1][8] - rows[0][8]):.1f}  (timing is valid only without --parallel)")


if __name__ == "__main__":
    main()
