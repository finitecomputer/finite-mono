"""Add the reviewed Tinfoil job without activating the separate Sites rollout.

This deliberately accepts only the repository's narrow YAML layout. Changes to
any active job or global setting fail closed; only full-line comments and blank
lines are ignored for comparison. Live bytes are retained in the candidate.
"""

import argparse
from pathlib import Path
import re


TINFOIL = b"finite-tinfoil-collector"
SITES = b"finite-sites-metrics"
JOB = re.compile(rb"^  - job_name: ([a-zA-Z0-9_.-]+)\n", re.MULTILINE)


def split_jobs(config):
    matches = list(JOB.finditer(config))
    if not matches:
        raise ValueError("unexpected scrape layout")
    jobs = {}
    for index, match in enumerate(matches):
        name = match[1]
        if name in jobs:
            raise ValueError("duplicate scrape job")
        end = matches[index + 1].start() if index + 1 < len(matches) else len(config)
        jobs[name] = config[match.start() : end]
    return config[: matches[0].start()], jobs


def significant(config):
    return [
        line
        for line in config.splitlines()
        if line.strip() and not line.lstrip().startswith(b"#")
    ]


def reconcile(source, live):
    header, jobs = split_jobs(source)
    live_header, live_jobs = split_jobs(live)
    if list(jobs)[-1] != TINFOIL or SITES not in jobs:
        raise ValueError("unexpected source scrape layout")
    # Sites has a separate credential and service rollout. Preserve its current
    # activation state, while requiring every other source job in source order.
    expected = [name for name in jobs if name != TINFOIL]
    if SITES not in live_jobs:
        expected.remove(SITES)
    actual = [name for name in live_jobs if name != TINFOIL]
    if actual != expected or significant(header) != significant(live_header):
        raise ValueError("unrelated Prometheus drift; reconcile before bootstrap")
    for name, body in live_jobs.items():
        if name not in jobs or significant(body) != significant(jobs[name]):
            raise ValueError("unrelated Prometheus drift; reconcile before bootstrap")
    if TINFOIL in live_jobs:
        return live
    return live + (b"\n" if live.endswith(b"\n") else b"\n\n") + jobs[TINFOIL]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("live", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.write_bytes(reconcile(args.source.read_bytes(), args.live.read_bytes()))


if __name__ == "__main__":
    main()
