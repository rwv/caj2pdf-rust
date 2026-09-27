#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, SHA-pinned Rust full-page pixel comparison for observed HN/C8.

The private MQ table and external documents stay outside Git. Rust receives
only pinned source ranges and reports packed page pixels; the independent #43
pixel hashes and black counts are read and compared only by this script.
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

import jbig2_dictionary_headers as dictionaries
import jbig2_generic_oracle as generic_oracle
import jbig2_oracle as oracle
import jbig2_text_instance_diagnostic as instances


EXPECTED = instances.EXPECTED
STANDARD = instances.STANDARD
FULL_MANIFEST_SHA256 = "bda920020d111e200352e7a38d8ec54c0be6c2e0a6b1c87c6ac55aac861c9bbe"
STRICT_POLICY = "strict"
HN_C8_POLICY = "hn-c8-unused-refinement-template"
ANOMALY_MARKER = "HN_C8_UNUSED_REFINEMENT_TEMPLATE"
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
HEX = re.compile(r"[0-9a-f]{64}\Z")
TOKEN = re.compile(r"[A-Za-z][A-Za-z0-9_]*\Z")
MAX_REQUEST_BYTES = 1024 * 1024
MAX_SOURCE_BYTES = 512 * 1024 * 1024
MAX_RESIDENT_BYTES = 4 * 1024 * 1024
MAX_SCRATCH_BYTES = 256 * 1024 * 1024
MAX_RSS_KIB = 1024 * 1024
MAX_MANIFEST_BYTES = 4 * 1024 * 1024


class ParityError(instances.DiagnosticError):
    """A requested source, bounded decoder result, or pixel comparison failed."""


def initial_report(policy: str = STRICT_POLICY) -> dict:
    return {
        "status": "NOT_RUN", "phase": "preflight", "expected_images": EXPECTED,
        "text_header_policy": policy,
        "submitted_cases": 0,
        "source_hashes_before": {}, "source_hashes_after": {},
        "table_sha256_before": None, "table_sha256_after": None,
        "rust_binary_sha256_before": None, "rust_binary_sha256_after": None,
        "compatibility": {
            "status": "NOT_RUN", "attempted": 0, "completed": 0,
            "matching": 0, "failing": 0, "skipped": 0, "unsupported": 0,
            "first_failure": None, "cases": [],
        },
        "strict_header_refusals": 0,
        "opt_in_anomaly": {
            "status": "NOT_RUN", "attempted": 0, "completed": 0,
            "matching": 0, "failing": 0, "skipped": 0, "unsupported": 0,
            "marker": ANOMALY_MARKER, "raw_flags": "0xa40c",
            "first_failure": None, "case": None,
        },
        "resources": {
            "max_request_bytes": 0, "peak_resident_bytes": 0,
            "peak_scratch_bytes": 0, "peak_output_bytes": 0,
            "peak_rss_kib": 0, "large_page": None,
        },
    }


def load_manifest(path: Path = oracle.DEFAULT_MANIFEST) -> dict:
    """Pin the raw #43 baseline before even reporting a clean-clone NOT_RUN."""
    with path.open("rb") as source:
        encoded = source.read(MAX_MANIFEST_BYTES + 1)
    if len(encoded) > MAX_MANIFEST_BYTES:
        raise ParityError("#43 full-page manifest exceeds 4 MiB")
    if hashlib.sha256(encoded).hexdigest() != FULL_MANIFEST_SHA256:
        raise ParityError("#43 full-page manifest SHA-256 differs from pinned baseline")
    manifest = json.loads(encoded.decode("utf-8"))
    oracle.validate_manifest(manifest)
    return manifest


def _oracle_images(manifest: dict) -> dict[tuple[str, int, int], dict]:
    images = {
        (sample["id"], image["page"], image["image"]): image
        for sample in manifest["samples"] for image in sample["images"]
    }
    if len(images) != EXPECTED:
        raise ParityError("#43 full-page oracle lacks a unique 546-case baseline")
    return images


