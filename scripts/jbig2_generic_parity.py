#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional full-corpus Rust parity for generic-only T.88 template-2 regions.

Re-runs the #51 black-box oracle, joins its strict hash-only manifest to a
fresh #42 directory inventory, then streams each original #4 region through
the #49 Rust decoder. Exact T.88 probability states stay in a private /tmp
fixture. A clean clone reports NOT_RUN with zero compatibility matches.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Any

import jbig2_generic_oracle as generic
import jbig2_oracle as full

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "tests/conformance/jbig2_generic_oracle.json"
EXPECTED = 546
TABLE_SHA = "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57"
MAX_TABLE_BYTES = 16 * 1024
MAX_PLAN_BYTES = 2 * 1024 * 1024
MAX_RUN_OUTPUT_BYTES = 2 * 1024 * 1024


class ParityError(Exception):
    """A source, baseline, decoder, or orchestration failure."""


def check_table(path: Path) -> Path:
    try:
        canonical = path.resolve(strict=True)
        if not canonical.is_relative_to(Path("/tmp")):
            raise ParityError("private T.88 table fixture must stay under /tmp")
        if canonical.stat().st_size > MAX_TABLE_BYTES:
            raise ParityError("private T.88 table fixture exceeds 16 KiB")
        if full.sha256_file(canonical) != TABLE_SHA:
            raise ParityError("private T.88 table fixture SHA-256 differs")
    except OSError as exc:
        raise ParityError(f"private T.88 table fixture is unavailable: {exc}") from exc
    return canonical


def field(value: Any, label: str) -> str:
    text = str(value)
    if not text or any(char in text for char in "\t\r\n\x00"):
        raise ParityError(f"{label} cannot be represented in bounded plan")
    return text


def plan_for_cases(cases: list[dict], manifest: dict) -> tuple[list[str], list[dict]]:
    """Join the fresh Rust inventory to the strict #51 baseline by coordinate."""
    generic.validate_manifest(manifest)
    if len(cases) != EXPECTED:
        raise ParityError(f"fresh inventory has {len(cases)} images, expected {EXPECTED}")
    baseline = {}
    for sample in manifest["samples"]:
        for image in sample["images"]:
            key = (sample["id"], image["page"], image["image"])
            if key in baseline:
                raise ParityError(f"duplicate baseline coordinate {key!r}")
            baseline[key] = (sample, image)
    if len(baseline) != EXPECTED:
        raise ParityError("generic-only baseline lacks 546 unique images")
    lines, selected = [], []
    seen = set()
    for case in cases:
        key = case["coordinate"]
        label = f"{key[0]} page {key[1]} image {key[2]}"
        if key in seen or key not in baseline:
            raise ParityError(f"{label}: duplicate or unpinned fresh image")
        seen.add(key)
        sample, image = baseline[key]
        if sample["source_sha256"] != case["source_sha256"]:
            raise ParityError(f"{label}: source SHA differs from baseline")
        for name in ("offset", "length", "width", "height"):
            if image[name] != case[name]:
                raise ParityError(f"{label}: {name} differs from generic-only baseline")
        spans = generic.selected_spans(case)
        profile = generic.checked_profile(case)
        if profile != image["generic_profile"]:
            raise ParityError(f"{label}: arithmetic/template/AT profile differs")
        _, hashes = generic.prehash_selected(case, spans)
        for span in spans:
            expected = image[f"segment_{span.number}"]
            if (span.offset, span.length, hashes[span.number]) != (
                expected["offset"], expected["length"], expected["encoded_sha256"]
            ):
                raise ParityError(
                    f"{label}: segment {span.number} offset {span.offset} length {span.length} "
                    "or encoded SHA differs from baseline"
                )
        path = Path(case["source_path"])
        columns = [
            key[0], key[1], key[2], str(path),
            spans[0].offset, spans[0].length, hashes[0],
            spans[1].offset, spans[1].length, hashes[4],
            case["width"], case["height"], image["normalized_pixel_sha256"],
            image["black_pixels"],
        ]
        lines.append("\t".join(field(value, label) for value in columns))
        selected.append({
            "key": key, "width": case["width"], "height": case["height"],
            "pixel_sha": image["normalized_pixel_sha256"],
            "black": image["black_pixels"],
            "generic_offset": spans[1].offset, "generic_length": spans[1].length,
        })
    if seen != set(baseline):
        raise ParityError("fresh inventory omits pinned generic-only images")
    if sum(len(line.encode("utf-8")) + 1 for line in lines) > MAX_PLAN_BYTES:
        raise ParityError("bounded Rust parity metadata plan exceeds 2 MiB")
    return lines, selected


