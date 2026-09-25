#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, hash-pinned comparison of Rust HN/C8 container records.

This runner stores only source identities and record metadata. It reads the
external CAJSamples files only after all 27 sizes and SHA-256 values match the
committed matrix. A clean clone reports NOT_RUN and zero compatibility passes.
The Rust example is the parser under test; this script does not parse HN/C8.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import conformance  # noqa: E402
import jbig1_oracle  # noqa: E402


MATRIX = ROOT / "tests/conformance/matrix.json"
MANIFEST = ROOT / "tests/conformance/jbig1_oracle.json"
EXAMPLE = "hnc8_container_inventory"
EXPECTED_SOURCES = 27
EXPECTED_TYPE0 = 1400
EXPECTED_NONZERO = {1: 6, 2: 1085, 3: 546}
ISSUE_100 = "issue-100/中国金融体制改革阶段研究_李卉.caj"
EXPECTED_MANIFEST_INVALID = {
    (ISSUE_100, 2, 1, "image_record", "page 2 image 1 is outside the source file"),
    (ISSUE_100, 3, None, "page_index", "page 3 image count is negative"),
    (ISSUE_100, 4, None, "page_index", "page 4 text is outside the source file"),
}
EXPECTED_RUST_INVALID = {
    (ISSUE_100, 2, 1, 12886, "image type", "unsupported"),
    (ISSUE_100, 3, None, 264, "image count", "malformed"),
    (ISSUE_100, 4, None, 276, "text span", "truncated"),
}
MAX_OUTPUT_BYTES = 2 * 1024 * 1024
MAX_ERROR_BYTES = 64 * 1024
MAX_RECORDS_PER_SOURCE = 100_000
MAX_FAILURES_IN_REPORT = 100


class ComparisonError(Exception):
    """A source, manifest, Rust adapter, or comparison was invalid."""


def initial_report() -> dict:
    return {
        "schema_version": 1,
        "status": "NOT_RUN",
        "phase": "preflight",
        "expected_sources": EXPECTED_SOURCES,
        "source_hashes_before": 0,
        "source_hashes_after": 0,
        "type0": {
            "status": "NOT_RUN", "expected": EXPECTED_TYPE0,
            "passed": 0, "failed": 0, "missing": 0, "not_run": EXPECTED_TYPE0,
        },
        "non_type0": {"status": "NOT_RUN", "counts": {}, "expected_counts": {
            str(kind): count for kind, count in EXPECTED_NONZERO.items()
        }},
        "baseline_invalid": {
            "status": "NOT_RUN", "expected": len(EXPECTED_MANIFEST_INVALID),
            "observed": 0, "records": [],
        },
        "expected_invalid": {
            "status": "NOT_RUN", "expected": len(EXPECTED_RUST_INVALID),
            "observed": 0, "records": [],
        },
        "failures": [],
    }


def add_failure(report: dict, message: str) -> None:
    report["status"] = "FAIL"
    if len(report["failures"]) < MAX_FAILURES_IN_REPORT:
        report["failures"].append(message)


def baseline(matrix_path: Path = MATRIX, manifest_path: Path = MANIFEST) -> tuple[list[dict], dict]:
    """Validate the committed coordinates before a run may say NOT_RUN."""
    rows = [
        row for row in conformance.load_matrix(matrix_path)
        if row["detected_type"] in ("HN", "C8")
    ]
    if len(rows) != EXPECTED_SOURCES:
        raise ComparisonError(f"HN/C8 matrix has {len(rows)} sources, expected 27")
    if any(row.get("sha256") is None for row in rows):
        raise ComparisonError("every HN/C8 matrix source needs a SHA-256")
    manifest = jbig1_oracle.load_manifest(manifest_path)
    if jbig1_oracle.sha256_file(matrix_path) != manifest["corpus_matrix_sha256"]:
        raise ComparisonError("matrix SHA-256 differs from the JBIG1 manifest")
    jbig1_oracle.validate_manifest_inventory(manifest, rows, EXPECTED_TYPE0)
    invalid = {
        (item["sample_id"], item["page"], item.get("image"), item["kind"], item["detail"])
        for item in manifest["discovery_failures"]
    }
    if invalid != EXPECTED_MANIFEST_INVALID or len(manifest["discovery_failures"]) != len(invalid):
        raise ComparisonError("expected-invalid details differ from pinned issue-100 observations")
    return rows, manifest


