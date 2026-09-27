#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, SHA-pinned arithmetic text-instance control-flow diagnostic.

The private T.88 MQ table and CAJSamples files stay outside Git. A completed
instance trace is not a placement or pixel comparison against the #85 oracle.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import tempfile
import time

import jbig2_dictionary_headers as dictionaries
import jbig2_generic_parity as parity
import jbig2_oracle as full
import jbig2_second_dictionary_diagnostic as second
import jbig2_text_oracle as text_oracle
import jbig2_text_region_headers as headers


ROOT = Path(__file__).resolve().parent.parent
EXPECTED = 546
STANDARD = 545
MAX_PLAN_BYTES = 2 * 1024 * 1024
MAX_OUTPUT_BYTES = 2 * 1024 * 1024
TIMEOUT_SECONDS = 3600
TEXT_MANIFEST_SHA256 = "be40e804b613716745edd8d432a4576c09dccbd0dcf148696a5494fb556f3310"


class DiagnosticError(Exception):
    """A requested input, decoder result, or bounded plan is invalid."""


def initial_report() -> dict:
    return {
        "status": "NOT_RUN", "phase": "preflight", "expected_images": EXPECTED,
        "source_hashes_before": 0, "source_hashes_after": 0,
        "table_hashes_before": 0, "table_hashes_after": 0,
        "diagnostic": {
            "status": "NOT_RUN", "attempted_cases": 0,
            "standard_valid_attempted": 0, "complete_regions": 0,
            "refused_regions": 0, "strict_header_refusals": 0,
            "completed_instances": 0, "ri_zero": 0, "ri_one": 0,
            "strips": 0, "first_typed_refusal": None,
            "first_standard_refusal": None, "region_traces": [],
        },
        "placement_compatibility": {
            "status": "NOT_RUN", "checked_cases": 0, "passed": 0,
            "reason": "no independent placement comparison is configured",
        },
        "pixel_compatibility": {
            "status": "NOT_RUN", "checked_cases": 0, "passed": 0,
            "reason": "region composition and comparison against #85 remain pending",
        },
    }


def load_text_manifest(path: Path) -> dict:
    with path.open("rb") as source:
        encoded = source.read(4 * 1024 * 1024 + 1)
    if len(encoded) > 4 * 1024 * 1024:
        raise DiagnosticError("text oracle manifest exceeds 4 MiB")
    if hashlib.sha256(encoded).hexdigest() != TEXT_MANIFEST_SHA256:
        raise DiagnosticError("text oracle manifest SHA-256 differs from pinned #85 baseline")
    manifest = json.loads(encoded.decode("utf-8"))
    text_oracle.validate_manifest(manifest)
    return manifest


def plan_for_cases(cases: list[dict], recorded: dict, dictionary_manifest: dict,
                   text_manifest: dict) -> tuple[list[str], list[tuple[tuple, str, int, int]]]:
    """Join #42, #43, #66, #69, and #85 before invoking the decoder."""
    if len(cases) != EXPECTED:
        raise DiagnosticError(f"fresh directory has {len(cases)} images, expected {EXPECTED}")
    prepared = text_oracle.preflight(cases, recorded, text_manifest)
    dictionary_lines, dictionary_keys = second.plan_for_cases(cases, dictionary_manifest)
    if len(prepared) != EXPECTED or len(dictionary_lines) != EXPECTED:
        raise DiagnosticError("pinned text region set is incomplete")
    lines = []
    selected = []
    for base, key, item in zip(dictionary_lines, dictionary_keys, prepared):
        case = item["case"]
        if key != case["coordinate"]:
            raise DiagnosticError("dictionary and text coordinate order differs")
        span = item["spans"][3]
        declared = item["header"]["instances"]
        kind = item["classification"]
        fields = (span.offset, span.length, item["hashes"][3], declared, kind)
        lines.append(base + "\t" + "\t".join(
            second._plan_field(value, str(key)) for value in fields))
        selected.append((key, kind, declared, item["header"]["data_offset"] + 17))
    if (sum(len(line.encode("utf-8")) + 1 for line in lines) > MAX_PLAN_BYTES
            or sum(kind == text_oracle.STANDARD for _, kind, _, _ in selected) != STANDARD
            or sum(kind == text_oracle.ANOMALY for _, kind, _, _ in selected) != 1):
        raise DiagnosticError("text-instance plan exceeds its bound or profile")
    return lines, selected