def parse_rust_output(output: str, selected: list[dict]) -> dict:
    lines = output.splitlines()
    if len(lines) != len(selected) + 1 or len(selected) != EXPECTED:
        raise ParityError(f"Rust parity emitted {len(lines)} lines, expected {EXPECTED + 1}")
    cases = []
    for index, (line, expected) in enumerate(zip(lines[:-1], selected)):
        parts = line.split("\t")
        if len(parts) != 17 or parts[0] != "CASE":
            raise ParityError(f"Rust parity line {index} has invalid fields")
        key = (parts[1], int(parts[2]), int(parts[3]))
        if key != expected["key"]:
            raise ParityError(f"Rust parity line {index} coordinate differs: {key!r}")
        width, height = int(parts[4]), int(parts[5])
        digest, black = parts[6], int(parts[7])
        rows, stride, pixels = map(int, parts[8:11])
        calls, read_bytes, max_read, max_write, mq_fetched, elapsed_ms = map(int, parts[11:17])
        if (width, height, digest, black) != (
            expected["width"], expected["height"], expected["pixel_sha"], expected["black"]
        ):
            raise ParityError(f"{key!r}: Rust dimensions, pixel SHA, or black count differs")
        if rows != height or stride != (width + 7) // 8 or pixels != width * height:
            raise ParityError(f"{key!r}: Rust row or symbol count differs")
        if calls <= 0 or read_bytes < mq_fetched or max_read > 64 * 1024 or max_write != stride or elapsed_ms < 0:
            raise ParityError(f"{key!r}: Rust I/O metrics violate row-stream bounds")
        cases.append({
            "key": key, "pixels": pixels, "row_stride": stride,
            "source_read_calls": calls, "source_bytes_read": read_bytes,
            "max_source_request": max_read, "max_output_write": max_write,
            "mq_bytes_fetched": mq_fetched, "elapsed_ms": elapsed_ms,
            "generic_offset": expected["generic_offset"],
            "generic_length": expected["generic_length"],
        })
    total = lines[-1].split("\t")
    if len(total) != 4 or total[0] != "TOTAL" or int(total[1]) != EXPECTED:
        raise ParityError("Rust parity total count differs")
    elapsed_ms, peak_rss_kib = int(total[2]), int(total[3])
    if elapsed_ms < 0 or peak_rss_kib < 0:
        raise ParityError("Rust parity total metrics are invalid")
    small = min(cases, key=lambda case: case["pixels"])
    large = max(cases, key=lambda case: case["pixels"])
    encoded = max(cases, key=lambda case: case["generic_length"])
    return {
        "rust_matches": len(cases), "rust_failures": 0, "rust_skips": 0,
        "total_pixels": sum(case["pixels"] for case in cases),
        "native_elapsed_seconds": round(elapsed_ms / 1000, 3),
        "native_peak_rss_kib": peak_rss_kib,
        "source_read_calls": sum(case["source_read_calls"] for case in cases),
        "source_bytes_read": sum(case["source_bytes_read"] for case in cases),
        "max_source_request": max(case["max_source_request"] for case in cases),
        "max_output_write": max(case["max_output_write"] for case in cases),
        "small_case": small, "large_case": large, "largest_encoded_case": encoded,
    }


def rust_binary(override: Path | None) -> Path:
    if override is not None:
        return override.resolve(strict=True)
    command = ["cargo", "build", "--locked", "--release", "-p", "caj2pdf-core", "--example", "jbig2_generic_parity"]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300, check=False)
    if result.returncode:
        raise ParityError(f"Rust generic parity example build failed: {result.stderr[-4096:]}")
    return ROOT / "target/release/examples/jbig2_generic_parity"


def run_rust(lines: list[str], selected: list[dict], fixture: Path, binary: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-generic-parity-", dir="/tmp") as directory:
        plan = Path(directory) / "plan.tsv"
        plan.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if plan.stat().st_size > MAX_PLAN_BYTES:
            raise ParityError("bounded Rust parity metadata plan exceeds 2 MiB")
        completed = subprocess.run(
            [str(binary), str(fixture), str(plan)], cwd=ROOT,
            capture_output=True, text=True, timeout=3600, check=False,
        )
        if len(completed.stdout.encode("utf-8")) > MAX_RUN_OUTPUT_BYTES or len(completed.stderr.encode("utf-8")) > MAX_RUN_OUTPUT_BYTES:
            raise ParityError("Rust parity output exceeds 2 MiB")
        if completed.returncode:
            passed = sum(line.startswith("CASE\t") for line in completed.stdout.splitlines())
            raise ParityError(f"Rust parity stopped after {passed}/{EXPECTED}: {completed.stderr[-4096:]}")
        return parse_rust_output(completed.stdout, selected)


def oracle_run(corpus: Path, tools: dict[str, Path]) -> dict:
    command = [
        sys.executable, str(ROOT / "scripts/jbig2_generic_oracle.py"),
        "--corpus-dir", str(corpus), "--manifest", str(MANIFEST), "--json",
        "--qpdf", str(tools["qpdf"]), "--pdfimages", str(tools["pdfimages"]),
        "--mutool", str(tools["mutool"]),
    ]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=3600, check=False)
    if len(result.stdout.encode("utf-8")) > MAX_RUN_OUTPUT_BYTES or len(result.stderr.encode("utf-8")) > MAX_RUN_OUTPUT_BYTES:
        raise ParityError("generic-only black-box oracle output exceeds 2 MiB")
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise ParityError(f"generic-only black-box oracle emitted invalid JSON: {exc}") from exc
    if result.returncode or report.get("status") != "PASS" or report.get("tool_agreements") != EXPECTED or report.get("failures"):
        raise ParityError(f"generic-only black-box oracle failed: {json.dumps(report, ensure_ascii=False)[:4096]}")
    return report