def plan_for_cases(cases: list[dict], recorded: dict, dictionary_manifest: dict,
                   full_manifest: dict) -> tuple[list[str], list[dict]]:
    """Join fresh source slices to #43, keeping expected pixels in Python."""
    text_manifest = instances.load_text_manifest(instances.text_oracle.DEFAULT_MANIFEST)
    base_lines, control = instances.plan_for_cases(
        cases, recorded, dictionary_manifest, text_manifest)
    baseline = _oracle_images(full_manifest)
    if len(cases) != EXPECTED or len(base_lines) != EXPECTED or len(control) != EXPECTED:
        raise ParityError("full-page plan is incomplete")
    lines = []
    selected = []
    for case, base, (key, kind, declared, flags_offset) in zip(cases, base_lines, control):
        expected = baseline.get(key)
        if expected is None or case["coordinate"] != key or kind not in (
                instances.text_oracle.STANDARD, instances.text_oracle.ANOMALY):
            raise ParityError("full-page coordinate or classification differs from #43")
        for field in ("offset", "length", "width", "height"):
            if case[field] != expected[field]:
                raise ParityError(f"{key}: {field} differs from #43")
        profile, record_sha = oracle.profile_and_hash(case)
        if profile != expected["profile"] or record_sha != expected["encoded_sha256"]:
            raise ParityError(f"{key}: encoded record or profile differs from #43")
        page, generic = generic_oracle.selected_spans(case)
        with case["source_path"].open("rb") as source:
            page_sha = oracle.sha256_span(source, page.offset, page.length)
            generic_sha = oracle.sha256_span(source, generic.offset, generic.length)
        suffix = (
            case["offset"], case["length"], record_sha,
            page.offset, page.length, page_sha,
            generic.offset, generic.length, generic_sha,
        )
        lines.append(base + "\t" + "\t".join(str(value) for value in suffix))
        selected.append({
            "coordinate": key, "classification": kind, "instances": declared,
            "flags_offset": flags_offset, "width": expected["width"],
            "height": expected["height"],
            "pixel_sha256": expected["normalized_pixel_sha256"],
            "black_pixels": expected["black_pixels"],
        })
    if sum(len(line.encode("utf-8")) + 1 for line in lines) > instances.MAX_PLAN_BYTES:
        raise ParityError("full-page plan exceeds 2 MiB")
    if (sum(item["classification"] == instances.text_oracle.STANDARD for item in selected) != STANDARD
            or sum(item["classification"] == instances.text_oracle.ANOMALY for item in selected) != 1):
        raise ParityError("full-page standard/anomaly split differs from pinned baseline")
    return lines, selected


def _number(value: str, line: int, name: str) -> int:
    if len(value) > 20 or not value.isascii() or not value.isdecimal():
        raise ParityError(f"Rust case line {line} has invalid {name}")
    return int(value)


def _failure(entry: dict, expected: dict, kind: str | None = None) -> dict:
    failure = {
        "coordinate": entry["coordinate"], "status": entry["status"],
        "kind": kind or entry["refusal_kind"],
        "source_byte_offset": entry["refusal_offset"], "stage": entry["stage"],
    }
    if kind == "pixel_mismatch":
        failure.update(
            expected_sha256=expected["pixel_sha256"], actual_sha256=entry["pixel_sha256"],
            expected_black=expected["black_pixels"], actual_black=entry["black_pixels"],
        )
    return failure


