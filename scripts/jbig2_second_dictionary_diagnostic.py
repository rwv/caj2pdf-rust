#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, SHA-pinned #2 dictionary decision-prefix diagnostic.

This is a decoder trace, not an independent symbol-pixel oracle. A decoded
prefix counts only after IAAI and its selected branch complete. Every case
is attempted from a fresh #1 store and #2 MQ coding unit. No external bytes
or probability states are committed to the repository.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import jbig2_dictionary_headers as headers
import jbig2_generic_parity as parity
import jbig2_oracle as full


ROOT = Path(__file__).resolve().parent.parent
EXPECTED = 546
MAX_PLAN_BYTES = 2 * 1024 * 1024
MAX_OUTPUT_BYTES = 2 * 1024 * 1024
TIMEOUT_SECONDS = 3600


class DiagnosticError(Exception):
    """Requested diagnostic input, execution, or output is invalid."""


def initial_report() -> dict:
    return {
        "status": "NOT_RUN",
        "phase": "preflight",
        "expected_images": EXPECTED,
        "source_hashes_before": 0,
        "source_hashes_after": 0,
        "diagnostic": {
            "status": "NOT_RUN", "attempted_cases": 0, "completed_cases": 0,
            "refused_cases": 0, "iaai_one": 0, "iaai_zero": 0,
            "iaai_multiple": 0, "first_typed_refusal": None,
        },
        "symbol_compatibility": {
            "status": "NOT_RUN", "checked_cases": 0, "passed": 0,
            "reason": "no independent per-symbol pixel oracle is configured",
        },
    }


def _plan_field(value: object, label: str) -> str:
    result = str(value)
    if not result or any(char in result for char in "\t\r\n\x00"):
        raise DiagnosticError(f"{label} cannot be represented in the bounded plan")
    return result


def plan_for_cases(cases: list[dict], manifest: dict) -> tuple[list[str], list[tuple[str, int, int]]]:
    if len(cases) != EXPECTED:
        raise DiagnosticError(f"fresh directory has {len(cases)} images, expected {EXPECTED}")
    pinned = {
        (sample["id"], image["page"], image["image"]): image
        for sample in manifest["samples"] for image in sample["images"]
    }
    if len(pinned) != EXPECTED:
        raise DiagnosticError("pinned dictionary coordinates are incomplete")
    lines: list[str] = []
    selected: list[tuple[str, int, int]] = []
    for case in cases:
        key = case["coordinate"]
        if key not in pinned or key in selected:
            raise DiagnosticError(f"duplicate or unpinned dictionary coordinate {key!r}")
        image = pinned[key]
        columns: list[object] = [*key, str(case["source_path"])]
        for number in (1, 2):
            segment = case["segments"][number]
            dictionary = image["dictionaries"][number - 1]
            if (segment["number"], segment["type"], segment["page_association"],
                    segment["refs"], segment["data_offset"], segment["data_length"]) != (
                        number, 0, 1, dictionary["refs"],
                        dictionary["data_offset"], dictionary["data_length"]):
                raise DiagnosticError(f"{key!r}: dictionary {number} segment drift")
            header_offset = segment["data_offset"] - segment["header_length"]
            if header_offset < case["offset"] + 48:
                raise DiagnosticError(f"{key!r}: dictionary {number} framing escapes image")
            columns.extend((header_offset, segment["header_length"] + segment["data_length"],
                            dictionary["data_sha256"], dictionary["new"],
                            dictionary["exported"]))
        lines.append("\t".join(_plan_field(value, str(key)) for value in columns))
        selected.append(key)
    if len(set(selected)) != EXPECTED:
        raise DiagnosticError("fresh directory omits a pinned dictionary coordinate")
    if sum(len(line.encode("utf-8")) + 1 for line in lines) > MAX_PLAN_BYTES:
        raise DiagnosticError("bounded dictionary plan exceeds 2 MiB")
    return lines, selected


