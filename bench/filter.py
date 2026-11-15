#!/usr/bin/env python3
"""Time `--filter` of lvim-fuzzy against fzf (and any other finder that has a
filter mode) on a generated list of paths.

    bench/filter.py [LINES] [PROGRAM...]

The corpus is written to $TMPDIR; each run is the median of 5.
"""
import os, random, statistics, subprocess, sys, tempfile, time

lines = int(sys.argv[1]) if len(sys.argv) > 1 else 1_000_000
here = os.path.dirname(os.path.abspath(__file__))
progs = sys.argv[2:] or ["fzf", os.path.join(here, "..", "target", "release", "lvim-fuzzy")]

words = ["src", "lua", "lvim", "tui", "view", "core", "utils", "config", "test", "docs",
         "plugin", "init", "main", "model", "render", "cache", "theme", "picker", "fuzzy", "engine"]
exts = [".rs", ".lua", ".md", ".toml", ".txt", ".json"]
corpus = os.path.join(tempfile.gettempdir(), f"lvim-fuzzy-bench-{lines}.txt")
if not os.path.exists(corpus):
    random.seed(7)
    with open(corpus, "w") as f:
        for i in range(lines):
            path = "/".join(random.choice(words) for _ in range(random.randint(2, 6)))
            f.write(f"{path}/{random.choice(words)}_{i}{random.choice(exts)}\n")

env = {k: v for k, v in os.environ.items() if not k.startswith(("FZF_", "LVIM_FUZZY_"))}

def run(prog, args):
    times = []
    for _ in range(5):
        with open(corpus, "rb") as f:
            t = time.perf_counter()
            r = subprocess.run([prog, *args], stdin=f, stdout=subprocess.PIPE, env=env)
            times.append(time.perf_counter() - t)
    return statistics.median(times) * 1000, r.stdout.count(b"\n")

names = [os.path.basename(p) for p in progs]
print(f"{lines} lines\n{'query':30}" + "".join(f"{n:>16}" for n in names))
for q in ["lvim", "tuiview", "src lua !md", "'theme ^src .rs$", "pickerfuzzyengine", "zzzz"]:
    for extra in ([], ["--no-sort"]):
        res = [run(p, ["-f", q, *extra]) for p in progs]
        counts = {c for _, c in res}
        label = q + (" (no-sort)" if extra else "")
        note = "" if len(counts) == 1 else "  counts differ: " + ", ".join(str(c) for _, c in res)
        print(f"{label:30}" + "".join(f"{ms:13.0f} ms" for ms, _ in res) + note)
