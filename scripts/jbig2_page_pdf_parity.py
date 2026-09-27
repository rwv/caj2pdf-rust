#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional SHA-pinned selected type-3 PDF and displayed-pixel parity.

The 47-state MQ fixture and all CAJSamples documents remain external. A normal
run does not open either and reports NOT_RUN with zero compatibility matches.
"""

from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import tempfile
import time

import jbig2_oracle as oracle
import jbig2_page_parity as page


ROOT = Path(__file__).resolve().parent.parent
EXPECTED = 546
STRICT = page.STRICT_POLICY
OPT_IN = page.HN_C8_POLICY
ANOMALY = oracle.ANOMALY_COORDINATE
# Fixed coordinates in the pinned #43 manifest, including one image_number > 1.
RENDER_COORDINATES = {
    "hn": (oracle.ANOMALY_ID, 2, 1),
    "c8": (next(key for key in oracle.EXPECTED_TYPE3 if key.startswith("issue-66/")), 1, 1),
    "multi": (next(key for key in oracle.EXPECTED_TYPE3 if key.startswith("issue-76/")), 23, 3),
    "anomaly": ANOMALY,
}
MAX_PDF = 64 * 1024 * 1024
MAX_PBM = 128 * 1024 * 1024
MAX_TEMP = 256 * 1024 * 1024
MAX_TEXT = 32 * 1024
MAX_REQUEST = 64 * 1024
MAX_SCRATCH = 256 * 1024 * 1024
MAX_SOURCE_READ = 512 * 1024 * 1024
REPORT_FIELDS = 17


class PdfParityError(Exception):
    """A requested source, converter, PDF, or pixel check failed."""


def initial_report(policy: str) -> dict:
    return {
        "status": "NOT_RUN", "phase": "preflight", "policy": policy,
        "expected_images": EXPECTED, "attempted": 0, "completed": 0,
        "matching": 0, "strict_anomaly_refusals": 0,
        "opt_in_anomaly_matches": 0, "failed": 0, "skipped": 0,
        "unsupported": 0, "rendered": {}, "first_failure": None,
        "source_hashes_before": {}, "source_hashes_after": {},
        "table_sha256_before": None, "table_sha256_after": None,
        "rust_binary_sha256_before": None, "rust_binary_sha256_after": None,
        "tools": {}, "oracle_backend_independence": "UNVERIFIED",
        "resources": {
            "max_request_bytes": 0, "max_scratch_bytes": 0,
            "max_pdf_bytes": 0, "max_pbm_bytes": 0,
            "max_temporary_bytes": 0, "max_rust_rss_kib": 0,
            "total_reader_bytes": 0, "selected_encoded_bytes": 0,
            "elapsed_seconds": 0.0, "selected_mib_per_second": 0.0,
            "largest_image": None,
        },
    }


def _sha_span(path: Path, offset: int, length: int) -> str:
    if offset < 0 or length <= 0 or offset + length > path.stat().st_size:
        raise PdfParityError("selected source span escapes its file")
    with path.open("rb") as source:
        return oracle.sha256_span(source, offset, length)


def _require_sha(label: str, actual: str, expected: str) -> None:
    if actual != expected:
        raise PdfParityError(f"{label} SHA-256 differs from pinned baseline")


def _tool_output(arguments: list[str], label: str, *, timeout: int = 90,
                 child_env: dict[str, str] | None = None) -> str:
    try:
        process = subprocess.Popen(
            arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, cwd=ROOT, env=child_env,
            start_new_session=True,
        )
    except OSError as exc:
        raise PdfParityError(f"{label} is unavailable: {exc}") from exc
    captured = bytearray()
    started = time.monotonic()
    try:
        with selectors.DefaultSelector() as selector:
            if process.stdout is None:
                raise PdfParityError(f"{label} has no output pipe")
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = timeout - (time.monotonic() - started)
                if remaining <= 0:
                    raise PdfParityError(f"{label} timed out")
                events = selector.select(remaining)
                if not events:
                    raise PdfParityError(f"{label} timed out")
                for key, _ in events:
                    chunk = os.read(key.fileobj.fileno(), 4096)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    captured.extend(chunk)
                    if len(captured) > MAX_TEXT:
                        raise PdfParityError(f"{label} diagnostic output exceeds 32 KiB")
        remaining = timeout - (time.monotonic() - started)
        if remaining <= 0:
            raise PdfParityError(f"{label} timed out")
        process.wait(timeout=remaining)
    except (PdfParityError, OSError, subprocess.TimeoutExpired) as exc:
        if process.poll() is None:
            if os.name == "posix":
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            else:
                process.kill()
        process.wait()
        if isinstance(exc, PdfParityError):
            raise
        raise PdfParityError(f"{label} failed or timed out: {exc}") from exc
    finally:
        if process.stdout is not None:
            process.stdout.close()
    output = captured.decode("utf-8", "replace")
    if process.returncode:
        raise PdfParityError(f"{label} exited {process.returncode}: {output[:2048]}")
    return output


def _tools() -> tuple[dict[str, str], dict]:
    selected = {}
    metadata = {}
    for name in ("qpdf", "pdfinfo", "pdfimages", "pdftoppm", "mutool"):
        path = shutil.which(name)
        if path is None:
            raise PdfParityError(f"required independent PDF tool {name} is missing")
        resolved = Path(path).resolve(strict=True)
        flag = "-v" if name in ("pdfinfo", "pdfimages", "pdftoppm", "mutool") else "--version"
        version = _tool_output([str(resolved), flag], f"{name} version", timeout=10)
        if not version.strip():
            raise PdfParityError(f"{name} did not identify its version")
        selected[name] = str(resolved)
        metadata[name] = {"version": version.splitlines()[0],
                          "binary_sha256": oracle.sha256_file(resolved)}
    return selected, metadata


def _build_binary(override: Path | None) -> Path:
    if override is not None:
        path = override.resolve(strict=True)
        if not path.is_file():
            raise PdfParityError("supplied Rust example is not a file")
        return path
    _tool_output(["cargo", "build", "--locked", "--release", "-p", "caj2pdf-core",
                  "--example", "jbig2_page_pdf"], "Rust PDF diagnostic build", timeout=300)
    return ROOT / "target/release/examples/jbig2_page_pdf"


def _parse_case(stdout: str, case: dict, expected: dict, policy: str) -> dict:
    fields = stdout.strip().split("\t")
    if len(fields) != REPORT_FIELDS or fields[0] != "CASE":
        raise PdfParityError("Rust PDF diagnostic did not emit its 17-field result")
    status, variant, anomaly, kind = fields[1], fields[4], fields[14], fields[15]
    try:
        numbers = [int(value) for value in (fields[2:4] + fields[5:14] + fields[16:17])]
    except ValueError as exc:
        raise PdfParityError("Rust PDF diagnostic emitted a nondecimal metric") from exc
    if any(number < 0 for number in numbers):
        raise PdfParityError("Rust PDF diagnostic emitted a negative metric")
    (source_page, source_image, width, height, offset, length, reader_bytes,
     pdf_bytes, max_request, scratch_bytes, rss_kib, error_offset) = numbers
    if (source_page, source_image) != (case["page"], case["image"]):
        raise PdfParityError("Rust selected the wrong source page or image")
    if (max_request > MAX_REQUEST or scratch_bytes > MAX_SCRATCH
            or reader_bytes > MAX_SOURCE_READ or pdf_bytes > MAX_PDF):
        raise PdfParityError("Rust PDF diagnostic exceeded a measured resource bound")
    if status == "REFUSED":
        if (case["coordinate"] != ANOMALY or policy != STRICT
                or kind != "malformed_text_header"
                or error_offset != expected["flags_offset"]
                or anomaly != "-" or pdf_bytes != 0):
            raise PdfParityError("unexpected selected type-3 PDF refusal")
        return {"status": status, "reader_bytes": reader_bytes,
                "max_request": max_request, "scratch_bytes": scratch_bytes,
                "rss_kib": rss_kib, "error_offset": error_offset}
    variant_matches = (variant == "C8" if case["variant"] == "C8"
                       else variant in ("HN-A", "HN-B"))
    if (status != "COMPLETE" or not variant_matches
            or (width, height) != (expected["width"], expected["height"])
            or (offset, length) != (case["offset"], case["length"])
            or reader_bytes == 0 or pdf_bytes == 0 or max_request == 0
            or kind != "-" or error_offset != 0):
        raise PdfParityError("Rust PDF diagnostic report differs from pinned selection")
    expected_marker = page.ANOMALY_MARKER if case["coordinate"] == ANOMALY else "-"
    if anomaly != expected_marker:
        raise PdfParityError("Rust PDF diagnostic anomaly marker differs")
    return {"status": status, "width": width, "height": height,
            "reader_bytes": reader_bytes, "pdf_bytes": pdf_bytes,
            "max_request": max_request, "scratch_bytes": scratch_bytes,
            "rss_kib": rss_kib}


def _pdf_markers(path: Path) -> None:
    needles = (b"/Decode [1 0]", b"/ColorSpace /DeviceGray", b"/BitsPerComponent 1")
    counts = [0] * len(needles)
    longest = max(map(len, needles))
    tail = b""
    with path.open("rb") as source:
        while block := source.read(64 * 1024):
            joined = tail + block
            for index, needle in enumerate(needles):
                start = max(0, len(tail) - len(needle) + 1)
                counts[index] += joined.count(needle, start)
            tail = joined[-(longest - 1):]
    if counts != [1, 1, 1]:
        raise PdfParityError(f"one-bit black-is-one PDF markers differ: {counts}")


def _file_size(path: Path, maximum: int) -> int:
    size = path.stat().st_size
    if size > maximum:
        raise PdfParityError(f"{path.name} exceeds its {maximum}-byte bound")
    return size


def _validate_pdf(pdf: Path, temporary: Path, expected: dict,
                  tools: dict[str, str]) -> tuple[Path, int]:
    pdf_size = _file_size(pdf, MAX_PDF)
    _pdf_markers(pdf)
    check = _tool_output([tools["qpdf"], "--check", str(pdf)], "qpdf --check")
    if re.search(r"\bwarning\s*:", check, re.IGNORECASE):
        raise PdfParityError("qpdf warned while checking selected PDF")
    pages = _tool_output([tools["qpdf"], "--show-npages", str(pdf)], "qpdf page count")
    if pages.strip() != "1":
        raise PdfParityError("selected PDF is not exactly one page")
    info = _tool_output([tools["pdfinfo"], str(pdf)], "pdfinfo")
    matches = re.findall(r"^Page size:\s+([0-9.]+) x ([0-9.]+) pts", info, re.M)
    if (len(matches) != 1 or abs(float(matches[0][0]) - expected["width"]) > 0.01
            or abs(float(matches[0][1]) - expected["height"]) > 0.01):
        raise PdfParityError("pdfinfo page geometry differs from 72-ppi selected image")
    listing = _tool_output([tools["pdfimages"], "-list", str(pdf)], "pdfimages -list")
    records = [line.split() for line in listing.splitlines()
               if line.lstrip().startswith("1 ")]
    if (len(records) != 1 or len(records[0]) < 8
            or records[0][1:3] != ["0", "image"]
            or records[0][3:8] != [str(expected["width"]), str(expected["height"]),
                                   "gray", "1", "1"]):
        raise PdfParityError("pdfimages does not list one selected one-bit gray image")
    root = temporary / "extracted"
    _tool_output([tools["pdfimages"], str(pdf), str(root)], "pdfimages extraction")
    extracted = temporary / "extracted-000.pbm"
    if (not extracted.is_file()
            or len(list(temporary.glob("extracted-*"))) != 1):
        raise PdfParityError("pdfimages did not produce exactly one binary PBM")
    _file_size(extracted, MAX_PBM)
    return extracted, pdf_size


def _pixel_hash(pbm: Path, expected: dict) -> tuple[str, int]:
    digest = hashlib.sha256()
    black = 0
    with closing(oracle.pbm_rows(pbm, expected["width"], expected["height"])) as rows:
        for row in rows:
            digest.update(row)
            black += sum(byte.bit_count() for byte in row)
    return digest.hexdigest(), black


def _render(pdf: Path, reference: Path, temporary: Path, expected: dict,
            tools: dict[str, str]) -> dict:
    width, height = expected["width"], expected["height"]
    if expected["black_pixels"] == 0:
        raise PdfParityError("renderer canary is blank")
    poppler_root = temporary / "poppler"
    _tool_output([tools["pdftoppm"], "-mono", "-r", "720", "-singlefile",
                  "-f", "1", "-l", "1", str(pdf), str(poppler_root)],
                 "Poppler page render", timeout=180)
    poppler = poppler_root.with_suffix(".pbm")
    _file_size(poppler, MAX_PBM)
    mupdf = temporary / "mupdf.pbm"
    _tool_output([tools["mutool"], "draw", "-q", "-r", "72", "-o",
                  str(mupdf), str(pdf), "1"], "MuPDF page render", timeout=180)
    _file_size(mupdf, MAX_PBM)
    compared = 0
    with (closing(oracle.pbm_rows(reference, width, height)) as wanted,
          closing(oracle.pbm_rows(poppler, width * 10, height * 10)) as large,
          closing(oracle.pbm_rows(mupdf, width, height)) as normal):
        for y in range(height):
            try:
                want = next(wanted)
                mu = next(normal)
            except StopIteration as exc:
                raise PdfParityError(f"renderer PBM ended before row {y}") from exc
            if want != mu:
                raise PdfParityError(f"MuPDF page pixel differs at row {y}")
            middle = b""
            for subrow in range(10):
                try:
                    row = next(large)
                except StopIteration as exc:
                    raise PdfParityError(
                        f"Poppler PBM ended before source row {y}, subrow {subrow}"
                    ) from exc
                if subrow == 5:
                    middle = row
            for x in range(width):
                pixel = bool(want[x // 8] & (0x80 >> (x % 8)))
                centre = x * 10 + 5
                displayed = bool(middle[centre // 8] & (0x80 >> (centre % 8)))
                if pixel != displayed:
                    raise PdfParityError(f"Poppler page pixel differs at ({x},{y})")
                compared += 1
        for name, rows in (("reference", wanted), ("Poppler", large), ("MuPDF", normal)):
            if next(rows, None) is not None:
                raise PdfParityError(f"{name} PBM has extra rows")
    return {"pixels_compared": compared, "mupdf_worst_bit_difference": 0,
            "poppler_centre_worst_bit_difference": 0,
            "comparison_rule": "MuPDF 72-ppi exact; Poppler 720-ppi 10x centre exact"}


def _run_case(binary: Path, fixture: Path, case: dict, expected: dict,
              baseline: dict, policy: str, tools: dict[str, str], report: dict) -> None:
    key = case["coordinate"]
    label = f"{key[0]} page {key[1]} image {key[2]}"
    source = Path(case["source_path"])
    encoded_sha = baseline[key]["encoded_sha256"]
    _require_sha(f"{label} encoded span before", _sha_span(source, case["offset"],
                 case["length"]), encoded_sha)
    try:
        with tempfile.TemporaryDirectory(prefix="caj2pdf-type3-pdf-", dir="/tmp") as name:
            temporary = Path(name)
            pdf = temporary / "selected.pdf"
            result = _tool_output([str(binary), str(fixture), str(source), str(pdf),
                                   str(case["page"]), str(case["image"]), policy],
                                  f"Rust PDF diagnostic {label}", timeout=120,
                                  child_env={**os.environ, "TMPDIR": str(temporary)})
            observed = _parse_case(result, case, expected, policy)
            report["attempted"] += 1
            resources = report["resources"]
            resources["max_request_bytes"] = max(resources["max_request_bytes"],
                                                  observed["max_request"])
            resources["max_scratch_bytes"] = max(resources["max_scratch_bytes"],
                                                  observed["scratch_bytes"])
            resources["max_rust_rss_kib"] = max(resources["max_rust_rss_kib"],
                                                observed["rss_kib"])
            resources["total_reader_bytes"] += observed["reader_bytes"]
            if observed["status"] == "REFUSED":
                if pdf.exists():
                    raise PdfParityError("strict anomaly refusal left a partial PDF")
                report["strict_anomaly_refusals"] += 1
                return
            report["completed"] += 1
            if _file_size(pdf, MAX_PDF) != observed["pdf_bytes"]:
                raise PdfParityError("PDF byte count differs from Rust conversion report")
            extracted, pdf_size = _validate_pdf(pdf, temporary, expected, tools)
            digest, black = _pixel_hash(extracted, expected)
            _require_sha(f"{label} visible packed pixels", digest, expected["pixel_sha256"])
            if black != expected["black_pixels"]:
                raise PdfParityError(f"{label} black-pixel count differs from #43")
            report["matching"] += 1
            if key == ANOMALY:
                report["opt_in_anomaly_matches"] += 1
            resources["selected_encoded_bytes"] += case["length"]
            resources["max_pdf_bytes"] = max(resources["max_pdf_bytes"], pdf_size)
            resources["max_pbm_bytes"] = max(resources["max_pbm_bytes"],
                                               extracted.stat().st_size)
            for role, selected_key in RENDER_COORDINATES.items():
                if key == selected_key:
                    render = _render(pdf, extracted, temporary, expected, tools)
                    report["rendered"][role] = {"coordinate": key, **render}
            temp_bytes = sum(path.stat().st_size for path in temporary.iterdir()
                             if path.is_file())
            if temp_bytes > MAX_TEMP:
                raise PdfParityError("simultaneous child PDF/PBM storage exceeds 256 MiB")
            resources["max_temporary_bytes"] = max(resources["max_temporary_bytes"],
                                                    temp_bytes)
            area = expected["width"] * expected["height"]
            if (resources["largest_image"] is None
                    or area > resources["largest_image"]["pixels"]):
                resources["largest_image"] = {"coordinate": key, "pixels": area,
                                               "pdf_bytes": pdf_size,
                                               "scratch_bytes": observed["scratch_bytes"]}
    finally:
        _require_sha(f"{label} encoded span after", _sha_span(source, case["offset"],
                     case["length"]), encoded_sha)


def run(corpus: Path | None, fixture: Path | None, binary: Path | None = None,
        policy: str = STRICT, report: dict | None = None) -> dict:
    if policy not in (STRICT, OPT_IN):
        raise PdfParityError("unknown type-3 text-header policy")
    report = initial_report(policy) if report is None else report
    manifest = page.load_manifest()
    if corpus is None and fixture is None:
        report["reason"] = "external corpus and private T.88 state table are unset"
        return report
    if corpus is None or fixture is None:
        raise PdfParityError("corpus and private T.88 table must be supplied together")
    fixture = page.instances.parity.check_table(fixture)
    report["table_sha256_before"] = oracle.sha256_file(fixture)
    report["phase"] = "source_precheck"
    rows, paths = oracle.validate_sources(oracle.DEFAULT_MATRIX, corpus)
    report["source_hashes_before"] = {row["id"]: row["sha256"] for row in rows}
    selected_binary = None
    tools = {}
    started = time.monotonic()
    try:
        report["phase"] = "inventory"
        cases = oracle.checked_cases(oracle.run_directory_inventory(corpus), rows, paths)
        _, recorded = page.instances.headers.load_baseline()
        _, dictionaries = page.dictionaries.load_baseline()
        _, selected = page.plan_for_cases(cases, recorded, dictionaries, manifest)
        if len(cases) != EXPECTED or len(selected) != EXPECTED:
            raise PdfParityError("pinned type-3 selection is incomplete")
        baseline = page._oracle_images(manifest)
        for case, expected in zip(cases, selected):
            if case["coordinate"] != expected["coordinate"]:
                raise PdfParityError("fresh inventory order differs from #43")
        report["phase"] = "tools"
        tools, report["tools"] = _tools()
        report["phase"] = "rust_build"
        selected_binary = _build_binary(binary)
        report["rust_binary_sha256_before"] = oracle.sha256_file(selected_binary)
        report["phase"] = "pdf_conversion"
        for case, expected in zip(cases, selected):
            try:
                _run_case(selected_binary, fixture, case, expected, baseline,
                          policy, tools, report)
            except (OSError, ValueError, PdfParityError, oracle.OracleError) as exc:
                raise PdfParityError(
                    f"{case['coordinate']!r}: selected PDF parity failed: {exc}"
                ) from exc
        report["phase"] = "complete"
    finally:
        report["resources"]["elapsed_seconds"] = round(time.monotonic() - started, 3)
        seconds = report["resources"]["elapsed_seconds"]
        if seconds > 0:
            report["resources"]["selected_mib_per_second"] = round(
                report["resources"]["selected_encoded_bytes"] / 1048576 / seconds, 3)
        report["phase_before_postcheck"] = report["phase"]
        report["phase"] = "source_postcheck"
        after, _ = oracle.validate_sources(oracle.DEFAULT_MATRIX, corpus)
        report["source_hashes_after"] = {row["id"]: row["sha256"] for row in after}
        page.instances.parity.check_table(fixture)
        report["table_sha256_after"] = oracle.sha256_file(fixture)
        _require_sha("private T.88 table after run", report["table_sha256_after"],
                     report["table_sha256_before"])
        if selected_binary is not None:
            report["rust_binary_sha256_after"] = oracle.sha256_file(selected_binary)
            _require_sha("Rust diagnostic binary after run",
                         report["rust_binary_sha256_after"],
                         report["rust_binary_sha256_before"])
        for name, path in tools.items():
            _require_sha(f"{name} executable after run", oracle.sha256_file(Path(path)),
                         report["tools"][name]["binary_sha256"])
    expected_matches = EXPECTED - (1 if policy == STRICT else 0)
    expected_renders = {"hn", "c8", "multi"} | ({"anomaly"} if policy == OPT_IN else set())
    if (report["attempted"] != EXPECTED or report["completed"] != expected_matches
            or report["matching"] != expected_matches
            or report["strict_anomaly_refusals"] != int(policy == STRICT)
            or report["opt_in_anomaly_matches"] != int(policy == OPT_IN)
            or set(report["rendered"]) != expected_renders
            or report["failed"] or report["skipped"] or report["unsupported"]):
        raise PdfParityError("strict 545/545 or opt-in 546/546 PDF parity is incomplete")
    report["status"] = "PASS"
    report["phase"] = "complete"
    report["oracle_backend_independence"] = "UNVERIFIED"
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
    parser.add_argument("--text-header-policy", choices=(STRICT, OPT_IN), default=STRICT)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report(args.text_header_policy)
    try:
        run(args.corpus_dir, args.table_fixture, args.rust_bin, args.text_header_policy, report)
    except (OSError, ValueError, TypeError, KeyError, IndexError, PdfParityError,
            oracle.OracleError, page.ParityError,
            page.instances.DiagnosticError,
            page.dictionaries.InventoryError,
            page.instances.parity.ParityError) as exc:
        report.update(status="FAIL", error=str(exc), first_failure=str(exc))
        report["failed"] += 1
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
