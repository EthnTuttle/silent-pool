#!/usr/bin/env python3
"""Qualified stage-latency analysis for a real spc-lab runtime run.

Clocks:
- node completion: `logs/node-*.log` `tip: best=<hash>` is written after the
  tip commit. The display hash is byte-reversed to the pool's internal
  `prev_hash` order. The header `ntime` and the RPC client timestamps are not
  used. The node api-log `submitblock` params are truncated by the node logger,
  so they cannot reconstruct the block hash; its `ts`/`wall_ms` are also not a
  client-latency measurement.
- pool issue: `logs/front-*.log` `job_issued.issued_at` is epoch milliseconds.
- miner activation: `logs/miner-*.log` `occurred_at_wall_ms` is epoch
  milliseconds. `occurred_at_mono_ms` is miner-relative and is not compared.
- collector ingestion times are NOT used.

The tip -> issue stage joins a submitblock completion to the first
`job_issued` with the new block hash as `prev_hash`. The issue -> activation
stage joins the exact job id within the same miner process log. The first
`--skip-first` submitblocks are excluded by default because the startup job
predates a meaningful chain transition; the raw count is reported separately.

Source hashes are recorded for every input file so a table can be tied to the
exact artifacts it was derived from.
"""

import argparse
import datetime as dt
import glob
import hashlib
import json
import re
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def iso_ms(value):
    return int(dt.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def pct(values, q):
    if not values:
        return None
    values = sorted(values)
    return values[min(len(values) - 1, int(q * (len(values) - 1)))]


def summary(values):
    if not values:
        return {"n": 0}
    return {
        "n": len(values),
        "p50_ms": pct(values, 0.50),
        "p90_ms": pct(values, 0.90),
        "p95_ms": pct(values, 0.95),
        "max_ms": max(values),
        "mean_ms": round(sum(values) / len(values), 2),
    }


def parse_tips(run_dir):
    rows = []
    for path in sorted(glob.glob(str(Path(run_dir) / "logs" / "node-*.log"))):
        for line in Path(path).read_text(errors="replace").splitlines():
            match = re.match(r"^(\S+Z) INFO tip: best=([0-9a-f]+) height=(\d+)", line)
            if not match:
                continue
            rows.append({
                "completed_ms": iso_ms(match.group(1)),
                "block_hash": bytes.fromhex(match.group(2))[::-1].hex(),
                "height": int(match.group(3)),
            })
    return rows

def parse_jobs(run_dir):
    jobs = []
    for path in sorted(glob.glob(str(Path(run_dir) / "logs" / "front-front-*.log"))):
        for line in Path(path).read_text(errors="replace").splitlines():
            clean = ANSI.sub("", line)
            match = re.search(r"evidence=(\{.*\})", clean)
            if not match or '"job_id"' not in match.group(1) or "activation" not in match.group(1):
                continue
            try:
                raw = json.loads(match.group(1))
            except json.JSONDecodeError:
                continue
            if "issued_at" not in raw or "prev_hash" not in raw:
                continue
            jobs.append({"front": raw.get("front_id"), "job_id": raw.get("job_id"),
                         "generation": int(raw.get("generation", 0)),
                         "issued_at": int(raw["issued_at"]), "prev_hash": raw["prev_hash"]})
    return jobs


def parse_activations(run_dir):
    acts = []
    for path in sorted(glob.glob(str(Path(run_dir) / "logs" / "miner-*.log"))):
        for line in Path(path).read_text(errors="replace").splitlines():
            try:
                envelope = json.loads(line)
            except json.JSONDecodeError:
                continue
            event = envelope.get("event", {})
            if event.get("name") != "job_activated":
                continue
            payload = event.get("payload", {})
            job_id = payload.get("job", {}).get("job_id")
            if not isinstance(job_id, int):
                continue
            acts.append({"miner": envelope.get("miner_id"), "job_id": job_id,
                         "generation": payload.get("job", {}).get("target_generation"),
                         "prev_hash": payload.get("job", {}).get("prev_hash_le"),
                         "at_ms": envelope.get("occurred_at_wall_ms")})
    return acts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dir")
    parser.add_argument("--skip-first", type=int, default=2)
    parser.add_argument("--json", type=Path, help="write the full result JSON here")
    args = parser.parse_args()

    run_dir = Path(args.run_dir)
    tips = parse_tips(run_dir)
    jobs = parse_jobs(run_dir)
    acts = parse_activations(run_dir)

    tip_to_issue = []
    for index, submit in enumerate(tips):
        candidates = [job for job in jobs
                      if job["prev_hash"] == submit["block_hash"]
                      and job["issued_at"] >= submit["completed_ms"]]
        if not candidates:
            continue
        first = min(candidates, key=lambda job: job["issued_at"])
        tip_to_issue.append({
            "index": index,
            "latency_ms": first["issued_at"] - submit["completed_ms"],
            "skipped": index < args.skip_first,
        })

    # Join activation to job by (prev_hash, job_id). A plain job id collides
    # across fronts, so an ambiguous key is skipped rather than paired with the
    # wrong front. Multi-front issue->activation is therefore not claimed when
    # keys collide; the unambiguous tip->issue stage is always reported.
    jobs_by_key = {}
    ambiguous = set()
    for job in jobs:
        try:
            numeric = int(str(job["job_id"]), 16)
        except ValueError:
            continue
        key = (job["prev_hash"], numeric)
        if key in jobs_by_key:
            ambiguous.add(key)
        jobs_by_key[key] = job
    issue_to_activation = []
    unmatched_activations = 0
    for act in acts:
        key = (act["prev_hash"], act["job_id"])
        if key in ambiguous:
            unmatched_activations += 1
            continue
        job = jobs_by_key.get(key)
        if job is None or act["at_ms"] < job["issued_at"]:
            unmatched_activations += 1
            continue
        issue_to_activation.append(act["at_ms"] - job["issued_at"])

    raw_latencies = [row["latency_ms"] for row in tip_to_issue]
    qualified = [row["latency_ms"] for row in tip_to_issue if not row["skipped"]]
    result = {
        "run_dir": str(run_dir),
        "clocks": {
            "node_completion": "node tip log line written after the tip commit; best hash reversed to internal order",
            "pool_issue": "job_issued.issued_at epoch ms (wall)",
            "miner_activation": "occurred_at_wall_ms epoch ms (wall); occurred_at_mono_ms is not used",
            "collector_ingested_at": "not used",
        },
        "startup_policy": {
            "skip_first_tips": args.skip_first,
            "reason": "the first jobs may predate a meaningful chain transition",
        },
        "counts": {
            "node_tip_observations": len(tips),
            "job_issued_records": len(jobs),
            "joined_tip_to_issue": len(tip_to_issue),
            "activation_joins": len(issue_to_activation),
            "activation_unmatched_or_ambiguous": unmatched_activations,
            "tip_to_issue_excluded_as_startup": len(tip_to_issue) - len(qualified),
        },
        "tip_to_first_issue": {
            "raw_all_joins": summary(raw_latencies),
            "qualified": summary(qualified),
        },
        "issue_to_activation": summary(issue_to_activation),
        "sources": {
            "node_logs": {path: sha256(path) for path in sorted(glob.glob(
                str(run_dir / "logs" / "node-*.log")))},
            "pool_logs": {path: sha256(path) for path in sorted(glob.glob(
                str(run_dir / "logs" / "front-front-*.log")))},
            "miner_logs": {path: sha256(path) for path in sorted(glob.glob(
                str(run_dir / "logs" / "miner-*.log")))},
        },
    }
    text = json.dumps(result, indent=2) + "\n"
    if args.json:
        args.json.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
