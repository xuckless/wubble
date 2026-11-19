#!/usr/bin/env python3
"""Median and 95th percentile (nearest rank) per browser, variant, page and temperature, from the
CSVs of scripts/measure-load.sh and scripts/measure-load-chromium.sh. Hand-run only.

    scripts/summarize-load.py a.csv b.csv ...
"""
import csv
import math
import sys
from collections import defaultdict

METRICS = ["fcp", "dcl", "load", "s0", "host_first_frame", "build"]


def rank(values, p):
    values = sorted(values)
    return values[max(math.ceil(p * len(values) / 100), 1) - 1]


groups = defaultdict(lambda: defaultdict(list))
for path in sys.argv[1:]:
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            key = (row["page"], row["temp"], row["browser"] + "/" + row["variant"])
            for metric in METRICS:
                if row.get(metric):
                    groups[key][metric].append(float(row[metric]))
            groups[key]["n"].append(1)
            if row.get("served"):
                groups[key]["served"].append(float(row["served"]))

print(f"{'page':<11} {'temp':<5} {'browser/variant':<20} {'n':>3} " + " ".join(f"{m + ' p50/p95':>22}" for m in METRICS))
for key in sorted(groups):
    g = groups[key]
    cells = []
    for metric in METRICS:
        v = g.get(metric)
        cells.append(f"{rank(v, 50):>10.1f}/{rank(v, 95):<10.1f}" if v else f"{'-':>22}")
    print(f"{key[0]:<11} {key[1]:<5} {key[2]:<20} {len(g['n']):>3} " + " ".join(cells))
