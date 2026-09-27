#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional SHA-pinned Rust text-region pixel comparison against issue #85.

The private MQ table and external CAJSamples files stay outside Git. Rust
receives no expected pixel hash: this script compares its completed output.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

import jbig2_text_instance_diagnostic as instances
import jbig2_text_oracle as oracle


EXPECTED = instances.EXPECTED
STANDARD = instances.STANDARD
MAX_REQUEST_BYTES = 256 * 1024
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()


class ParityError(instances.DiagnosticError):
    """A pinned input, bounded decoder result, or pixel comparison failed."""


def initial_report() -> dict:
    return {
        "status": "NOT_RUN", "phase": "preflight", "expected_images": EXPECTED,
        "source_hashes_before": 0, "source_hashes_after": 0,
        "table_hashes_before": 0, "table_hashes_after": 0,
        "compatibility": {
            "status": "NOT_RUN", "attempted": 0, "standard_attempted": 0,
            "completed": 0, "matching": 0, "failing": 0, "skipped": 0,
            "first_typed_refusal": None, "first_standard_refusal": None,
            "first_pixel_mismatch": None, "cases": [],
        },
        "strict_header_refusals": 0,
        "resources": {
            "peak_resident_bytes": 0, "max_request_bytes": 0,
            "max_temporary_bytes": 0, "large_page": None,
        },
    }


def plan_for_cases(cases: list[dict], recorded: dict, dictionary_manifest: dict,
                   text_manifest: dict) -> tuple[list[str], list[dict]]:
    lines, control = instances.plan_for_cases(
        cases, recorded, dictionary_manifest, text_manifest)
    baseline = {(sample["id"], image["page"], image["image"]): image
                for sample in text_manifest["samples"] for image in sample["images"]}
    if len(baseline) != EXPECTED:
        raise ParityError("#85 pixel manifest lacks a unique 546-case baseline")
    selected = []
    for case, (key, kind, declared, flags_offset) in zip(cases, control):
        reference = baseline.get(key)
        if (reference is None or case["coordinate"] != key
                or (case["width"], case["height"]) !=
                (reference["width"], reference["height"])):
            raise ParityError("pixel coordinate or dimensions differ from #85")
        selected.append({
            "coordinate": key, "classification": kind, "instances": declared,
            "flags_offset": flags_offset, "width": reference["width"],
            "height": reference["height"],
            "pixel_sha256": reference["normalized_pixel_sha256"],
            "black_pixels": reference["black_pixels"],
        })
    if len(selected) != EXPECTED:
        raise ParityError("text-region pixel plan is incomplete")
    return lines, selected


