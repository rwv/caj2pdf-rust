#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, metadata-only inventory of the pinned JBIG2 text-region headers.

The #42 Rust directory inventory locates each type-6 text region #3. This
script reads only its 23-byte observed header (T.88 §§7.4.1, 7.4.3.1) and
checks the result against the #43 oracle coordinates and pinned aggregates.
It never reads the arithmetic body or decodes a symbol instance. A metadata
PASS is not text-placement or pixel compatibility.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import sys

import conformance
import jbig2_oracle as full


ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/conformance/matrix.json"
FULL_ORACLE = ROOT / "tests/conformance/jbig2_oracle.json"
EXPECTED_IMAGES = 546
EXPECTED_SOURCES = 27
HEADER_BYTES = 23
MAX_TEXT_DATA_BYTES = 64 * 1024 * 1024
# Pinned repository measurements (#69). Counts and offsets only.
EXPECTED_FLAGS = {
    "0x840e": 1, "0x880e": 90, "0x8c0e": 87, "0x900e": 89, "0x940e": 62,
    "0x980e": 61, "0x9c0e": 62, "0xa00e": 34, "0xa40c": 1, "0xa40e": 10,
    "0xa80e": 14, "0xac0e": 2, "0xb00e": 4, "0xb80e": 4, "0xbc0e": 25,
}
EXPECTED_AGGREGATES = {
    "headers": EXPECTED_IMAGES,
    "flags": EXPECTED_FLAGS,
    "header_bytes": {"23": EXPECTED_IMAGES},
    "refinement": {"0": 1, "1": 545},
    "instance_range": [6, 15576],
    "instance_sum": 354063,
    "data_byte_range": [64, 28634],
    "body_byte_range": [41, 28611],
    "anomalies": [{"id": full.ANOMALY_ID, "page": 11, "image": 1,
                   "record_offset": 930673, "flags": "0xa40c"}],
}


class InventoryError(Exception):
    """A pinned source, coordinate, header, or aggregate claim differed."""


def initial_report() -> dict:
    return {
        "schema_version": 1, "status": "NOT_RUN", "phase": "preflight",
        "source_hashes_before": 0, "source_hashes_after": 0,
        "metadata": {"status": "NOT_RUN", "expected_images": EXPECTED_IMAGES,
                     "checked_images": 0, "aggregates": {}},
        "text_compatibility": {"status": "NOT_RUN", "checked_cases": 0,
                               "passed": 0, "failed": 0,
                               "reason": "metadata only; no text-instance decoder or pixel oracle"},
        "failures": [],
    }