def parse_output(output: str, selected: list[dict],
                 policy: str = STRICT_POLICY) -> tuple[dict, int, dict, dict]:
    if policy not in (STRICT_POLICY, HN_C8_POLICY):
        raise ParityError("unknown text-header compatibility policy")
    lines = output.splitlines()
    if len(selected) != EXPECTED or len(lines) != EXPECTED + 1:
        raise ParityError(f"Rust emitted {len(lines)} lines, expected {EXPECTED + 1}")
    if lines[-1].split("\t") != ["TOTAL", str(EXPECTED)]:
        raise ParityError("Rust total does not acknowledge all 546 attempts")
    report = initial_report(policy)
    standard = report["compatibility"]
    anomaly = report["opt_in_anomaly"]
    resources = report["resources"]
    standard.update(status="FAIL", attempted=STANDARD)
    if policy == HN_C8_POLICY:
        anomaly.update(status="FAIL", attempted=1)
    strict_refusals = 0
    largest_area = -1
    for line_number, (line, expected) in enumerate(zip(lines[:-1], selected)):
        fields = line.split("\t")
        if len(fields) != 20 or fields[0] != "CASE":
            raise ParityError(f"Rust case line {line_number} has invalid fields")
        coordinate = (
            fields[1], _number(fields[2], line_number, "page"),
            _number(fields[3], line_number, "image"),
        )
        if coordinate != expected["coordinate"]:
            raise ParityError(f"Rust case line {line_number} differs from pinned order")
        numbers = {
            "width": 5, "height": 6, "rows": 7, "output_bytes": 8,
            "black_pixels": 10, "source_bytes": 11, "max_request_bytes": 12,
            "peak_resident_bytes": 13, "scratch_bytes": 14, "rss_kib": 15,
            "refusal_offset": 17,
        }
        entry = {name: _number(fields[index], line_number, name)
                 for name, index in numbers.items()}
        entry.update(
            coordinate=coordinate, status=fields[4], pixel_sha256=fields[9],
            refusal_kind=fields[16], stage=fields[18], anomaly_marker=fields[19],
        )
        width, height = expected["width"], expected["height"]
        packed = ((width + 7) // 8) * height
        early_refusal = (entry["status"] == "REFUSED"
                         and entry["stage"] in ("Source", "Directory", "PageInfo")
                         and entry["width"] == entry["height"] == 0)
        if (not early_refusal and (entry["width"], entry["height"]) != (width, height)
                or entry["rows"] > height or entry["output_bytes"] > packed
                or entry["black_pixels"] > width * height
                or entry["max_request_bytes"] > MAX_REQUEST_BYTES
                or entry["source_bytes"] > MAX_SOURCE_BYTES
                or entry["peak_resident_bytes"] > MAX_RESIDENT_BYTES
                or entry["scratch_bytes"] > MAX_SCRATCH_BYTES
                or entry["rss_kib"] > MAX_RSS_KIB
                or HEX.fullmatch(entry["pixel_sha256"]) is None
                or TOKEN.fullmatch(entry["stage"]) is None):
            raise ParityError(f"Rust case line {line_number} has impossible page metrics")
        if expected["classification"] == instances.text_oracle.STANDARD:
            if entry["anomaly_marker"] != "-":
                raise ParityError(f"standard case {line_number} has an anomaly marker")
            standard["cases"].append(entry)
            if entry["status"] == "COMPLETE":
                if (entry["rows"] != height or entry["output_bytes"] != packed
                        or entry["scratch_bytes"] < packed
                        or min(entry["source_bytes"], entry["max_request_bytes"],
                               entry["peak_resident_bytes"], entry["rss_kib"]) == 0
                        or entry["refusal_kind"] != "-" or entry["stage"] != "Complete"):
                    raise ParityError(f"Rust case {line_number} falsely reports completion")
                standard["completed"] += 1
                if (entry["pixel_sha256"] == expected["pixel_sha256"]
                        and entry["black_pixels"] == expected["black_pixels"]):
                    standard["matching"] += 1
                    entry["pixel_match"] = True
                else:
                    entry["pixel_match"] = False
                    if standard["first_failure"] is None:
                        standard["first_failure"] = _failure(entry, expected, "pixel_mismatch")
            elif entry["status"] == "REFUSED":
                if (TOKEN.fullmatch(entry["refusal_kind"]) is None
                        or entry["pixel_sha256"] != EMPTY_SHA256):
                    raise ParityError(f"Rust case {line_number} lacks a typed refusal")
                if "unsupported" in entry["refusal_kind"]:
                    standard["unsupported"] += 1
                if standard["first_failure"] is None:
                    standard["first_failure"] = _failure(entry, expected)
            else:
                raise ParityError(f"standard case {line_number} has invalid status")
            area = width * height
            if area > largest_area:
                largest_area = area
                resources["large_page"] = {
                    "coordinate": coordinate, "width": width, "height": height,
                    "status": entry["status"], "pixel_match": entry.get("pixel_match", False),
                    "packed_bytes": packed, "scratch_bytes": entry["scratch_bytes"],
                    "output_bytes": entry["output_bytes"],
                    "peak_resident_bytes": entry["peak_resident_bytes"],
                    "process_rss_high_water_kib": entry["rss_kib"],
                }
        else:
            anomaly["case"] = entry
            if policy == STRICT_POLICY:
                if (entry["status"] == "HEADER_REFUSED"
                        and entry["rows"] == entry["output_bytes"] ==
                        entry["black_pixels"] == entry["scratch_bytes"] == 0
                        and entry["pixel_sha256"] == EMPTY_SHA256
                        and entry["refusal_kind"] == "malformed_text_header"
                        and entry["refusal_offset"] == expected["flags_offset"]
                        and entry["stage"] == "Header"
                        and entry["anomaly_marker"] == "-"):
                    strict_refusals += 1
                else:
                    anomaly["first_failure"] = _failure(entry, expected, "strict_header_not_refused")
            elif entry["status"] == "COMPLETE":
                if (entry["rows"] != height or entry["output_bytes"] != packed
                        or entry["scratch_bytes"] < packed
                        or min(entry["source_bytes"], entry["max_request_bytes"],
                               entry["peak_resident_bytes"], entry["rss_kib"]) == 0
                        or entry["refusal_kind"] != "-" or entry["stage"] != "Complete"
                        or entry["anomaly_marker"] != ANOMALY_MARKER):
                    raise ParityError("opt-in anomaly falsely reports completion")
                anomaly["completed"] = 1
                if (entry["pixel_sha256"] == expected["pixel_sha256"]
                        and entry["black_pixels"] == expected["black_pixels"]):
                    anomaly.update(status="PASS", matching=1)
                    entry["pixel_match"] = True
                else:
                    anomaly["first_failure"] = _failure(entry, expected, "pixel_mismatch")
                    entry["pixel_match"] = False
            elif entry["status"] == "REFUSED":
                if (TOKEN.fullmatch(entry["refusal_kind"]) is None
                        or entry["pixel_sha256"] != EMPTY_SHA256):
                    raise ParityError("opt-in anomaly lacks a typed refusal")
                if "unsupported" in entry["refusal_kind"]:
                    anomaly["unsupported"] = 1
                anomaly["first_failure"] = _failure(entry, expected)
            else:
                raise ParityError("opt-in anomaly has invalid status")
        for field in ("max_request_bytes", "peak_resident_bytes"):
            resources[field] = max(resources[field], entry[field])
        resources["peak_rss_kib"] = max(resources["peak_rss_kib"], entry["rss_kib"])
        resources["peak_scratch_bytes"] = max(resources["peak_scratch_bytes"], entry["scratch_bytes"])
        resources["peak_output_bytes"] = max(resources["peak_output_bytes"], entry["output_bytes"])
    standard["failing"] = standard["attempted"] - standard["matching"]
    if standard["matching"] == STANDARD and standard["unsupported"] == 0:
        standard["status"] = "PASS"
    if policy == HN_C8_POLICY:
        anomaly["failing"] = anomaly["attempted"] - anomaly["matching"]
    return standard, strict_refusals, anomaly, resources


def build_binary(override: Path | None) -> Path:
    if override is not None:
        return override.resolve(strict=True)
    result = instances.parity.bounded_run([
        "cargo", "build", "--locked", "--release", "-p", "caj2pdf-core",
        "--example", "jbig2_page_parity",
    ], "Rust full-page parity build", 300)
    if result.returncode:
        raise ParityError(f"Rust full-page parity build failed: {result.stderr[-4096:]}")
    return instances.ROOT / "target/release/examples/jbig2_page_parity"


def run_binary(binary: Path, fixture: Path, lines: list[str], selected: list[dict],
               policy: str) -> tuple[dict, int, dict, dict]:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-page-parity-", dir="/tmp") as temp:
        plan = Path(temp) / "plan.tsv"
        plan.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if plan.stat().st_size > instances.MAX_PLAN_BYTES:
            raise ParityError("written full-page plan exceeds 2 MiB")
        command = [str(binary), str(fixture), str(plan)]
        if policy == HN_C8_POLICY:
            command.append(policy)
        return parse_output(instances.bounded_decode(command), selected, policy)


def run(corpus: Path | None, fixture: Path | None, binary: Path | None = None,
        manifest_path: Path = oracle.DEFAULT_MANIFEST,
        report: dict | None = None, policy: str = STRICT_POLICY) -> dict:
    if policy not in (STRICT_POLICY, HN_C8_POLICY):
        raise ParityError("unknown text-header compatibility policy")
    report = initial_report(policy) if report is None else report
    report["text_header_policy"] = policy
    full_manifest = load_manifest(manifest_path)
    if corpus is None and fixture is None:
        report["reason"] = "external corpus and private T.88 state table are unset"
        return report
    if corpus is None or fixture is None:
        raise ParityError("corpus and private T.88 state table must be supplied together")
    fixture = instances.parity.check_table(fixture)
    report["table_sha256_before"] = oracle.sha256_file(fixture)
    if report["table_sha256_before"] != instances.parity.TABLE_SHA:
        raise ParityError("private T.88 state table changed during precheck")
    report["phase"] = "source_precheck"
    rows, paths = oracle.validate_sources(oracle.DEFAULT_MATRIX, corpus)
    report["source_hashes_before"] = {row["id"]: row["sha256"] for row in rows}
    root = corpus.resolve(strict=True)
    selected_binary = None
    try:
        report["phase"] = "inventory"
        cases = oracle.checked_cases(oracle.run_directory_inventory(root), rows, paths)
        _, recorded = instances.headers.load_baseline()
        _, dictionary_manifest = dictionaries.load_baseline()
        lines, selected = plan_for_cases(cases, recorded, dictionary_manifest, full_manifest)
        report["phase"] = "rust_build"
        selected_binary = build_binary(binary)
        report["rust_binary_sha256_before"] = oracle.sha256_file(selected_binary)
        report["phase"] = "rust_decode"
        report["submitted_cases"] = len(lines)
        compatibility, strict, anomaly, resources = run_binary(
            selected_binary, fixture, lines, selected, policy)
        report.update(compatibility=compatibility, strict_header_refusals=strict,
                      opt_in_anomaly=anomaly, resources=resources)
    finally:
        active_phase = report["phase"]
        report["phase"] = "source_postcheck"
        failures = []
        try:
            after_rows, _ = oracle.validate_sources(oracle.DEFAULT_MATRIX, root)
            report["source_hashes_after"] = {row["id"]: row["sha256"] for row in after_rows}
        except (oracle.OracleError, OSError) as exc:
            failures.append(f"source postcheck: {exc}")
        try:
            instances.parity.check_table(fixture)
            report["table_sha256_after"] = oracle.sha256_file(fixture)
            if report["table_sha256_after"] != report["table_sha256_before"]:
                failures.append("private T.88 state table changed during the run")
        except (instances.parity.ParityError, OSError) as exc:
            failures.append(f"table postcheck: {exc}")
        if selected_binary is not None:
            try:
                report["rust_binary_sha256_after"] = oracle.sha256_file(selected_binary)
                if report["rust_binary_sha256_after"] != report["rust_binary_sha256_before"]:
                    failures.append("Rust diagnostic binary changed during the run")
            except OSError as exc:
                failures.append(f"Rust binary postcheck: {exc}")
        if failures:
            report["status"] = "FAIL"
            report["compatibility"]["status"] = "FAIL"
            if policy == HN_C8_POLICY:
                report["opt_in_anomaly"]["status"] = "FAIL"
            raise ParityError("; ".join(failures))
        report["phase"] = active_phase
    if (report["compatibility"]["status"] == "PASS"
            and ((policy == STRICT_POLICY and report["strict_header_refusals"] == 1)
                 or (policy == HN_C8_POLICY and report["opt_in_anomaly"]["status"] == "PASS"))):
        report.update(status="PASS", phase="complete")
    else:
        report.update(status="FAIL", phase="complete",
                      error="observed full-page packed pixels did not match #43")
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
    parser.add_argument("--text-header-policy", choices=(STRICT_POLICY, HN_C8_POLICY),
                        default=STRICT_POLICY)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report(args.text_header_policy)
    try:
        report = run(args.corpus_dir, args.table_fixture, args.rust_bin, args.manifest,
                     report, args.text_header_policy)
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError,
            subprocess.TimeoutExpired, oracle.OracleError,
            dictionaries.InventoryError, instances.parity.ParityError,
            instances.DiagnosticError) as exc:
        report.update(status="FAIL", error=str(exc))
        report["compatibility"]["status"] = "FAIL"
        if args.text_header_policy == HN_C8_POLICY:
            report["opt_in_anomaly"]["status"] = "FAIL"
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
