#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, metadata-only inventory of the pinned JBIG2 dictionaries.

The existing #42 Rust directory inventory locates each segment. This script
reads only the two observed dictionary headers and streams SHA-256 over their
declared data spans and enclosing type-3 image records. It never decodes or
copies a symbol bitmap. Metadata PASS is not symbol-pixel compatibility.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import sys

import conformance
import jbig2_oracle as full


ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/conformance/matrix.json"
MANIFEST = ROOT / "tests/conformance/jbig2_dictionary_headers.json"
FULL_ORACLE = ROOT / "tests/conformance/jbig2_oracle.json"
MATRIX_SHA256 = "af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9"
MANIFEST_SHA256 = "e4897fcde9f0fea32471d58790bad8246776ed2ba1f1d8586f2f8b6adf65ad52"
OFFICIAL_T88_SHA256 = "a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69"
DIRECTORY_REPORT_SHA256 = "f7d7ecf2b10667cbe153e5c25b2080da5fd029555cb683734c676faa50b2a47e"
EXPECTED_IMAGES = 546
EXPECTED_SOURCES = 27
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_DICTIONARY_BYTES = 64 * 1024 * 1024
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
SHA1 = re.compile(r"[0-9a-f]{40}\Z")
ROOT_KEYS = {"schema_version", "matrix_sha256", "official_t88_pdf_sha256",
             "directory_report_sha256", "samples"}
SAMPLE_KEYS = {"id", "source_size", "source_sha256", "source_git_blob_oid", "images"}
IMAGE_KEYS = {"page", "image", "record_offset", "record_length", "record_sha256", "dictionaries"}
DICTIONARY_KEYS = {"number", "refs", "flags", "huffman", "refinement_aggregate",
                   "template", "refinement_template", "bitmap_context_used",
                   "bitmap_context_retained", "at", "refinement_at", "exported", "new",
                   "data_offset", "data_length", "data_sha256", "header_length",
                   "body_offset", "body_length"}


class InventoryError(Exception):
    """A pinned source, header, span digest, or metadata claim differed."""


