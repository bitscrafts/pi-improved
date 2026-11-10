#!/usr/bin/env python3
"""Load-stability harness (spec 119): the gate's own test command under a stated CPU load.

Runs `cargo test --workspace --lib --bins --tests --no-fail-fast` (the quality-gate.sh test step)
for N rounds while HOGS busy-loop processes hold the CPUs, then prints per-test failure counts
across rounds. Tests living in a source file that uses sleep/timeout/Duration are "timing
sensitive" and are listed even at 0 of N.

  --env tree     run in the dev tree (warm target). Known NOT to reproduce the gate's failures.
  --env export   `git archive HEAD` + uncommitted overlay into a fresh dir under /projects/_tmp,
                 own cold CARGO_TARGET_DIR, built once; every round runs in that export.
                 This is the gate's condition (fresh-gate export). The export is kept
                 (path printed) and never deleted by this script.
  --compile-load also run `cargo check --workspace --tests` (own target dir under --keep-logs) beside each round.

usage: scripts/archive/flake_hunt.py [--env tree|export] [--rounds N] [--hogs N] [--cpus N] [--compile-load] [--direct]
                             [--filter SUBSTR] [--keep-logs DIR] [--export-dir DIR]
"""
import argparse, os, re, subprocess, sys, tempfile, time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = Path("/projects/_tmp")
CMD = ["cargo", "test", "--workspace", "--lib", "--bins", "--tests", "--no-fail-fast"]
TIMING = re.compile(r"sleep|timeout|Duration::from_|Instant::now")
RUNNING = re.compile(r"^\s+Running (?:.*\((\S+)\)|(\S+)$)")
RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)")


def export_tree() -> Path:
    BASE.mkdir(parents=True, exist_ok=True)
    dest = Path(tempfile.mkdtemp(prefix="flake-hunt-", dir=BASE))
    subprocess.run(
        ["bash", "-c", 'set -o pipefail; git -C "$1" archive HEAD | tar -x -C "$2"', "_", str(ROOT), str(dest)],
        check=True)
    files = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z", "-m", "-o", "--exclude-standard"],
        check=True, capture_output=True).stdout.split(b"\0")
    for rel in filter(None, files):
        src = ROOT / rel.decode()
        if src.is_file():
            (dest / rel.decode()).parent.mkdir(parents=True, exist_ok=True)
            (dest / rel.decode()).write_bytes(src.read_bytes())
    return dest


def timing_sources(cwd: Path) -> set:
    """Binary names of tests that live in a file using sleep/timeout/Duration: the file stem for
    `tests/*.rs`, the crate name (`-` -> `_`) for unit tests under `src/`."""
    out = set()
    for p in (cwd / "crates").rglob("*.rs"):
        rel = p.relative_to(cwd / "crates").parts
        if {"target", "_tmp"} & set(rel):
            continue
        try:
            if TIMING.search(p.read_text(errors="ignore")):
                out.add(p.stem if rel[1] == "tests" else rel[0].replace("-", "_"))
        except OSError:
            pass
    return out


def test_binaries(cwd: Path, env) -> list:
    """[(executable, crate dir)] of every test binary, from cargo's own JSON artifact stream."""
    import json
    out = subprocess.run(CMD + ["--no-run", "--message-format=json"], cwd=cwd, env=env,
                         capture_output=True, text=True, check=True).stdout
    exes = []
    for line in out.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("executable") and m["profile"]["test"]:
            d = Path(m["target"]["src_path"]).parent
            while not (d / "Cargo.toml").exists():
                d = d.parent
            exes.append((m["executable"], str(d)))
    return exes


def run_direct(pin, exes, flt, env):
    log, rc = [], 0
    for exe, d in exes:
        r = subprocess.run(pin + [exe] + ([flt] if flt else []), cwd=d, env=env, capture_output=True, text=True)
        log.append(f"     Running {exe}\n{r.stdout}{r.stderr}")
        rc = rc or r.returncode
    return subprocess.CompletedProcess([], rc), "\n".join(log)