def record_baseline_invalid(report: dict, manifest: dict) -> None:
    """Expose the #22 historical observations separately from Rust errors."""
    records = [dict(item) for item in manifest["discovery_failures"]]
    report["baseline_invalid"].update(status="PASS", observed=len(records), records=records)


def audit(rows: list[dict], corpus: Path) -> dict:
    result = conformance.audit_inventory(rows, corpus)
    if result["status"] != "PASS" or result["passed"] != EXPECTED_SOURCES:
        detail = result.get("reason") or result.get("failures")
        raise ComparisonError(f"27-source size/SHA-256 audit failed: {detail}")
    return result


def rust_binary(override: Path | None) -> Path:
    if override is not None:
        try:
            binary = override.resolve(strict=True)
        except OSError as exc:
            raise ComparisonError(f"requested Rust inventory binary is missing: {exc}") from exc
        if not binary.is_file():
            raise ComparisonError("requested Rust inventory binary is not a file")
        return binary
    command = ["cargo", "build", "--locked", "-p", "caj2pdf-core", "--example", EXAMPLE]
    try:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            process = subprocess.run(command, cwd=ROOT, stdout=output, stderr=errors,
                                     timeout=300, check=False)
            if process.returncode:
                errors.seek(0)
                detail = errors.read(MAX_ERROR_BYTES + 1)[-4096:].decode("utf-8", "replace")
                raise ComparisonError(f"Rust inventory example build failed: {detail}")
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ComparisonError(f"Rust inventory example build failed: {exc}") from exc
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    binary = target / "debug/examples" / EXAMPLE
    if not binary.is_file():
        raise ComparisonError(f"Rust inventory example is missing after build: {binary}")
    return binary


def run_example(binary: Path, source: Path, diagnostic: bool = False) -> tuple[int, str, str]:
    command = [str(binary), str(source)]
    if diagnostic:
        command.append("--diagnostic")
    try:
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            process = subprocess.run(command, cwd=ROOT, stdout=output, stderr=errors,
                                     timeout=120, check=False)
            if output.tell() > MAX_OUTPUT_BYTES or errors.tell() > MAX_ERROR_BYTES:
                raise ComparisonError(f"Rust inventory output exceeds bounded spool for {source.name}")
            output.seek(0)
            errors.seek(0)
            return (
                process.returncode,
                output.read(MAX_OUTPUT_BYTES + 1).decode("utf-8", "strict"),
                errors.read(MAX_ERROR_BYTES + 1).decode("utf-8", "replace"),
            )
    except (OSError, UnicodeError, subprocess.TimeoutExpired) as exc:
        raise ComparisonError(f"Rust inventory failed for {source.name}: {exc}") from exc


def variant_matches(actual: str, matrix_variant: str) -> bool:
    token = actual.lower().replace("_", "").replace("-", "")
    return (token == "c8" if matrix_variant == "C8" else token in ("hna", "hnb"))


def nonnegative(value: str, label: str) -> int:
    try:
        parsed = int(value, 10)
    except ValueError as exc:
        raise ComparisonError(f"Rust inventory {label} is not an integer: {value!r}") from exc
    if parsed < 0 or str(parsed) != value:
        raise ComparisonError(f"Rust inventory {label} is not canonical nonnegative decimal")
    return parsed