def exact_keys(value: object, keys: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != keys:
        raise InventoryError(f"{label} has missing or unexpected fields")
    return value


def integer(value: object, label: str, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        raise InventoryError(f"{label} must be an integer >= {minimum}")
    return value


def digest(value: object, label: str, sha1: bool = False) -> str:
    pattern = SHA1 if sha1 else SHA256
    if not isinstance(value, str) or pattern.fullmatch(value) is None:
        raise InventoryError(f"{label} is not a lowercase {'SHA-1' if sha1 else 'SHA-256'}")
    return value


def initial_report() -> dict:
    return {
        "schema_version": 1, "status": "NOT_RUN", "phase": "preflight",
        "source_hashes_before": 0, "source_hashes_after": 0,
        "metadata": {"status": "NOT_RUN", "expected_images": EXPECTED_IMAGES,
                     "checked_images": 0, "span_digests_checked": 0, "aggregates": {}},
        "symbol_compatibility": {"status": "NOT_RUN", "checked_cases": 0,
                                 "passed": 0, "failed": 0,
                                 "reason": "no independent per-symbol pixel oracle or private Table E.1 fixture"},
        "failures": [],
    }


def validate_manifest(manifest: object, matrix_rows: list[dict], oracle: dict) -> dict:
    """Reject altered baselines and any field that could hide encoded bytes."""
    manifest = exact_keys(manifest, ROOT_KEYS, "dictionary manifest")
    if type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1:
        raise InventoryError("dictionary manifest schema_version is not 1")
    if (digest(manifest["matrix_sha256"], "matrix_sha256") != MATRIX_SHA256
            or full.sha256_file(MATRIX) != MATRIX_SHA256):
        raise InventoryError("dictionary manifest matrix SHA-256 differs")
    if manifest["official_t88_pdf_sha256"] != OFFICIAL_T88_SHA256:
        raise InventoryError("dictionary manifest official T.88 SHA-256 differs")
    if manifest["directory_report_sha256"] != DIRECTORY_REPORT_SHA256:
        raise InventoryError("dictionary manifest #42 directory report SHA-256 differs")
    source_rows = {row["id"]: row for row in matrix_rows}
    oracle_rows = {sample["id"]: sample for sample in oracle["samples"]}
    samples = manifest["samples"]
    if not isinstance(samples, list) or len(samples) != len(full.EXPECTED_TYPE3):
        raise InventoryError("dictionary manifest requires five source samples")
    seen_samples: set[str] = set()
    image_total = 0
    observed_dictionaries: dict[int, list[dict]] = {1: [], 2: []}
    for sample in samples:
        sample = exact_keys(sample, SAMPLE_KEYS, "dictionary sample")
        sample_id = sample["id"]
        if (not isinstance(sample_id, str) or sample_id not in full.EXPECTED_TYPE3
                or sample_id in seen_samples):
            raise InventoryError(f"unexpected or duplicate dictionary source {sample_id!r}")
        seen_samples.add(sample_id)
        row = source_rows[sample_id]
        if (integer(sample["source_size"], "source_size", 1) != row["size_bytes"]
                or digest(sample["source_sha256"], "source_sha256") != row["sha256"]
                or digest(sample["source_git_blob_oid"], "source_git_blob_oid", True)
                != row["git_blob_oid"]):
            raise InventoryError(f"{sample_id}: source identity differs from matrix")
        oracle_sample = oracle_rows[sample_id]
        oracle_images = {
            (image["page"], image["image"]): image for image in oracle_sample["images"]
        }
        images = sample["images"]
        if not isinstance(images, list) or len(images) != full.EXPECTED_TYPE3[sample_id]:
            raise InventoryError(f"{sample_id}: dictionary image count differs")
        seen_images: set[tuple[int, int]] = set()
        for image in images:
            image = exact_keys(image, IMAGE_KEYS, "dictionary image")
            page = integer(image["page"], "page", 1)
            index = integer(image["image"], "image", 1)
            key = (page, index)
            label = f"{sample_id} page {page} image {index}"
            if key in seen_images or key not in oracle_images:
                raise InventoryError(f"{label}: duplicate or unpinned image coordinate")
            seen_images.add(key)
            record_offset = integer(image["record_offset"], "record_offset")
            record_length = integer(image["record_length"], "record_length", 49)
            if (record_length > MAX_DICTIONARY_BYTES
                    or record_offset + record_length > sample["source_size"]):
                raise InventoryError(f"{label}: image record is outside bounded source")
            recorded = oracle_images[key]
            record_sha = digest(image["record_sha256"], "record_sha256")
            if (record_offset, record_length, record_sha) != (
                recorded["offset"], recorded["length"], recorded["encoded_sha256"]
            ):
                raise InventoryError(f"{label}: image span/hash differs from #43 oracle")
            dictionaries = image["dictionaries"]
            if not isinstance(dictionaries, list) or len(dictionaries) != 2:
                raise InventoryError(f"{label}: expected exactly dictionaries #1 and #2")
            for number, dictionary in enumerate(dictionaries, 1):
                dictionary = exact_keys(dictionary, DICTIONARY_KEYS, f"{label} dictionary {number}")
                expected_flags = 0x0800 if number == 1 else 0x1802
                expected_refs = [] if number == 1 else [1]
                for field in ("number", "flags", "huffman", "refinement_aggregate",
                              "template", "refinement_template", "bitmap_context_used",
                              "bitmap_context_retained", "header_length"):
                    integer(dictionary[field], field)
                if (not isinstance(dictionary["refs"], list)
                        or any(type(value) is not int for value in dictionary["refs"])
                        or not isinstance(dictionary["at"], list)
                        or len(dictionary["at"]) != 2
                        or any(type(value) is not int or value < -128 or value > 127
                               for value in dictionary["at"])
                        or not isinstance(dictionary["refinement_at"], list)):
                    raise InventoryError(f"{label}: dictionary {number} AT/reference fields are invalid")
                expected_fields = (number, expected_refs, expected_flags, 0, number - 1,
                                   2, number - 1, 0, 0, [2, -1], [], 12)
                actual_fields = tuple(dictionary[field] for field in (
                    "number", "refs", "flags", "huffman", "refinement_aggregate",
                    "template", "refinement_template", "bitmap_context_used",
                    "bitmap_context_retained", "at", "refinement_at", "header_length"
                ))
                if actual_fields != expected_fields:
                    raise InventoryError(f"{label}: dictionary {number} mode/AT differs")
                exported = integer(dictionary["exported"], "exported")
                new = integer(dictionary["new"], "new")
                if exported > 0xffff_ffff or new > 0xffff_ffff:
                    raise InventoryError(f"{label}: symbol count exceeds its 32-bit header field")
                data_offset = integer(dictionary["data_offset"], "data_offset")
                data_length = integer(dictionary["data_length"], "data_length", 12)
                body_offset = integer(dictionary["body_offset"], "body_offset")
                body_length = integer(dictionary["body_length"], "body_length")
                digest(dictionary["data_sha256"], "data_sha256")
                if (data_length > MAX_DICTIONARY_BYTES
                        or data_offset < record_offset + 48
                        or data_offset + data_length > record_offset + record_length
                        or body_offset != data_offset + 12
                        or body_length != data_length - 12):
                    raise InventoryError(f"{label}: dictionary {number} data/body span is invalid")
                if number == 1 and (new < 3 or exported != new):
                    raise InventoryError(f"{label}: first dictionary symbol count differs")
                if number == 2 and exported != dictionaries[0]["exported"] + new:
                    raise InventoryError(f"{label}: second dictionary count relation differs")
                observed_dictionaries[number].append(dictionary)
            first, second = dictionaries
            if second["data_offset"] < first["data_offset"] + first["data_length"]:
                raise InventoryError(f"{label}: dictionary data spans overlap or regress")
            image_total += 1
        if seen_images != set(oracle_images):
            raise InventoryError(f"{sample_id}: dictionary image coordinates are incomplete")
    if seen_samples != set(full.EXPECTED_TYPE3) or image_total != EXPECTED_IMAGES:
        raise InventoryError("dictionary manifest lacks the 546 pinned images")
    expected_ranges = {
        1: {"new": (3, 514), "exported": (3, 514), "data_length": (60, 9959)},
        2: {"new": (0, 318), "exported": (3, 542), "data_length": (15, 1394)},
    }
    for number, fields in expected_ranges.items():
        entries = observed_dictionaries[number]
        for field, bounds in fields.items():
            if (min(item[field] for item in entries), max(item[field] for item in entries)) != bounds:
                raise InventoryError(f"dictionary {number} {field} range differs from pinned profile")
    if sum(item["new"] == 0 for item in observed_dictionaries[2]) != 57:
        raise InventoryError("dictionary 2 zero-new count differs from pinned profile")
    return manifest


def load_baseline(path: Path = MANIFEST) -> tuple[list[dict], dict]:
    try:
        if path.stat().st_size > MAX_MANIFEST_BYTES:
            raise InventoryError("dictionary metadata manifest exceeds 1 MiB")
        if full.sha256_file(path) != MANIFEST_SHA256:
            raise InventoryError("dictionary metadata manifest SHA-256 differs from pinned report")
        manifest = json.loads(path.read_text(encoding="utf-8"))
        oracle = json.loads(FULL_ORACLE.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise InventoryError(f"cannot read dictionary or #43 manifest: {exc}") from exc
    full.validate_manifest(oracle)
    rows = [row for row in conformance.load_matrix(MATRIX)
            if row["variant"] in ("HN", "C8")]
    if len(rows) != EXPECTED_SOURCES:
        raise InventoryError(f"HN/C8 matrix has {len(rows)} sources, expected 27")
    return rows, validate_manifest(manifest, rows, oracle)


def audit_sources(rows: list[dict], corpus: Path) -> Path:
    result = conformance.audit_inventory(rows, corpus)
    if result["status"] != "PASS" or result["passed"] != EXPECTED_SOURCES:
        detail = result.get("reason") or result.get("failures")
        raise InventoryError(f"27-source size/SHA-256 audit failed: {detail}")
    return corpus.resolve(strict=True)


def parse_dictionary_header(case: dict, segment: dict, number: int) -> dict:
    """Read the observed §7.4.2.1 header; refuse any other flag layout."""
    label = f"{case['id']} page {case['page']} image {case['image']} dictionary {number}"
    data_offset = segment["data_offset"]
    data_length = segment["data_length"]
    if data_length < 12 or data_length > MAX_DICTIONARY_BYTES:
        raise InventoryError(f"{label}: declared dictionary data size is outside 12..64 MiB")
    with case["source_path"].open("rb") as source:
        source.seek(data_offset)
        header = source.read(12)
        if len(header) != 12:
            raise InventoryError(f"{label}: truncated dictionary header")
        flags = int.from_bytes(header[0:2], "big")
        expected_flags = 0x0800 if number == 1 else 0x1802
        if flags != expected_flags:
            raise InventoryError(f"{label}: flags 0x{flags:04x} differ from observed 0x{expected_flags:04x}")
        at = [int.from_bytes(header[2:3], "big", signed=True),
              int.from_bytes(header[3:4], "big", signed=True)]
        exported = int.from_bytes(header[4:8], "big")
        new = int.from_bytes(header[8:12], "big")
        data_sha = full.sha256_span(source, data_offset, data_length)
    return {
        "number": number, "refs": segment["refs"], "flags": flags,
        "huffman": flags & 1, "refinement_aggregate": (flags >> 1) & 1,
        "template": (flags >> 10) & 3, "refinement_template": (flags >> 12) & 1,
        "bitmap_context_used": (flags >> 8) & 1,
        "bitmap_context_retained": (flags >> 9) & 1,
        "at": at, "refinement_at": [], "exported": exported, "new": new,
        "data_offset": data_offset, "data_length": data_length,
        "data_sha256": data_sha, "header_length": 12,
        "body_offset": data_offset + 12, "body_length": data_length - 12,
    }


def inspect_case(case: dict) -> dict:
    with case["source_path"].open("rb") as source:
        record_sha = full.sha256_span(source, case["offset"], case["length"])
    dictionaries = [
        parse_dictionary_header(case, case["segments"][number], number)
        for number in (1, 2)
    ]
    return {
        "page": case["page"], "image": case["image"],
        "record_offset": case["offset"], "record_length": case["length"],
        "record_sha256": record_sha, "dictionaries": dictionaries,
    }


def aggregate(images: list[dict]) -> dict:
    results = {}
    for number in (1, 2):
        dictionaries = [image["dictionaries"][number - 1] for image in images]
        results[str(number)] = {
            "headers": len(dictionaries),
            "flags": {f"0x{key:04x}": value for key, value in sorted(Counter(
                item["flags"] for item in dictionaries
            ).items())},
            "adaptive_templates": {f"{key[0]},{key[1]}": value for key, value in sorted(Counter(
                tuple(item["at"]) for item in dictionaries
            ).items())},
            "new_symbol_count_range": [min(item["new"] for item in dictionaries),
                                       max(item["new"] for item in dictionaries)],
            "exported_symbol_count_range": [min(item["exported"] for item in dictionaries),
                                            max(item["exported"] for item in dictionaries)],
            "data_byte_range": [min(item["data_length"] for item in dictionaries),
                                max(item["data_length"] for item in dictionaries)],
            "body_byte_range": [min(item["body_length"] for item in dictionaries),
                                max(item["body_length"] for item in dictionaries)],
            "zero_new_count": sum(item["new"] == 0 for item in dictionaries),
            "new_symbol_count_distribution": {str(key): value for key, value in sorted(Counter(
                item["new"] for item in dictionaries
            ).items())},
            "exported_symbol_count_distribution": {
                str(key): value for key, value in sorted(Counter(
                    item["exported"] for item in dictionaries
                ).items())
            },
        }
    return results


def run(corpus: Path | None, manifest_path: Path = MANIFEST, report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    rows, manifest = load_baseline(manifest_path)
    if corpus is None:
        report["reason"] = "external CAJSamples corpus is unset"
        return report
    report["phase"] = "source_precheck"
    root = audit_sources(rows, corpus)
    report["source_hashes_before"] = len(rows)
    paths = {row["id"]: conformance.contained_file(
        root, conformance.relative_path(row["path"])
    ) for row in rows}
    expected = {
        (sample["id"], image["page"], image["image"]): image
        for sample in manifest["samples"] for image in sample["images"]
    }
    observed: list[dict] = []
    error: Exception | None = None
    report["phase"] = "directory_and_headers"
    try:
        directory = full.run_directory_inventory(root)
        cases = full.checked_cases(directory, rows, paths)
        if len(cases) != EXPECTED_IMAGES:
            raise InventoryError(f"Rust directory found {len(cases)} images, expected 546")
        for case in cases:
            key = case["coordinate"]
            if key not in expected:
                raise InventoryError(f"unlisted dictionary image coordinate {key!r}")
            image = inspect_case(case)
            if image != expected[key]:
                raise InventoryError(f"{key!r}: dictionary header, span, or SHA-256 drift")
            observed.append(image)
        if len(observed) != len(expected):
            raise InventoryError("dictionary inventory omitted a pinned image")
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
    report["metadata"].update(status="PASS", checked_images=len(observed),
                              span_digests_checked=len(observed) * 3,
                              aggregates=aggregate(observed))
    report["status"] = "PASS"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=(
        Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None
    ))
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, args.manifest, report)
    except (InventoryError, conformance.ConformanceError, full.OracleError,
            OSError, ValueError, KeyError, TypeError) as exc:
        report["status"] = "FAIL"
        report["failures"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        item = report["metadata"]
        print(f"JBIG2 dictionary headers [{report['status']}]: "
              f"metadata={item['checked_images']}/{item['expected_images']}, "
              f"symbol compatibility={report['symbol_compatibility']['checked_cases']} "
              f"[{report['symbol_compatibility']['status']}]")
        for failure in report["failures"]:
            print(f"FAIL: {failure}", file=sys.stderr)
        if report.get("reason"):
            print(report["reason"], file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