def parse_output(output: str, selected: list[dict]) -> tuple[dict, int, dict]:
    lines = output.splitlines()
    if len(selected) != EXPECTED or len(lines) != EXPECTED + 1:
        raise ParityError(f"Rust emitted {len(lines)} lines, expected {EXPECTED + 1}")
    report = initial_report()["compatibility"]
    resources = initial_report()["resources"]
    report.update(status="FAIL", attempted=EXPECTED, standard_attempted=STANDARD,
                  skipped=0)
    strict_header_refusals = 0
    largest_area = -1
    for number, (line, expected) in enumerate(zip(lines[:-1], selected)):
        fields = line.split("\t")
        if len(fields) != 16 or fields[0] != "CASE":
            raise ParityError(f"Rust case line {number} has invalid fields")
        try:
            coordinate = (fields[1], int(fields[2]), int(fields[3]))
            completed, rows, output_bytes, black, scratch, request, resident, offset = (
                int(fields[index]) for index in (5, 6, 7, 8, 10, 11, 12, 14))
        except ValueError as exc:
            raise ParityError(f"Rust case line {number} has invalid numbers") from exc
        status, pixel_sha, refusal, stage = fields[4], fields[9], fields[13], fields[15]
        width, height = expected["width"], expected["height"]
        raster_bytes = ((width + 7) // 8) * height
        if (coordinate != expected["coordinate"]
                or min(completed, rows, output_bytes, black, scratch, request, resident, offset) < 0
                or completed > expected["instances"] or rows > height
                or output_bytes > raster_bytes or black > width * height
                or request > MAX_REQUEST_BYTES
                or re.fullmatch(r"[0-9a-f]{64}", pixel_sha) is None
                or re.fullmatch(r"[A-Za-z][A-Za-z0-9]*", stage) is None):
            raise ParityError(f"Rust case line {number} differs from pinned plan")
        entry = {"coordinate": coordinate, "status": status,
                 "completed_instances": completed, "rows": rows,
                 "output_bytes": output_bytes, "black_pixels": black,
                 "pixel_sha256": pixel_sha, "temporary_bytes": scratch,
                 "max_request_bytes": request, "peak_resident_bytes": resident}
        if expected["classification"] == oracle.ANOMALY:
            if (status != "HEADER_REFUSED" or completed or rows or output_bytes or black
                    or scratch or request or pixel_sha != EMPTY_SHA256
                    or refusal != "malformed_text_header" or stage != "Header"
                    or offset != expected["flags_offset"]):
                raise ParityError("0xa40c did not fail as a strict header")
            strict_header_refusals += 1
        elif status == "COMPLETE":
            if (refusal != "-" or stage != "Complete"
                    or completed != expected["instances"] or rows != height
                    or output_bytes != raster_bytes or scratch != raster_bytes):
                raise ParityError(f"Rust complete case {number} has incomplete output")
            report["completed"] += 1
            if (pixel_sha == expected["pixel_sha256"]
                    and black == expected["black_pixels"]):
                report["matching"] += 1
                entry["pixel_match"] = True
            else:
                entry["pixel_match"] = False
                if report["first_pixel_mismatch"] is None:
                    report["first_pixel_mismatch"] = {
                        "coordinate": coordinate, "expected_sha256": expected["pixel_sha256"],
                        "actual_sha256": pixel_sha, "expected_black": expected["black_pixels"],
                        "actual_black": black,
                    }
        elif status == "REFUSED":
            if not refusal or refusal == "-" or any(char.isspace() for char in refusal):
                raise ParityError(f"Rust case {number} lacks a typed refusal")
            failure = {"coordinate": coordinate, "kind": refusal,
                       "source_byte_offset": offset, "stage": stage,
                       "completed_instances": completed, "completed_rows": rows}
            if report["first_standard_refusal"] is None:
                report["first_standard_refusal"] = failure
            if report["first_typed_refusal"] is None:
                report["first_typed_refusal"] = failure
        else:
            raise ParityError(f"Rust case {number} has invalid status {status!r}")
        report["cases"].append(entry)
        resources["peak_resident_bytes"] = max(resources["peak_resident_bytes"], resident)
        resources["max_request_bytes"] = max(resources["max_request_bytes"], request)
        resources["max_temporary_bytes"] = max(resources["max_temporary_bytes"], scratch)
        area = width * height
        if expected["classification"] == oracle.STANDARD and area > largest_area:
            largest_area = area
            resources["large_page"] = {"coordinate": coordinate, "width": width,
                                       "height": height, "status": status,
                                       "pixel_match": entry.get("pixel_match", False),
                                       "peak_resident_bytes": resident,
                                       "temporary_bytes": scratch}
    if lines[-1].split("\t") != ["TOTAL", str(EXPECTED)]:
        raise ParityError("Rust total does not acknowledge all 546 attempts")
    report["failing"] = STANDARD - report["matching"]
    if report["matching"] == STANDARD and strict_header_refusals == 1:
        report["status"] = "PASS"
    return report, strict_header_refusals, resources


def build_binary(override: Path | None) -> Path:
    if override is not None:
        return override.resolve(strict=True)
    result = instances.parity.bounded_run([
        "cargo", "build", "--locked", "--release", "-p", "caj2pdf-core",
        "--example", "jbig2_text_region_parity",
    ], "Rust text-region parity build", 300)
    if result.returncode:
        raise ParityError(f"Rust parity build failed: {result.stderr[-4096:]}")
    return instances.ROOT / "target/release/examples/jbig2_text_region_parity"


def run_binary(binary: Path, fixture: Path, lines: list[str], selected: list[dict]) -> tuple[dict, int, dict]:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-text-region-parity-", dir="/tmp") as temp:
        plan = Path(temp) / "plan.tsv"
        plan.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if plan.stat().st_size > instances.MAX_PLAN_BYTES:
            raise ParityError("written text-region parity plan exceeds 2 MiB")
        output = instances.bounded_decode([str(binary), str(fixture), str(plan)])
        return parse_output(output, selected)


def run(corpus: Path | None, fixture: Path | None, binary: Path | None = None,
        manifest_path: Path = oracle.DEFAULT_MANIFEST,
        report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    text_manifest = instances.load_text_manifest(manifest_path)
    if corpus is None and fixture is None:
        report["reason"] = "external corpus and private T.88 state table are unset"
        return report
    if corpus is None or fixture is None:
        raise ParityError("corpus and private T.88 state table must be supplied together")
    fixture = instances.parity.check_table(fixture)
    report["table_hashes_before"] = 1
    rows, recorded = instances.headers.load_baseline()
    _, dictionary_manifest = instances.dictionaries.load_baseline()
    report["phase"] = "source_precheck"
    root = instances.headers.audit_sources(rows, corpus)
    report["source_hashes_before"] = len(rows)
    try:
        report["phase"] = "inventory"
        paths = {row["id"]: instances.headers.conformance.contained_file(
            root, instances.headers.conformance.relative_path(row["path"])) for row in rows}
        cases = instances.full.checked_cases(instances.full.run_directory_inventory(root), rows, paths)
        lines, selected = plan_for_cases(cases, recorded, dictionary_manifest, text_manifest)
        report["phase"] = "rust_build"
        selected_binary = build_binary(binary)
        report["phase"] = "rust_decode"
        compatibility, strict, resources = run_binary(selected_binary, fixture, lines, selected)
        report.update(compatibility=compatibility, strict_header_refusals=strict,
                      resources=resources)
    finally:
        active_phase = report["phase"]
        report["phase"] = "source_postcheck"
        failures = []
        try:
            instances.headers.audit_sources(rows, root)
            report["source_hashes_after"] = len(rows)
        except (instances.headers.InventoryError, OSError) as exc:
            failures.append(f"source postcheck: {exc}")
        try:
            instances.parity.check_table(fixture)
            report["table_hashes_after"] = 1
        except (instances.parity.ParityError, OSError) as exc:
            failures.append(f"table postcheck: {exc}")
        if failures:
            raise ParityError("; ".join(failures))
        report["phase"] = active_phase
    if report["compatibility"]["status"] == "PASS":
        report.update(status="PASS", phase="complete")
    else:
        report.update(status="FAIL", phase="complete",
                      error="one or more standard text-region pixels did not match #85")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path,
                        default=Path(os.environ["CAJ2PDF_CORPUS_DIR"])
                        if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--table-fixture", type=Path,
                        default=Path(os.environ["CAJ2PDF_T88_H2_FIXTURE_FILE"])
                        if os.environ.get("CAJ2PDF_T88_H2_FIXTURE_FILE") else None)
    parser.add_argument("--manifest", type=Path, default=oracle.DEFAULT_MANIFEST)
    parser.add_argument("--rust-bin", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, args.table_fixture, args.rust_bin, args.manifest, report)
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError,
            subprocess.TimeoutExpired, instances.dictionaries.InventoryError,
            instances.headers.InventoryError, instances.full.OracleError,
            instances.parity.ParityError, instances.DiagnosticError) as exc:
        report.update(status="FAIL", error=str(exc))
        report["compatibility"]["status"] = "FAIL"
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