def parse_rows(output: str, row: dict) -> tuple[list[dict], list[dict]]:
    """Read the Rust example's bounded metadata protocol, without source bytes."""
    images: list[dict] = []
    errors: list[dict] = []
    lines = output.splitlines()
    if len(lines) > MAX_RECORDS_PER_SOURCE:
        raise ComparisonError(f"{row['id']}: Rust inventory exceeded record ceiling")
    for ordinal, line in enumerate(lines, 1):
        fields = line.split("\t")
        if len(fields) not in (7, 8) or fields[0] not in ("I", "E"):
            raise ComparisonError(f"{row['id']}: invalid Rust metadata row {ordinal}")
        if not variant_matches(fields[1], row["variant"]):
            raise ComparisonError(f"{row['id']}: Rust variant differs at row {ordinal}")
        page = nonnegative(fields[2], "page")
        if page == 0:
            raise ComparisonError(f"{row['id']}: zero page at row {ordinal}")
        if fields[0] == "I":
            if len(fields) != 8:
                raise ComparisonError(f"{row['id']}: malformed image row {ordinal}")
            image = nonnegative(fields[3], "image")
            descriptor = nonnegative(fields[4], "descriptor offset")
            kind = nonnegative(fields[5], "record type")
            offset = nonnegative(fields[6], "payload offset")
            length = nonnegative(fields[7], "payload length")
            if image == 0 or length == 0:
                raise ComparisonError(f"{row['id']}: zero image index or length at row {ordinal}")
            images.append({"source_id": row["id"], "page": page, "image": image,
                           "descriptor_offset": descriptor, "record_type": kind,
                           "offset": offset, "length": length})
        else:
            if len(fields) != 7:
                raise ComparisonError(f"{row['id']}: malformed error row {ordinal}")
            image = None if fields[3] == "-" else nonnegative(fields[3], "error image")
            offset = nonnegative(fields[4], "error offset")
            field, error_kind = fields[5:]
            if image == 0 or not field or not error_kind or any(
                char in field + error_kind for char in "\r\n\x00\t"
            ):
                raise ComparisonError(f"{row['id']}: malformed error fields at row {ordinal}")
            errors.append({"source_id": row["id"], "page": page, "image": image,
                           "offset": offset, "field": field, "error_kind": error_kind})
    return images, errors


def compare_records(images: list[dict], errors: list[dict], manifest: dict, report: dict) -> None:
    expected = {
        (sample["id"], image["page"], image["image"]): (image["offset"], image["length"])
        for sample in manifest["samples"] for image in sample["images"]
    }
    observed: dict[tuple[str, int, int], tuple[int, int]] = {}
    nonzero = {kind: 0 for kind in EXPECTED_NONZERO}
    seen: set[tuple[str, int, int]] = set()
    duplicate_type0 = 0
    duplicate_nonzero = 0
    unobserved_types = 0
    for image in images:
        key = (image["source_id"], image["page"], image["image"])
        if key in seen:
            add_failure(report, f"{key!r}: duplicate Rust image coordinate")
            if image["record_type"] == 0:
                duplicate_type0 += 1
            else:
                duplicate_nonzero += 1
            continue
        seen.add(key)
        kind = image["record_type"]
        if kind == 0:
            observed[key] = (image["offset"], image["length"])
        elif kind in nonzero:
            nonzero[kind] += 1
        else:
            unobserved_types += 1
            add_failure(report, f"{key!r}: unobserved image discriminator {kind}")
    matched = sum(observed.get(key) == value for key, value in expected.items())
    missing = expected.keys() - observed.keys()
    unexpected = observed.keys() - expected.keys()
    mismatch = {key for key in expected.keys() & observed.keys()
                if expected[key] != observed[key]}
    report["type0"].update(status="PASS", passed=matched, missing=len(missing),
                           failed=len(unexpected) + len(mismatch) + duplicate_type0,
                           not_run=0)
    for key in sorted(missing):
        add_failure(report, f"{key!r}: missing type-0 image")
    for key in sorted(unexpected):
        add_failure(report, f"{key!r}: unexpected type-0 image")
    for key in sorted(mismatch):
        add_failure(report, f"{key!r}: type-0 offset/length differs from #22 manifest")
    if missing or unexpected or mismatch or duplicate_type0:
        report["type0"]["status"] = "FAIL"
    report["non_type0"].update(status="PASS", counts={str(k): v for k, v in nonzero.items()})
    if nonzero != EXPECTED_NONZERO or duplicate_nonzero or unobserved_types:
        report["non_type0"]["status"] = "FAIL"
        if nonzero != EXPECTED_NONZERO:
            add_failure(report, f"non-type-0 discriminator counts differ: {nonzero!r}")

    actual_invalid = {
        (item["source_id"], item["page"], item["image"], item["offset"],
         item["field"], item["error_kind"])
        for item in errors
    }
    report["expected_invalid"].update(status="PASS", observed=len(errors), records=errors)
    if actual_invalid != EXPECTED_RUST_INVALID or len(errors) != len(EXPECTED_RUST_INVALID):
        report["expected_invalid"]["status"] = "FAIL"
        add_failure(report, "diagnostic invalid-record locations differ from the pinned issue-100 Rust profile")