def parse(log: str):
    """-> {(binary, test): status}"""
    res, binary = {}, "?"
    for line in log.splitlines():
        m = RUNNING.match(line)
        if m:
            binary = Path(m.group(1) or m.group(2)).name
            binary = re.sub(r"-[0-9a-f]{16}$", "", binary)
            continue
        m = RESULT.match(line)
        if m:
            res[(binary, m.group(1))] = m.group(2)
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--env", choices=["tree", "export"], default="tree")
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--hogs", type=int, default=24)
    ap.add_argument("--compile-load", action="store_true")
    ap.add_argument("--cpus", type=int, default=0, help="pin the test run AND the hogs to the first N cores (0 = all)")
    ap.add_argument("--direct", action="store_true",
                    help="run the pre-built test binaries one after another (as cargo does) instead of via "
                         "`cargo test`; for hunting a filtered test when cargo's own startup dominates a starved box")
    ap.add_argument("--filter", default="")
    ap.add_argument("--keep-logs", default="")
    ap.add_argument("--export-dir", default="", help="reuse an export a previous run printed (skips re-export)")
    a = ap.parse_args()

    env = dict(os.environ, PATH="/usr/local/cargo/bin:" + os.environ["PATH"])
    if a.env == "export":
        cwd = Path(a.export_dir) if a.export_dir else export_tree()
        env["CARGO_TARGET_DIR"] = str(cwd / "target")
        print(f"export: {cwd} (kept)", flush=True)
        # compile outside the loaded window; a reused export is already built (cargo re-checks it anyway)
        subprocess.run(["cargo", "build", "--workspace"], cwd=cwd, env=env, check=True, stderr=subprocess.DEVNULL)
        subprocess.run(CMD + ["--no-run"], cwd=cwd, env=env, check=True, stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
    else:
        cwd = ROOT
    exes = test_binaries(cwd, env) if a.direct else []
    pin = ["taskset", "-c", f"0-{a.cpus - 1}"] if a.cpus else []
    cmd = CMD + ([a.filter] if a.filter else [])
    watch = timing_sources(cwd)
    logdir = Path(a.keep_logs) if a.keep_logs else Path(tempfile.mkdtemp(prefix="flake-logs-", dir=BASE))
    logdir.mkdir(parents=True, exist_ok=True)
    hogs = [subprocess.Popen(pin + ["bash", "-c", "while :; do :; done"]) for _ in range(a.hogs)]
    fails, seen, rcs = defaultdict(int), {}, []
    try:
        for i in range(1, a.rounds + 1):
            cl = None
            if a.compile_load:
                cl = subprocess.Popen(["cargo", "check", "--workspace", "--tests"], cwd=cwd,
                                      env=dict(env, CARGO_TARGET_DIR=str(logdir / "compile-load-target")),
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            t0 = time.time()
            if a.direct:
                p, log = run_direct(pin, exes, a.filter, env)
            else:
                # one interleaved stream: `Running <binary>` headers (stderr) must precede that binary's results
                p = subprocess.run(pin + cmd, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True)
                log = p.stdout
            (logdir / f"round-{i}.log").write_text(log)
            if cl:
                cl.terminate(); cl.wait()
            res = parse(log)
            bad = sorted(k for k, v in res.items() if v == "FAILED")
            for k, v in res.items():
                seen[k] = True
                if v == "FAILED":
                    fails[k] += 1
            rcs.append(p.returncode)
            print(f"round {i}/{a.rounds} rc={p.returncode} tests={len(res)} failed={len(bad)} "
                  f"{time.time() - t0:.0f}s {[t for _, t in bad]}", flush=True)
    finally:
        for h in hogs:
            h.kill(); h.wait()
    print(f"\nENV={a.env} cwd={cwd} LOAD={a.hogs} cpu hogs on {a.cpus or os.cpu_count()} cores"
          f"{' + concurrent cargo build' if a.compile_load else ''} ROUNDS={a.rounds}")
    print(f"command: {' '.join(cmd)}\nround rcs: {rcs}   logs: {logdir}")
    print("\nPER-TEST FAILURES ACROSS ROUNDS (failed tests, then timing-sensitive tests at 0)")
    rows = [(k, fails.get(k, 0)) for k in seen if fails.get(k, 0) or k[0] in watch]
    for (b, t), n in sorted(rows, key=lambda r: (-r[1], r[0])):
        print(f"{n:>3} of {a.rounds}  {b}::{t}")
    print(f"\nTOTAL: {sum(fails.values())} failures over {a.rounds} rounds; {len(seen)} distinct tests seen")
    return 1 if any(rcs) else 0


if __name__ == "__main__":
    sys.exit(main())