def parse_output(output: str, selected: list[tuple[str, int, int]]) -> dict:
    """Parse only complete per-case lines; never infer decisions from flags."""
    lines = output.splitlines()
    if len(selected) != EXPECTED or len(lines) != EXPECTED + 1:
        raise DiagnosticError(f"Rust emitted {len(lines)} lines, expected {EXPECTED + 1}")
    totals = {"iaai_one": 0, "iaai_zero": 0, "iaai_multiple": 0}
    first_refusal = None
    completed = refused = 0
    for index, (line, key) in enumerate(zip(lines[:-1], selected)):
        parts = line.split("\t")
        if len(parts) != 10 or parts[0] != "CASE":
            raise DiagnosticError(f"Rust case line {index} has invalid fields")
        try:
            actual = (parts[1], int(parts[2]), int(parts[3]))
            counts = dict(zip(totals, map(int, parts[5:8])))
            byte_offset = int(parts[9])
        except ValueError as exc:
            raise DiagnosticError(f"Rust case line {index} has invalid numbers") from exc
        status, refusal = parts[4], parts[8]
        if (actual != key or status not in ("COMPLETE", "REFUSED")
                or any(value < 0 for value in counts.values()) or byte_offset < 0):
            raise DiagnosticError(f"Rust case line {index} differs from pinned plan")
        if status == "COMPLETE":
            if refusal != "-" or counts["iaai_zero"] or counts["iaai_multiple"]:
                raise DiagnosticError(f"Rust complete case {index} has a refusal branch")
            completed += 1
        else:
            if refusal not in ("iaai_zero", "iaai_multiple"):
                raise DiagnosticError(f"Rust case {index} has an unexpected refusal {refusal!r}")
            other = "iaai_multiple" if refusal == "iaai_zero" else "iaai_zero"
            if counts[refusal] != 1 or counts[other]:
                raise DiagnosticError(f"Rust case {index} refusal lacks a complete IAAI value")
            refused += 1
            if first_refusal is None:
                first_refusal = {"coordinate": key, "kind": refusal,
                                 "source_byte_offset": byte_offset}
        for name, value in counts.items():
            totals[name] += value
    tail = lines[-1].split("\t")
    if len(tail) != 2 or tail != ["TOTAL", str(EXPECTED)]:
        raise DiagnosticError("Rust total does not acknowledge all 546 attempts")
    return {"status": "PASS", "attempted_cases": EXPECTED,
            "completed_cases": completed, "refused_cases": refused,
            **totals, "first_typed_refusal": first_refusal}


def _build_binary(override: Path | None) -> Path:
    if override is not None:
        return override.resolve(strict=True)
    command = ["cargo", "build", "--locked", "--release", "-p", "caj2pdf-core",
               "--example", "jbig2_second_dictionary_diagnostic"]
    result = parity.bounded_run(command, "Rust #2 dictionary diagnostic build", 300)
    if result.returncode:
        raise DiagnosticError(f"Rust diagnostic build failed: {result.stderr[-4096:]}")
    return ROOT / "target/release/examples/jbig2_second_dictionary_diagnostic"


def _run_binary(binary: Path, fixture: Path, lines: list[str], selected: list) -> dict:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-dictionary-diagnostic-", dir="/tmp") as temp:
        plan = Path(temp) / "plan.tsv"
        plan.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if plan.stat().st_size > MAX_PLAN_BYTES:
            raise DiagnosticError("dictionary plan exceeds 2 MiB after writing")
        result = parity.bounded_run([str(binary), str(fixture), str(plan)],
                                    "Rust #2 dictionary diagnostic", TIMEOUT_SECONDS)
        if (len(result.stdout.encode("utf-8")) > MAX_OUTPUT_BYTES
                or len(result.stderr.encode("utf-8")) > MAX_OUTPUT_BYTES):
            raise DiagnosticError("Rust diagnostic output exceeds 2 MiB")
        if result.returncode:
            raise DiagnosticError(f"Rust diagnostic failed: {result.stderr[-4096:]}")
        return parse_output(result.stdout, selected)


def run(corpus: Path | None, fixture: Path | None,
        binary: Path | None = None, report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    rows, manifest = headers.load_baseline()
    if corpus is None and fixture is None:
        report["reason"] = "external corpus and private T.88 state table are unset"
        return report
    if corpus is None or fixture is None:
        raise DiagnosticError("corpus and private T.88 state table must be supplied together")
    fixture = parity.check_table(fixture)
    report["phase"] = "source_precheck"
    root = headers.audit_sources(rows, corpus)
    report["source_hashes_before"] = len(rows)
    try:
        report["phase"] = "inventory"
        metadata = headers.run(root)
        if metadata["status"] != "PASS" or metadata["metadata"]["checked_images"] != EXPECTED:
            raise DiagnosticError("SHA-pinned dictionary metadata inventory did not pass")
        paths = {row["id"]: headers.conformance.contained_file(
            root, headers.conformance.relative_path(row["path"])) for row in rows}
        cases = full.checked_cases(full.run_directory_inventory(root), rows, paths)
        lines, selected = plan_for_cases(cases, manifest)
        report["phase"] = "rust_build"
        selected_binary = _build_binary(binary)
        report["phase"] = "rust_decode"
        report["diagnostic"] = _run_binary(selected_binary, fixture, lines, selected)
    finally:
        report["phase"] = "source_postcheck"
        headers.audit_sources(rows, root)
        parity.check_table(fixture)
        report["source_hashes_after"] = len(rows)
    report["status"] = "PASS"
    report["phase"] = "complete"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path,
                        default=Path(os.environ["CAJ2PDF_CORPUS_DIR"])
                        if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--table-fixture", type=Path,
                        default=Path(os.environ["CAJ2PDF_T88_H2_FIXTURE_FILE"])
                        if os.environ.get("CAJ2PDF_T88_H2_FIXTURE_FILE") else None)
    parser.add_argument("--rust-bin", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, args.table_fixture, args.rust_bin, report)
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired,
            headers.InventoryError, full.OracleError, parity.ParityError,
            DiagnosticError) as exc:
        report.update(status="FAIL", error=str(exc))
        report["diagnostic"]["status"] = "FAIL"
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