def run(corpus: Path | None, fixture: Path | None, binary: Path | None, tools: dict[str, str]) -> dict:
    report = {"status": "NOT_RUN", "expected_images": EXPECTED, "rust_matches": 0,
              "rust_failures": 0, "rust_skips": 0, "tool_agreements": 0,
              "source_hashes_checked_before": 0, "source_hashes_checked_after": 0}
    generic.validate_manifest(json.loads(MANIFEST.read_text(encoding="utf-8")))
    if corpus is None:
        return {**report, "reason": "external CAJSamples corpus is unset"}
    if fixture is None:
        return {**report, "reason": "private T.88 state table fixture is unset"}
    if not corpus.is_dir():
        raise ParityError("supplied CAJSamples corpus directory is unavailable")
    fixture = check_table(fixture)
    rows, paths = full.validate_sources(full.DEFAULT_MATRIX, corpus)
    if len(rows) != 27:
        raise ParityError(f"source matrix has {len(rows)} entries, expected 27")
    report["source_hashes_checked_before"] = len(rows)
    resolved_tools = {name: full.tool_path(tools[name]) for name in ("qpdf", "pdfimages", "mutool")}
    missing = [name for name, path in resolved_tools.items() if path is None]
    if missing:
        return {**report, "reason": f"external oracle tool unavailable: {', '.join(missing)}"}
    oracle = oracle_run(corpus, resolved_tools)
    report["tool_agreements"] = oracle["tool_agreements"]
    report["toolchain_drift"] = oracle.get("toolchain_drift", False)
    if oracle.get("source_hashes_checked") != len(rows):
        raise ParityError("generic-only oracle did not hash all 27 sources")
    inventory = full.run_directory_inventory(corpus)
    cases = full.checked_cases(inventory, rows, paths)
    baseline = json.loads(MANIFEST.read_text(encoding="utf-8"))
    lines, selected = plan_for_cases(cases, baseline)
    selected_binary = rust_binary(binary)
    rust_error = None
    metrics = None
    try:
        metrics = run_rust(lines, selected, fixture, selected_binary)
    except (OSError, ParityError, subprocess.TimeoutExpired) as exc:
        rust_error = str(exc)
    try:
        after, _ = full.validate_sources(full.DEFAULT_MATRIX, corpus)
        if len(after) != 27:
            raise ParityError(f"postcheck source matrix has {len(after)} entries, expected 27")
        report["source_hashes_checked_after"] = len(after)
    except (OSError, full.OracleError) as exc:
        raise ParityError(f"source SHA-256 postcheck failed after Rust run: {exc}") from exc
    if rust_error:
        raise ParityError(rust_error)
    if metrics is None or metrics["rust_matches"] != EXPECTED:
        raise ParityError("Rust generic parity lacks 546 matches")
    report.update(status="PASS", rust_matches=EXPECTED, **{key: value for key, value in metrics.items() if key != "rust_matches"})
    report["oracle_toolchain_drift"] = report.pop("toolchain_drift")
    report["oracle_max_temp_record_bytes"] = oracle["max_temp_record_bytes"]
    report["oracle_max_temp_pdf_bytes"] = oracle["max_temp_pdf_bytes"]
    report["oracle_max_single_pbm_raster_bytes"] = oracle["max_single_pbm_raster_bytes"]
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--table-fixture", type=Path, default=Path(os.environ["CAJ2PDF_T88_H2_FIXTURE_FILE"]) if os.environ.get("CAJ2PDF_T88_H2_FIXTURE_FILE") else None)
    parser.add_argument("--rust-bin", type=Path)
    parser.add_argument("--qpdf", default="qpdf")
    parser.add_argument("--pdfimages", default="pdfimages")
    parser.add_argument("--mutool", default="mutool")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = run(args.corpus_dir, args.table_fixture, args.rust_bin,
                     {name: getattr(args, name) for name in ("qpdf", "pdfimages", "mutool")})
    except (OSError, ValueError, KeyError, full.OracleError, generic.OracleError, ParityError,
            json.JSONDecodeError, subprocess.TimeoutExpired) as exc:
        report = {"status": "FAIL", "expected_images": EXPECTED, "rust_matches": 0,
                  "rust_failures": 1, "rust_skips": 0, "tool_agreements": 0,
                  "error": str(exc)}
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(f"JBIG2 generic Rust parity [{report['status']}]: Rust={report['rust_matches']}/{EXPECTED}, "
              f"black-box tools={report['tool_agreements']}/{EXPECTED}")
        if report.get("error") or report.get("reason"):
            print(report.get("error") or report.get("reason"), file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