def run(corpus: Path | None, binary: Path | None = None, report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    rows, manifest = baseline()
    if corpus is None:
        report["reason"] = "external CAJSamples corpus is unset"
        return report
    report["phase"] = "source_precheck"
    audit(rows, corpus)
    report["source_hashes_before"] = len(rows)
    record_baseline_invalid(report, manifest)
    root = corpus.resolve(strict=True)
    report["phase"] = "rust_build"
    selected_binary = rust_binary(binary)
    all_images: list[dict] = []
    all_errors: list[dict] = []
    report["phase"] = "rust_inventory"
    try:
        for row in rows:
            path = conformance.contained_file(root, conformance.relative_path(row["path"]))
            malformed = row["id"] == ISSUE_100
            if malformed:
                normal_code, _, normal_error = run_example(selected_binary, path)
                if normal_code == 0:
                    add_failure(report, f"{row['id']}: normal cursor accepted malformed input")
                elif "page 2, image 1" not in normal_error:
                    add_failure(report, f"{row['id']}: normal cursor did not stop at page 2 image 1")
            code, output, error = run_example(selected_binary, path, diagnostic=malformed)
            if code:
                add_failure(report, f"{row['id']}: Rust inventory exited {code}: {error[-512:]}")
                continue
            images, errors = parse_rows(output, row)
            all_images.extend(images)
            all_errors.extend(errors)
            if errors and not malformed:
                add_failure(report, f"{row['id']}: unexpected Rust diagnostic error rows")
        report["phase"] = "comparison"
        compare_records(all_images, all_errors, manifest, report)
    finally:
        report["phase"] = "source_postcheck"
        audit(rows, corpus)
        report["source_hashes_after"] = len(rows)
    if report["status"] != "FAIL":
        report["status"] = "PASS" if all(
            report[key]["status"] == "PASS" for key in (
                "type0", "non_type0", "baseline_invalid", "expected_invalid"
            )
        ) else "FAIL"
    report["phase"] = "complete"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=(
        Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None
    ))
    parser.add_argument("--rust-bin", type=Path, help="already built hnc8_container_inventory example")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, args.rust_bin, report)
    except (ComparisonError, conformance.ConformanceError, jbig1_oracle.OracleError,
            OSError, ValueError, KeyError) as exc:
        add_failure(report, str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        item = report["type0"]
        print(f"HN/C8 Rust container comparison [{report['status']}]: "
              f"type-0 PASS={item['passed']} FAIL={item['failed']} "
              f"missing={item['missing']} NOT_RUN={item['not_run']}; "
              f"source SHA before/after={report['source_hashes_before']}/"
              f"{report['source_hashes_after']}")
        for failure in report["failures"]:
            print(f"FAIL: {failure}", file=sys.stderr)
        if report.get("reason"):
            print(report["reason"], file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