def parse_output(output: str, selected: list[tuple[tuple, str, int, int]]) -> dict:
    lines = output.splitlines()
    if len(selected) != EXPECTED or len(lines) != EXPECTED + 1:
        raise DiagnosticError(f"Rust emitted {len(lines)} lines, expected {EXPECTED + 1}")
    counts = {"complete_regions": 0, "refused_regions": 0,
              "strict_header_refusals": 0, "completed_instances": 0,
              "ri_zero": 0, "ri_one": 0, "strips": 0}
    first_refusal = None
    first_standard_refusal = None
    traces = []
    for index, (line, (key, kind, declared, flags_offset)) in enumerate(zip(lines[:-1], selected)):
        fields = line.split("\t")
        if len(fields) != 13 or fields[0] != "CASE":
            raise DiagnosticError(f"Rust case line {index} has invalid fields")
        try:
            actual = (fields[1], int(fields[2]), int(fields[3]))
            completed, ri_zero, ri_one, strips, offset = map(int, (
                fields[5], fields[6], fields[7], fields[8], fields[10]))
        except ValueError as exc:
            raise DiagnosticError(f"Rust case line {index} has invalid numbers") from exc
        status, refusal, decision, trace_sha = fields[4], fields[9], fields[11], fields[12]
        if (actual != key or min(completed, ri_zero, ri_one, strips, offset) < 0
                or completed > declared or ri_zero + ri_one != completed
                or strips > 1_000_000 or re.fullmatch(r"[A-Za-z][A-Za-z0-9]*", decision) is None
                or re.fullmatch(r"[0-9a-f]{64}", trace_sha) is None):
            raise DiagnosticError(f"Rust case line {index} differs from pinned plan")
        if kind == text_oracle.ANOMALY:
            if (status != "HEADER_REFUSED" or refusal != "malformed_text_header"
                    or decision != "Header"
                    or completed or strips or offset != flags_offset
                    or trace_sha != hashlib.sha256(b"").hexdigest()):
                raise DiagnosticError("0xa40c did not fail as a strict header")
            counts["strict_header_refusals"] += 1
        elif status == "COMPLETE":
            if refusal != "-" or completed != declared or decision != "Complete":
                raise DiagnosticError(f"Rust complete case {index} lacks declared instances")
            counts["complete_regions"] += 1
        elif status == "REFUSED":
            if not refusal or refusal == "-" or any(char.isspace() for char in refusal):
                raise DiagnosticError(f"Rust case {index} lacks a typed refusal")
            counts["refused_regions"] += 1
        else:
            raise DiagnosticError(f"Rust case {index} has invalid status {status!r}")
        if status != "COMPLETE" and first_refusal is None:
            first_refusal = {"coordinate": key, "kind": refusal,
                             "source_byte_offset": offset,
                             "semantic_decision": decision,
                             "completed_instances": completed}
        if status == "REFUSED" and first_standard_refusal is None:
            first_standard_refusal = {"coordinate": key, "kind": refusal,
                                      "source_byte_offset": offset,
                                      "semantic_decision": decision,
                                      "completed_instances": completed}
        traces.append({"coordinate": key, "status": status,
                       "completed_instances": completed, "ri_zero": ri_zero,
                       "ri_one": ri_one, "strips": strips,
                       "semantic_decision": decision,
                       "event_sha256": trace_sha})
        counts["completed_instances"] += completed
        counts["ri_zero"] += ri_zero
        counts["ri_one"] += ri_one
        counts["strips"] += strips
    if lines[-1].split("\t") != ["TOTAL", str(EXPECTED)]:
        raise DiagnosticError("Rust total does not acknowledge all 546 attempts")
    trace_status = "PASS" if (counts["complete_regions"] == STANDARD
                              and counts["refused_regions"] == 0
                              and counts["strict_header_refusals"] == 1) else "INCOMPLETE"
    return {"status": trace_status, "attempted_cases": EXPECTED,
            "standard_valid_attempted": STANDARD, **counts,
            "first_typed_refusal": first_refusal,
            "first_standard_refusal": first_standard_refusal,
            "region_traces": traces}


def build_binary(override: Path | None) -> Path:
    if override is not None:
        return override.resolve(strict=True)
    result = parity.bounded_run([
        "cargo", "build", "--locked", "--release", "-p", "caj2pdf-core",
        "--example", "jbig2_text_instance_diagnostic",
    ], "Rust text-instance diagnostic build", 300)
    if result.returncode:
        raise DiagnosticError(f"Rust diagnostic build failed: {result.stderr[-4096:]}")
    return ROOT / "target/release/examples/jbig2_text_instance_diagnostic"


