#!/usr/bin/env python3
# Summarises sampler.sh's output by phase and process role as a Markdown table.
#
#   python3 -I analyze.py OUTDIR
import collections, csv, os, re, statistics as st, sys

out = sys.argv[1] if len(sys.argv) > 1 else "data"
roles = {}
for row in csv.reader(open(os.path.join(out, "roles.csv"))):
    if len(row) == 2:
        roles[row[0]] = row[1]


def group(pid):
    r = roles.get(pid, "?")
    if r == "main":
        return "main"
    if r == "gpu-process":
        return "gpu"
    if r == "renderer":
        return "tabs"
    return "helpers"


def mb(s):
    m = re.match(r"([\d.]+)([KMGB]?)", s.rstrip("+-"))
    if not m:
        return 0.0
    v, u = float(m[1]), m[2]
    return v / 1024 if u == "K" else v * 1024 if u == "G" else v / 1048576 if u == "B" else v


snap = collections.defaultdict(lambda: collections.defaultdict(lambda: [0.0, 0.0]))
phase_of = {}
for ts, ph, pid, cpu, mem in csv.reader(open(os.path.join(out, "proc.csv"))):
    g = group(pid)
    snap[ts][g][0] += float(cpu)
    snap[ts][g][1] += mb(mem)
    phase_of[ts] = ph

order = []
for ts in sorted(snap):
    if phase_of[ts] not in order:
        order.append(phase_of[ts])
sysd = collections.defaultdict(list)
for row in csv.reader(open(os.path.join(out, "sys.csv"))):
    if len(row) == 5 and row[2]:
        sysd[row[1]].append((float(row[2]), float(row[4] or 0)))

G = ["main", "gpu", "tabs", "helpers"]
print("| phase | n | " + " | ".join(f"{g} CPU%" for g in G) + " | total CPU% (p95) | "
      + " | ".join(f"{g} MB" for g in G) + " | total MB | GPU util% | free mem% |")
print("|" + "---|" * (4 + 2 * len(G) + 3))
for ph in order:
    tss = [ts for ts in snap if phase_of[ts] == ph]
    row = [ph, str(len(tss))]
    tot = [sum(snap[ts][g][0] for g in G) for ts in tss]
    for g in G:
        row.append(f"{st.mean(snap[ts][g][0] for ts in tss):.0f}")
    tot_s = sorted(tot)
    row.append(f"{st.mean(tot):.0f} ({tot_s[int(0.95 * (len(tot_s) - 1))]:.0f})")
    last = sorted(tss)[-1]
    for g in G:
        row.append(f"{snap[last][g][1]:.0f}")
    row.append(f"{sum(snap[last][g][1] for g in G):.0f}")
    s = sysd.get(ph, [])
    row.append(f"{st.mean(x[0] for x in s):.0f}" if s else "-")
    row.append(f"{s[-1][1]:.0f}" if s else "-")
    print("| " + " | ".join(row) + " |")