def load_baseline() -> tuple[list[dict], dict]:
    """Validate the committed #43 oracle and its per-image text flags."""
    try:
        oracle = json.loads(FULL_ORACLE.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise InventoryError(f"cannot read #43 manifest: {exc}") from exc
    full.validate_manifest(oracle)
    rows = [row for row in conformance.load_matrix(MATRIX) if row["variant"] in ("HN", "C8")]
    if len(rows) != EXPECTED_SOURCES:
        raise InventoryError(f"HN/C8 matrix has {len(rows)} sources, expected 27")
    images = {
        (sample["id"], image["page"], image["image"]): image
        for sample in oracle["samples"] for image in sample["images"]
    }
    flags = Counter(image["profile"]["text_flags"] for image in images.values())
    if dict(sorted(flags.items())) != EXPECTED_FLAGS:
        raise InventoryError("#43 oracle text flag distribution differs from pinned profile")
    anomalies = [key for key, image in images.items() if image["profile"]["text_flag_anomaly"]]
    if anomalies != [full.ANOMALY_COORDINATE]:
        raise InventoryError("#43 oracle text flag anomaly coordinate differs")
    return rows, images


def audit_sources(rows: list[dict], corpus: Path) -> Path:
    result = conformance.audit_inventory(rows, corpus)
    if result["status"] != "PASS" or result["passed"] != EXPECTED_SOURCES:
        detail = result.get("reason") or result.get("failures")
        raise InventoryError(f"27-source size/SHA-256 audit failed: {detail}")
    return corpus.resolve(strict=True)


def parse_text_header(case: dict, segment: dict) -> dict:
    """Read the observed arithmetic header; refuse Huffman or refinement AT."""
    label = f"{case['id']} page {case['page']} image {case['image']} text region"
    data_offset = segment["data_offset"]
    data_length = segment["data_length"]
    if data_length < HEADER_BYTES + 2 or data_length > MAX_TEXT_DATA_BYTES:
        raise InventoryError(f"{label}: declared data size is outside 25 bytes..64 MiB")
    with case["source_path"].open("rb") as source:
        header = full.read_span(source, data_offset, HEADER_BYTES)
    width, height, x, y = (int.from_bytes(header[i:i + 4], "big") for i in range(0, 16, 4))
    region_flags = header[16]
    flags = int.from_bytes(header[17:19], "big")
    if flags & 1 or (flags & 2 and not flags & 0x8000):
        raise InventoryError(f"{label}: flags 0x{flags:04x} are outside the observed layout")
    if (width, height, x, y, region_flags) != (case["width"], case["height"], 0, 0, 0):
        raise InventoryError(f"{label}: region does not cover the page at (0,0) with OR")
    return {
        "flags": f"0x{flags:04x}",
        "refinement": (flags >> 1) & 1,
        "instances": int.from_bytes(header[19:23], "big"),
        "header_bytes": HEADER_BYTES,
        "data_offset": data_offset, "data_length": data_length,
        "body_offset": data_offset + HEADER_BYTES, "body_length": data_length - HEADER_BYTES,
    }


def inspect_case(case: dict, recorded: dict) -> dict:
    """Check the #42 coordinate/span against #43, then read only the header."""
    label = f"{case['id']} page {case['page']} image {case['image']}"
    if (case["offset"], case["length"]) != (recorded["offset"], recorded["length"]):
        raise InventoryError(f"{label}: image span differs from #43 oracle")
    segment = case["segments"][3]
    if (segment["type"], segment["refs"], segment["page_association"]) != (6, [2], 1):
        raise InventoryError(f"{label}: segment 3 is not a text region referring to #2")
    header = parse_text_header(case, segment)
    if header["flags"] != recorded["profile"]["text_flags"]:
        raise InventoryError(f"{label}: text flags differ from #43 oracle")
    return header


def aggregate(observed: list[tuple[tuple, dict, int]]) -> dict:
    headers = [header for _, header, _ in observed]
    ranges = {
        field: [min(item[field] for item in headers), max(item[field] for item in headers)]
        for field in ("instances", "data_length", "body_length")
    }
    return {
        "headers": len(headers),
        "flags": dict(sorted(Counter(item["flags"] for item in headers).items())),
        "header_bytes": {str(key): value for key, value in sorted(Counter(
            item["header_bytes"] for item in headers).items())},
        "refinement": {str(key): value for key, value in sorted(Counter(
            item["refinement"] for item in headers).items())},
        "instance_range": ranges["instances"],
        "instance_sum": sum(item["instances"] for item in headers),
        "data_byte_range": ranges["data_length"],
        "body_byte_range": ranges["body_length"],
        "anomalies": [
            {"id": key[0], "page": key[1], "image": key[2],
             "record_offset": record_offset, "flags": header["flags"]}
            for key, header, record_offset in observed
            # SBRTEMPLATE set without SBREFINE violates T.88 §7.4.3.1.1.
            if header["refinement"] == 0 and int(header["flags"], 16) & 0x8000
        ],
    }


def run(corpus: Path | None, report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    rows, recorded = load_baseline()
    if corpus is None:
        report["reason"] = "external CAJSamples corpus is unset"
        return report
    report["phase"] = "source_precheck"
    root = audit_sources(rows, corpus)
    report["source_hashes_before"] = len(rows)
    paths = {row["id"]: conformance.contained_file(
        root, conformance.relative_path(row["path"])
    ) for row in rows}
    observed: list[tuple[tuple, dict, int]] = []
    error: Exception | None = None
    report["phase"] = "directory_and_headers"
    try:
        cases = full.checked_cases(full.run_directory_inventory(root), rows, paths)
        if len(cases) != EXPECTED_IMAGES:
            raise InventoryError(f"Rust directory found {len(cases)} images, expected 546")
        for case in cases:
            key = case["coordinate"]
            if key not in recorded:
                raise InventoryError(f"unpinned text region coordinate {key!r}")
            observed.append((key, inspect_case(case, recorded[key]), case["offset"]))
        if {key for key, _, _ in observed} != set(recorded):
            raise InventoryError("text region inventory omitted a pinned image")
        summary = aggregate(observed)
        if summary != EXPECTED_AGGREGATES:
            raise InventoryError("text region aggregates differ from pinned profile")
    except (InventoryError, full.OracleError, OSError, ValueError, KeyError) as exc:
        error = exc
    report["phase"] = "source_postcheck"
    try:
        audit_sources(rows, root)
        report["source_hashes_after"] = len(rows)
    except (InventoryError, conformance.ConformanceError, OSError) as exc:
        if error is None:
            error = InventoryError(f"post-run source audit failed: {exc}")
        else:
            error = InventoryError(f"{error}; post-run source audit also failed: {exc}")
    if error is not None:
        report["status"] = "FAIL"
        report["metadata"]["status"] = "FAIL"
        raise InventoryError(str(error))
    report["phase"] = "complete"
    report["metadata"].update(status="PASS", checked_images=len(observed), aggregates=summary)
    report["status"] = "PASS"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=(
        Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None
    ))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, report)
    except (InventoryError, conformance.ConformanceError, full.OracleError,
            OSError, ValueError, KeyError, TypeError) as exc:
        report["status"] = "FAIL"
        report["failures"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        item = report["metadata"]
        print(f"JBIG2 text region headers [{report['status']}]: "
              f"metadata={item['checked_images']}/{item['expected_images']}, "
              f"text compatibility={report['text_compatibility']['checked_cases']} "
              f"[{report['text_compatibility']['status']}]")
        for failure in report["failures"]:
            print(f"FAIL: {failure}", file=sys.stderr)
        if report.get("reason"):
            print(report["reason"], file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