def bounded_decode(command: list[str]) -> str:
    """Capture at most 2 MiB per pipe while the decoder is running."""
    process = subprocess.Popen(
        command, cwd=ROOT, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        start_new_session=True,
    )
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    started = time.monotonic()
    try:
        with selectors.DefaultSelector() as selector:
            assert process.stdout is not None and process.stderr is not None
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                remaining = TIMEOUT_SECONDS - (time.monotonic() - started)
                if remaining <= 0:
                    raise DiagnosticError("Rust text-instance diagnostic timed out")
                events = selector.select(remaining)
                if not events:
                    raise DiagnosticError("Rust text-instance diagnostic timed out")
                for key, _ in events:
                    chunk = os.read(key.fileobj.fileno(), 4096)
                    if chunk:
                        output = captured[key.data]
                        output.extend(chunk)
                        if len(output) > MAX_OUTPUT_BYTES:
                            raise DiagnosticError(
                                f"Rust text-instance diagnostic {key.data} exceeds 2 MiB")
                    else:
                        selector.unregister(key.fileobj)
        remaining = TIMEOUT_SECONDS - (time.monotonic() - started)
        if remaining <= 0:
            raise DiagnosticError("Rust text-instance diagnostic timed out")
        process.wait(timeout=remaining)
    except (DiagnosticError, subprocess.TimeoutExpired):
        if process.poll() is None:
            if os.name == "posix":
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            else:
                process.kill()
        process.wait()
        raise
    finally:
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()
    if process.returncode:
        detail = captured["stderr"].decode("utf-8", "replace")
        raise DiagnosticError(f"Rust diagnostic exited {process.returncode}: {detail[-4096:]}")
    return captured["stdout"].decode("utf-8")


def run_binary(binary: Path, fixture: Path, lines: list[str], selected: list) -> dict:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-text-instance-diagnostic-", dir="/tmp") as temp:
        plan = Path(temp) / "plan.tsv"
        plan.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if plan.stat().st_size > MAX_PLAN_BYTES:
            raise DiagnosticError("written text-instance plan exceeds 2 MiB")
        output = bounded_decode([str(binary), str(fixture), str(plan)])
        return parse_output(output, selected)


def run(corpus: Path | None, fixture: Path | None, binary: Path | None = None,
        manifest_path: Path = text_oracle.DEFAULT_MANIFEST,
        report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    text_manifest = load_text_manifest(manifest_path)
    if corpus is None and fixture is None:
        report["reason"] = "external corpus and private T.88 state table are unset"
        return report
    if corpus is None or fixture is None:
        raise DiagnosticError("corpus and private T.88 state table must be supplied together")
    fixture = parity.check_table(fixture)
    report["table_hashes_before"] = 1
    rows, recorded = headers.load_baseline()
    _, dictionary_manifest = dictionaries.load_baseline()
    report["phase"] = "source_precheck"
    root = headers.audit_sources(rows, corpus)
    report["source_hashes_before"] = len(rows)
    try:
        report["phase"] = "inventory"
        paths = {row["id"]: headers.conformance.contained_file(
            root, headers.conformance.relative_path(row["path"])) for row in rows}
        cases = full.checked_cases(full.run_directory_inventory(root), rows, paths)
        lines, selected = plan_for_cases(cases, recorded, dictionary_manifest, text_manifest)
        report["phase"] = "rust_build"
        selected_binary = build_binary(binary)
        report["phase"] = "rust_decode"
        report["diagnostic"] = run_binary(selected_binary, fixture, lines, selected)
    finally:
        active_phase = report["phase"]
        report["phase"] = "source_postcheck"
        postcheck_errors = []
        try:
            headers.audit_sources(rows, root)
            report["source_hashes_after"] = len(rows)
        except (headers.InventoryError, OSError) as exc:
            postcheck_errors.append(f"source postcheck: {exc}")
        try:
            parity.check_table(fixture)
            report["table_hashes_after"] = 1
        except (parity.ParityError, OSError) as exc:
            postcheck_errors.append(f"table postcheck: {exc}")
        if postcheck_errors:
            raise DiagnosticError("; ".join(postcheck_errors))
        report["phase"] = active_phase
    if report["diagnostic"]["status"] != "PASS":
        report.update(status="FAIL", phase="complete",
                      error="one or more standard text regions did not complete")
    else:
        report.update(status="PASS", phase="complete")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path,
                        default=Path(os.environ["CAJ2PDF_CORPUS_DIR"])
                        if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--table-fixture", type=Path,
                        default=Path(os.environ["CAJ2PDF_T88_H2_FIXTURE_FILE"])
                        if os.environ.get("CAJ2PDF_T88_H2_FIXTURE_FILE") else None)
    parser.add_argument("--manifest", type=Path, default=text_oracle.DEFAULT_MANIFEST)
    parser.add_argument("--rust-bin", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        report = run(args.corpus_dir, args.table_fixture, args.rust_bin, args.manifest, report)
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError,
            subprocess.TimeoutExpired, dictionaries.InventoryError,
            headers.InventoryError, full.OracleError, parity.ParityError,
            DiagnosticError) as exc:
        report.update(status="FAIL", error=str(exc))
        report["diagnostic"]["status"] = "FAIL"
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
