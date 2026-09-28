#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Produce only the exact bounded flat capability artifact stream."""

import argparse
import json
import sys
import os
import time

from capability_io import Meter, produce
from capability_protocol import COLLECT_WAIT_RESERVE


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print(json.dumps({"status": "NOT_RUN", "vendor_passes": 0, "application_launches": 0}))
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--produce", action="store_true")
    parser.add_argument("--wait-seconds", type=int)
    args = parser.parse_args(argv)
    if args.wait_seconds is not None:
        if args.produce or not 30 <= args.wait_seconds <= 600 + COLLECT_WAIT_RESERVE:
            return 1
        from capability_io import exclusive
        from capability_session import encoded, process_identity
        exclusive("/runtime/collector-admission.json", encoded({"protocol": "original-capability-collector/1",
            "pid": os.getpid(), "birth": process_identity(os.getpid()), "uid_gid": [os.getuid(), os.getgid()],
            "seconds": args.wait_seconds}), Meter())
        until = time.monotonic() + args.wait_seconds
        while time.monotonic() < until:
            if os.path.exists("/output/ready"):
                return 0
            time.sleep(min(0.1, until - time.monotonic()))
        return 1
    if not args.produce:
        print(json.dumps({"status": "NOT_RUN", "vendor_passes": 0, "application_launches": 0}))
        return 0
    try:
        produce("/output", sys.stdout.buffer, Meter())
        return 0
    except (Exception, KeyboardInterrupt):
        print("capability-artifact-stream-incomplete", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
