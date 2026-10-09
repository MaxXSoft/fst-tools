#!/usr/bin/env python3
"""Compare saved release test executables; write raw results to stdout as JSON.

Build with cargo test -p queryfst --bin queryfst --release --no-run --message-format=json.
Save the executable from compiler-artifact.executable under ignored debug/.
Run: python3 queryfst/benches/sql_compile.py baseline=debug/base candidate=debug/new
Each executable must contain the same sql::bench::sql_compile_microbench test.
"""

import argparse
import json
import os
import random
import statistics
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", nargs="+", help="label=path to saved test executable")
    parser.add_argument("--rounds", type=int, default=9)
    parser.add_argument("--ms", type=int, default=30, help="minimum time per case and phase")
    args = parser.parse_args()
    if args.rounds < 1 or args.ms < 1:
        parser.error("rounds and ms must be positive")
    binaries = dict(item.split("=", 1) for item in args.binaries)
    records = []
    fingerprints = {}
    rng = random.Random(20261009)
    for round_index in range(args.rounds):
        labels = list(binaries)
        rng.shuffle(labels)
        for label in labels:
            output = subprocess.check_output(
                [str(Path(binaries[label]).resolve()), "sql::bench::sql_compile_microbench",
                 "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                env={**os.environ, "SQL_BENCH_MS": str(args.ms), "SQL_BENCH_ROTATION": str(round_index)},
                text=True,
            )
            found = []
            for line in output.splitlines():
                if "BENCH " not in line:
                    continue
                item = json.loads(line.split("BENCH ", 1)[1])
                key = (item["case"], item["phase"])
                previous = fingerprints.setdefault(key, item["fingerprint"])
                if previous != item["fingerprint"]:
                    raise RuntimeError(f"result mismatch: {label} {key}")
                found.append(key)
                records.append({"variant": label, "round": round_index, **item})
            if len(found) != 48 or len(set(found)) != 48:
                raise RuntimeError(f"expected 16 cases x 3 phases, got {len(found)}")
    summary = []
    for label in binaries:
        for case, phase in sorted(fingerprints):
            values = [r["ns"] for r in records if r["variant"] == label and r["case"] == case and r["phase"] == phase]
            median = statistics.median(values)
            summary.append({"variant": label, "case": case, "phase": phase,
                            "median_ns": median,
                            "mad_ns": statistics.median(abs(v - median) for v in values),
                            "min_ns": min(values), "max_ns": max(values)})
    print(json.dumps({"rounds": args.rounds, "ms": args.ms, "records": records, "summary": summary}, indent=2))


if __name__ == "__main__":
    main()
