#!/usr/bin/env python3
"""Latency benchmark for the t9 CLI.

usage: bench.py [--t9 PATH] [--runs N]
Builds two synthetic vaults (small: 50 items / 500 entries, large: 2,000 items / 50,000 entries) in a temp dir and
reports, per command, the median time inside the process (T9_TRACE) and the median wall-clock time including
process spawn. Build first: cargo build --release.
"""
import argparse, os, re, shutil, statistics, subprocess, sys, tempfile, time

ap = argparse.ArgumentParser()
ap.add_argument("--t9", default=os.path.join(os.path.dirname(__file__), "..", "target", "release", "t9"))
ap.add_argument("--runs", type=int, default=15)
a = ap.parse_args()
t9 = os.path.abspath(a.t9)
here = os.path.dirname(os.path.abspath(__file__))

def ms(s):
    m = re.search(r"total ([\d.]+)(µs|ms|s)\b", s)
    v = float(m.group(1)); return v / 1000 if m.group(2) == "µs" else v * 1000 if m.group(2) == "s" else v

def bench(env, cwd, args):
    inproc, wall = [], []
    for _ in range(a.runs):
        t = time.perf_counter()
        r = subprocess.run([t9, *args], cwd=cwd, env=env, capture_output=True, text=True)
        wall.append((time.perf_counter() - t) * 1000)
        if r.returncode != 0:
            sys.exit(f"t9 {args} failed: {r.stderr}")
        inproc.append(ms(r.stderr))
    return statistics.median(inproc), statistics.median(wall)

def spawn_baseline():
    t = time.perf_counter()
    for _ in range(a.runs):
        subprocess.run(["/bin/echo"], capture_output=True)
    return (time.perf_counter() - t) * 1000 / a.runs

cases = [
    ("t9 log \"…\"", ["log", "benchmark entry"]),
    ("t9 add task \"…\"", ["add", "task", "benchmark task"]),
    ("t9 status ID active", ["status", "BIG-3", "active"]),
    ("t9 ls", ["ls"]),
    ("t9 show ID", ["show", "BIG-3"]),
    ("t9 context", ["context"]),
    ("t9 search (2 words)", ["search", "invoice", "refund"]),
    ("t9 whereami", ["whereami"]),
]
print(f"process spawn baseline (/bin/echo): {spawn_baseline():.1f} ms\n")
for label, items, days, per_day in [("small", 50, 10, 50), ("large", 2000, 50, 1000)]:
    tmp = tempfile.mkdtemp(prefix="t9bench-")
    root = os.path.realpath(os.path.join(tmp, "work")); os.makedirs(root)
    vault = os.path.join(tmp, "vault")
    subprocess.run([sys.executable, os.path.join(here, "gen_vault.py"), vault, root, "--items", str(items),
                    "--days", str(days), "--per-day", str(per_day)], check=True, capture_output=True)
    env = {**os.environ, "T9_HOME": os.path.join(tmp, "home"), "T9_VAULT": vault, "T9_AGENT": "bench", "T9_TRACE": "1"}
    env.pop("T9_SESSION", None); env.pop("T9_PROJECT", None)
    t = time.perf_counter()
    subprocess.run([t9, "reindex"], cwd=root, env=env, check=True, capture_output=True)
    rss = subprocess.run(["/usr/bin/time", "-l", t9, "context"], cwd=root, env=env, capture_output=True, text=True).stderr
    rss_mb = int(re.search(r"(\d+)\s+maximum resident set size", rss).group(1)) / 1048576
    print(f"## {label}: {items} items, {days * per_day} entries — reindex {time.perf_counter() - t:.2f} s, "
          f"peak RSS (context) {rss_mb:.1f} MB")
    print(f"{'command':<24} {'in-process':>11} {'wall':>9}")
    for name, args in cases:
        i, w = bench(env, root, args)
        print(f"{name:<24} {i:>8.1f} ms {w:>6.1f} ms")
    print()
    shutil.rmtree(tmp)
