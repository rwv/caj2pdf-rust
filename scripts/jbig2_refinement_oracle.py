# SPDX-License-Identifier: MIT
"""Report the optional T.88 refinement-pixel evidence state without false parity.

There is no independent per-symbol refinement-pixel oracle in this repository.
An explicitly selected private file is identity-checked, but even a verified
file cannot become a compatibility pass until an independent expected-pixel
comparison is implemented.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re

import jbig2_dictionary_headers


MAX_FIXTURE_BYTES = 4 * 1024 * 1024
SHA256 = re.compile(r"[0-9a-fA-F]{64}\Z")


def initial_report() -> dict:
    return {
        "status": "NOT_RUN",
        "refinement_compatibility": {
            "status": "NOT_RUN",
            "checked_cases": 0,
            "passed": 0,
            "failed": 0,
            "reason": "no independent refinement-pixel oracle is configured",
        },
        "corpus_metadata": {"status": "NOT_RUN", "checked_images": 0},
        "fixture": {"status": "NOT_RUN"},
    }


def verify_fixture(path: Path, expected_sha256: str) -> str:
    """Bound a private fixture read and require its independent SHA-256 pin."""
    if not SHA256.fullmatch(expected_sha256):
        raise ValueError("fixture SHA-256 must be 64 hexadecimal characters")
    if not path.is_file():
        raise ValueError("explicit refinement fixture is missing or not a file")
    length = path.stat().st_size
    if length == 0 or length > MAX_FIXTURE_BYTES:
        raise ValueError("refinement fixture size is outside the 1..4 MiB limit")
    def digest_once() -> tuple[int, str]:
        digest = hashlib.sha256()
        count = 0
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(64 * 1024), b""):
                count += len(chunk)
                if count > MAX_FIXTURE_BYTES:
                    raise ValueError("refinement fixture grew beyond the 4 MiB limit")
                digest.update(chunk)
        return count, digest.hexdigest()

    before = digest_once()
    after = digest_once()
    if before != after or before[0] != length or path.stat().st_size != length:
        raise ValueError("refinement fixture changed while being read")
    if before[1] != expected_sha256.lower():
        raise ValueError("refinement fixture SHA-256 differs from the supplied pin")
    return before[1]


def run(corpus_dir: Path | None, fixture: Path | None, fixture_sha256: str | None) -> dict:
    report = initial_report()
    try:
        if (fixture is None) != (fixture_sha256 is None):
            raise ValueError("fixture path and SHA-256 must be supplied together")
        if fixture is not None and fixture_sha256 is not None:
            report["fixture"] = {
                "status": "VERIFIED",
                "sha256": verify_fixture(fixture, fixture_sha256),
            }
        if corpus_dir is not None:
            metadata = jbig2_dictionary_headers.run(corpus_dir)
            if metadata["status"] != "PASS" or metadata["metadata"]["checked_images"] != 546:
                raise ValueError("requested corpus failed the SHA-pinned dictionary inventory")
            report["corpus_metadata"] = {"status": "PASS", "checked_images": 546}
        if report["fixture"]["status"] == "VERIFIED":
            report["refinement_compatibility"]["reason"] = (
                "fixture identity verified; no independent refinement-pixel comparison is wired"
            )
    except (OSError, ValueError, KeyError, jbig2_dictionary_headers.InventoryError) as error:
        report["status"] = "FAIL"
        report["reason"] = str(error)
        if fixture is not None and report["fixture"]["status"] != "VERIFIED":
            report["fixture"] = {"status": "FAIL"}
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=None)
    parser.add_argument("--fixture-file", type=Path, default=None)
    parser.add_argument("--fixture-sha256", default=None)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    corpus = args.corpus_dir
    if corpus is None and os.environ.get("CAJ2PDF_CORPUS_DIR"):
        corpus = Path(os.environ["CAJ2PDF_CORPUS_DIR"])
    fixture = args.fixture_file
    if fixture is None and os.environ.get("CAJ2PDF_T88_REFINEMENT_FIXTURE_FILE"):
        fixture = Path(os.environ["CAJ2PDF_T88_REFINEMENT_FIXTURE_FILE"])
    fixture_sha256 = (
        args.fixture_sha256
        or os.environ.get("CAJ2PDF_T88_REFINEMENT_FIXTURE_SHA256")
        or None
    )
    report = run(corpus, fixture, fixture_sha256)
    print(json.dumps(report, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
