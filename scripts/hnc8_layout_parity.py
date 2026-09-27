#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, hash-pinned HN/C8 source-page and reference-PDF layout comparison.

Only source identifiers, checked ranges, dimensions, hashes, page boxes and
image draw matrices enter the committed oracle. Private documents, PDFs,
image streams and text stay outside this repository. A clean clone reports
NOT_RUN with zero comparisons.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil

import conformance
import hnc8_layout_pdf
import hnc8_layout_source
import jbig1_oracle


ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/conformance/matrix.json"
CONTAINER_ORACLE = ROOT / "tests/conformance/jbig1_oracle.json"
LAYOUT_ORACLE = ROOT / "tests/conformance/hnc8_layout_oracle.json"
EXPECTED_SOURCES = 27
EXPECTED_PAGES = 77
EXPECTED_DRAWS = 127
EXPECTED_EXISTING_PAGES = 75
EXPECTED_EXISTING_DRAWS = 125
EXPECTED_DECLARED_ROWS = 2047
EXPECTED_VALID_ROWS = 2044
EXPECTED_VALID_IMAGE_ROWS = 2027
EXPECTED_ZERO_IMAGE_ROWS = 17
EXPECTED_MULTI_IMAGE_ROWS = 405
EXPECTED_MIXED_TYPE_ROWS = 402
EXPECTED_TYPES = {0: 1400, 1: 6, 2: 1085, 3: 546}
MALFORMED_SOURCE = "issue-100/中国金融体制改革阶段研究_李卉.caj"
EXPECTED_MALFORMED = {
    (MALFORMED_SOURCE, 2): (1, 12886, "image type", "unsupported"),
    (MALFORMED_SOURCE, 3): (None, 264, "image count", "malformed"),
    (MALFORMED_SOURCE, 4): (None, 276, "text span", "truncated"),
}
MAX_REFERENCE_PDF = 128 * 1024 * 1024
MAX_ORACLE_BYTES = 4 * 1024 * 1024
CASE_SPECS = (
    ("hn_a", "issue-21/实时网络流量异常检测算法研究和系统实现_林尚朕.caj", 68, 91, None),
    ("c8", "issue-33/test1.caj", 7, 34, None),
    ("hn_b", "issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj", 2, 2, (1, 6)),
)
REFERENCE_PDF_HASHES = {
    "hn_a": "833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40",
    "c8": "acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885",
    "hn_b": "b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51",
}
PDF_TOOLS = ("qpdf", "mutool", "pdfimages")


class LayoutParityError(ValueError):
    """A requested source, PDF, tool or metadata comparison was invalid."""


def initial_report() -> dict:
    return {
        "status": "NOT_RUN", "phase": "baseline", "expected_sources": EXPECTED_SOURCES,
        "expected_existing_pages": EXPECTED_EXISTING_PAGES,
        "expected_existing_draws": EXPECTED_EXISTING_DRAWS,
        "source_hashes_before": {}, "source_hashes_after": {},
        "pdf_hashes_before": {}, "pdf_hashes_after": {},
        "source_pages_checked": 0, "output_pages_checked": 0,
        "draws_checked": 0, "existing_pages_matched": 0,
        "existing_draws_matched": 0, "malformed_checked": 0,
        "mismatched_pages": 0, "mismatched_draws": 0,
        "mismatched_source_rows": 0, "mismatched_case_metadata": 0,
        "placement_unknown_extra_draws": 0,
        "inventory": {},
        "geometry": {"first_box_max_difference": 0.0,
                     "additional_scale_max_difference": 0.0,
                     "fractional_additional_positions": 0,
                     "additional_placement_rule": "UNKNOWN"},
        "failed": 0, "skipped": 0, "unsupported": 0,
        "first_failure": None, "tool_hashes": {}, "resources": {
            "max_source_request_bytes": 0, "source_reader_bytes": 0,
            "max_pdf_bytes": 0, "max_tool_output_bytes": 0,
            "reference_pdf_total_bytes": 0,
            "max_extractor_managed_temp_bytes": 0, "max_validator_rss_kib": 0,
            "python_max_rss_kib": None,
            "temporary_bytes_scope": "extractor-managed files only; reference artifacts measured by generation protocol",
        },
    }


def _sha256_file(path: Path, limit: int) -> str:
    size = path.stat().st_size
    if size < 0 or size > limit:
        raise LayoutParityError(f"{path.name}: file exceeds {limit}-byte bound")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(64 * 1024):
            digest.update(block)
    return digest.hexdigest()


def _linux_vm_hwm_kib() -> int | None:
    """Read this process's peak RSS, avoiding inherited getrusage high water."""
    status = Path("/proc/self/status")
    if not status.is_file():
        return None
    for line in status.read_text(encoding="ascii").splitlines():
        if line.startswith("VmHWM:"):
            fields = line.split()
            if len(fields) != 3 or fields[2] != "kB":
                raise LayoutParityError("Linux VmHWM has an unexpected unit")
            return int(fields[1])
    raise LayoutParityError("Linux process status lacks VmHWM")


def _baseline(oracle_path: Path, *, candidate: bool) -> tuple[list[dict], dict | None]:
    rows = [row for row in conformance.load_matrix(MATRIX)
            if row["detected_type"] in ("HN", "C8")]
    if len(rows) != EXPECTED_SOURCES:
        raise LayoutParityError("HN/C8 source matrix no longer has 27 entries")
    container = jbig1_oracle.load_manifest(CONTAINER_ORACLE)
    matrix_sha = _sha256_file(MATRIX, MAX_ORACLE_BYTES)
    if matrix_sha != container["corpus_matrix_sha256"]:
        raise LayoutParityError("source matrix digest differs from pinned #22 oracle")
    identifiers = {row["id"] for row in rows}
    if len(identifiers) != EXPECTED_SOURCES or any(
            source_id not in identifiers for _, source_id, _, _, _ in CASE_SPECS):
        raise LayoutParityError("selected source identities differ from 27-source matrix")
    if candidate:
        return rows, None
    if not oracle_path.is_file() or oracle_path.stat().st_size > MAX_ORACLE_BYTES:
        raise LayoutParityError("committed layout oracle is missing or too large")
    with oracle_path.open("r", encoding="utf-8") as source:
        oracle = json.load(source)
    if (oracle.get("schema_version") != 1 or oracle.get("matrix_sha256") != matrix_sha
            or [item.get("case") for item in oracle.get("cases", [])]
            != [spec[0] for spec in CASE_SPECS]):
        raise LayoutParityError("layout oracle schema, matrix, or case order differs")
    return rows, oracle


def _source_audit(rows: list[dict], corpus: Path) -> dict[str, str]:
    result = conformance.audit_inventory(rows, corpus)
    if result["status"] != "PASS" or result["passed"] != EXPECTED_SOURCES:
        raise LayoutParityError(f"27-source size/SHA-256 audit failed: {result}")
    return {row["id"]: row["sha256"] for row in rows}


def _tool_paths() -> tuple[dict[str, Path], dict[str, str]]:
    paths = {}
    hashes = {}
    for name in PDF_TOOLS:
        executable = shutil.which(name)
        if executable is None:
            raise LayoutParityError(f"required independent PDF tool {name} is missing")
        path = Path(executable).resolve(strict=True)
        if not path.is_file():
            raise LayoutParityError(f"{name} is not a regular executable file")
        paths[name] = path
        hashes[name] = _sha256_file(path, 128 * 1024 * 1024)
    return paths, hashes


def _full_source_inventory(rows: list[dict], corpus: Path, resources: dict) -> dict:
    declared = valid = image_rows = zero_rows = multi_rows = mixed_rows = 0
    nonzero_raw_12 = next_text_pairs = next_text_matches = nonzero_gaps = 0
    nonzero_raw_12_locations: list[tuple[str, int]] = []
    kinds = {kind: 0 for kind in EXPECTED_TYPES}
    malformed: dict[tuple[str, int], tuple[int | None, int, str, str]] = {}
    for row in rows:
        with hnc8_layout_source.FileInput(corpus / row["id"]) as source:
            extractor = hnc8_layout_source.SourceExtractor(source, row["id"])
            previous_page = None
            for page_number in range(1, extractor.header["page_count"] + 1):
                declared += 1
                try:
                    page = extractor.read_page(page_number)
                except hnc8_layout_source.SourceMetadataError as error:
                    key = (row["id"], page_number)
                    observed = error.image, error.offset, error.field, error.kind
                    if key not in EXPECTED_MALFORMED or observed != EXPECTED_MALFORMED[key]:
                        raise LayoutParityError(f"unexpected source-page error: {error}") from error
                    malformed[key] = observed
                    previous_page = None
                    continue
                valid += 1
                nonzero_raw_12 += page["raw_12"] != 0
                if page["raw_12"] != 0:
                    nonzero_raw_12_locations.append((row["id"], page_number))
                if previous_page is not None and previous_page["raw_16"] != 0:
                    next_text_pairs += 1
                    next_text_matches += previous_page["raw_16"] == page["text_offset"]
                previous_page = page
                image_rows += bool(page["images"])
                zero_rows += not page["images"]
                multi_rows += len(page["images"]) > 1
                mixed_rows += len({image["record_type"] for image in page["images"]}) > 1
                for image in page["images"]:
                    kinds[image["record_type"]] += 1
                    nonzero_gaps += image["gap_length"] != 0
            resources["max_source_request_bytes"] = max(
                resources["max_source_request_bytes"], extractor.max_request_bytes)
            resources["source_reader_bytes"] += extractor.reader_bytes
    if (declared != EXPECTED_DECLARED_ROWS or valid != EXPECTED_VALID_ROWS
            or image_rows != EXPECTED_VALID_IMAGE_ROWS
            or zero_rows != EXPECTED_ZERO_IMAGE_ROWS
            or multi_rows != EXPECTED_MULTI_IMAGE_ROWS
            or mixed_rows != EXPECTED_MIXED_TYPE_ROWS
            or kinds != EXPECTED_TYPES
            or (nonzero_raw_12, next_text_pairs, next_text_matches, nonzero_gaps)
            != (1, 436, 436, 0)
            or nonzero_raw_12_locations != [(MALFORMED_SOURCE, 1)]
            or malformed != EXPECTED_MALFORMED):
        raise LayoutParityError("full 27-source page/descriptor inventory differs")
    return {
        "declared_rows": declared, "valid_rows": valid,
        "image_rows": image_rows, "zero_image_rows": zero_rows,
        "multi_image_rows": multi_rows, "mixed_type_rows": mixed_rows,
        "types": {str(kind): count for kind, count in kinds.items()},
        "unknown_field_observations": {
            "nonzero_raw_12_rows": nonzero_raw_12,
            "nonzero_raw_12_locations": [
                {"source_id": source_id, "page_number": page_number}
                for source_id, page_number in nonzero_raw_12_locations],
            "nonfinal_nonzero_raw_16_pairs": next_text_pairs,
            "raw_16_next_text_matches": next_text_matches,
            "nonzero_descriptor_gaps": nonzero_gaps,
        },
        "malformed": [{"source_id": key[0], "page_number": key[1],
                       "image_number": error[0], "offset": error[1],
                       "field": error[2], "kind": error[3]}
                      for key, error in sorted(malformed.items())],
    }


def _requested_pdfs(arguments: argparse.Namespace) -> dict[str, Path] | None:
    values = {"hn_a": arguments.hn_a_pdf, "c8": arguments.c8_pdf,
              "hn_b": arguments.hn_b_pdf}
    if arguments.corpus_dir is None and all(value is None for value in values.values()):
        return None
    if arguments.corpus_dir is None or any(value is None for value in values.values()):
        raise LayoutParityError("a requested comparison needs the corpus and all three PDFs")
    return {key: Path(value).resolve(strict=True) for key, value in values.items()}


def _compact_page(page: dict) -> dict:
    """Keep only issue #107's checked coordinates, dimensions and hashes."""
    images = []
    for image in page["images"]:
        selected = {name: image[name] for name in (
            "image_number", "record_type", "descriptor_offset", "payload_offset",
            "payload_length", "payload_sha256", "width", "height")}
        if image["record_type"] == 0:
            selected["stride_width"] = image["stride_width"]
        images.append(selected)
    return {"page_number": page["page_number"],
            "text_offset": page["text_offset"],
            "text_length": page["text_length"],
            "text_sha256": page["text_sha256"], "images": images}


def _case_metadata(key: str, source_id: str, source_path: Path, pdf_path: Path,
                   output_pages: int, draws: int, mapping: tuple[int, ...] | None,
                   tools: dict[str, Path], resources: dict) -> dict:
    with hnc8_layout_source.FileInput(source_path) as source:
        extractor = hnc8_layout_source.SourceExtractor(source, source_id)
        expected_source_pages = 6 if key == "hn_b" else output_pages
        if extractor.header["page_count"] != expected_source_pages:
            raise LayoutParityError(f"{key}: source page count differs from pinned scope")
        source_pages = list(extractor.iter_pages())
        source_header = extractor.header
        resources["max_source_request_bytes"] = max(
            resources["max_source_request_bytes"], extractor.max_request_bytes)
        resources["source_reader_bytes"] += extractor.reader_bytes
    pdf = hnc8_layout_pdf.extract_pdf_metadata(pdf_path, tools)
    pdf_resources = pdf.get("resources", {})
    resources["max_tool_output_bytes"] = max(resources["max_tool_output_bytes"],
                                            pdf_resources.get("max_tool_output_bytes", 0))
    resources["max_extractor_managed_temp_bytes"] = max(
        resources["max_extractor_managed_temp_bytes"],
        pdf_resources.get("max_temporary_bytes", 0))
    resources["max_validator_rss_kib"] = max(resources.get("max_validator_rss_kib", 0),
                                              pdf_resources.get("max_child_rss_kib", 0))
    if pdf["page_count"] != output_pages:
        raise LayoutParityError(f"{key}: reference PDF page count differs from pinned scope")
    actual_draws = sum(len(page["draws"]) for page in pdf["pages"])
    if actual_draws != draws:
        raise LayoutParityError(f"{key}: reference PDF draw count differs from pinned scope")
    selected = mapping or tuple(range(1, output_pages + 1))
    if len(selected) != output_pages or any(n < 1 or n > len(source_pages) for n in selected):
        raise LayoutParityError(f"{key}: invalid source-to-output page map")
    if key == "hn_b" and [page["image_count"] for page in source_pages] != [1, 0, 0, 0, 0, 1]:
        raise LayoutParityError("HN-B six-row image-count observation changed")
    for output_number, source_number in enumerate(selected, 1):
        images = source_pages[source_number - 1]["images"]
        drawn = pdf["pages"][output_number - 1]["draws"]
        if len(images) != len(drawn):
            raise LayoutParityError(
                f"{key}: source page {source_number}/PDF page {output_number} draw count differs")
        for image, draw in zip(images, drawn):
            expected_width = image.get("stride_width") if image["record_type"] == 0 else image["width"]
            if (expected_width != draw["width"] or image["height"] != draw["height"]):
                raise LayoutParityError(
                    f"{key}: image dimensions differ at source page {source_number}"
                    f" image {image['image_number']}")
            if (image["record_type"] == 2
                    and (draw["filter"] != "/DCTDecode"
                         or image["payload_sha256"] != draw["raw_stream_sha256"])):
                raise LayoutParityError(
                    f"{key}: JPEG stream identity differs at source page {source_number}"
                    f" image {image['image_number']}")
    return {
        "case": key, "source_id": source_id, "source_variant": source_header["variant"],
        "source_pages": [_compact_page(page) for page in source_pages],
        "output_page_to_source_page": list(selected),
        "pdf_sha256": _sha256_file(pdf_path, MAX_REFERENCE_PDF),
        "pdf_pages": pdf["pages"],
    }


def _geometry(cases: list[dict]) -> dict:
    first_max = extra_max = 0.0
    extra_count = fractional = 0
    existing_types = {0: 0, 2: 0}
    for case in cases:
        mapping = case["output_page_to_source_page"]
        if case["case"] == "hn_b":
            empty = case["source_pages"][1:5]
            if len(empty) != 4 or any(len(page["images"]) != 0
                                       or page["text_length"] <= 0
                                       or page["text_sha256"] is None for page in empty):
                raise LayoutParityError("HN-B four image-free positive-text rows differ")
        for output_index, pdf_page in enumerate(case["pdf_pages"]):
            source_page = case["source_pages"][mapping[output_index] - 1]
            box = pdf_page["media_box"]
            if len(box) != 4 or any(not math.isfinite(v) for v in box):
                raise LayoutParityError("nonfinite or incomplete PDF MediaBox")
            width, height = box[2] - box[0], box[3] - box[1]
            if width <= 0 or height <= 0:
                raise LayoutParityError("empty PDF MediaBox")
            for draw_index, (image, draw) in enumerate(zip(source_page["images"],
                                                           pdf_page["draws"])):
                ctm = draw["pdf_ctm"]
                if len(ctm) != 6 or any(not math.isfinite(v) for v in ctm):
                    raise LayoutParityError("nonfinite or incomplete image CTM")
                a, b, c, d, e, f = ctm
                pixels_w = image["stride_width"] if image["record_type"] == 0 else image["width"]
                pixels_h = image["height"]
                if pixels_w is None or pixels_h is None:
                    raise LayoutParityError("unmeasured image dimensions in layout subset")
                if case["case"] != "hn_b":
                    existing_types[image["record_type"]] = existing_types.get(image["record_type"], 0) + 1
                if draw_index == 0:
                    first_max = max(first_max, abs(width - pixels_w * 0.24),
                                    abs(height - pixels_h * 0.24), abs(a - width),
                                    abs(d + height), abs(e - box[0]),
                                    abs(f - box[3]), abs(b), abs(c))
                else:
                    extra_count += 1
                    extra_max = max(extra_max, abs(a - pixels_w * 0.24),
                                    abs(d + pixels_h * 0.24), abs(b), abs(c))
                    fractional += (abs(e - round(e)) > 1e-6
                                   or abs(f - round(f)) > 1e-6)
    if (first_max > 1e-6 or extra_max > 1e-6 or extra_count != 50
            or fractional != 50 or existing_types != {0: 74, 2: 51}):
        raise LayoutParityError("measured 300-ppi first/additional image geometry differs")
    return {"first_box_max_difference": first_max,
            "additional_scale_max_difference": extra_max,
            "fractional_additional_positions": fractional,
            "additional_placement_rule": "UNKNOWN",
            "existing_image_types": {str(kind): count for kind, count in existing_types.items()}}


def _mismatch_counts(actual: dict, expected: dict) -> tuple[int, int, int, int]:
    source_rows = output_pages = draws = cases = 0
    for observed_case, pinned_case in zip(actual["cases"], expected["cases"]):
        cases += ({key: value for key, value in observed_case.items()
                   if key not in ("source_pages", "pdf_pages")}
                  != {key: value for key, value in pinned_case.items()
                      if key not in ("source_pages", "pdf_pages")})
        observed_source = observed_case["source_pages"]
        pinned_source = pinned_case["source_pages"]
        source_rows += abs(len(observed_source) - len(pinned_source))
        source_rows += sum(left != right for left, right in zip(observed_source, pinned_source))
        observed_pages = observed_case["pdf_pages"]
        pinned_pages = pinned_case["pdf_pages"]
        output_pages += abs(len(observed_pages) - len(pinned_pages))
        for left, right in zip(observed_pages, pinned_pages):
            output_pages += left != right
            left_draws, right_draws = left["draws"], right["draws"]
            draws += abs(len(left_draws) - len(right_draws))
            draws += sum(one != two for one, two in zip(left_draws, right_draws))
    return source_rows, output_pages, draws, cases


def _write_candidate(data: dict, output: Path, corpus: Path) -> None:
    resolved = output.resolve()
    if resolved.is_relative_to(ROOT.resolve()) or resolved.is_relative_to(corpus.resolve()):
        raise LayoutParityError("candidate output must be outside the repository and corpus")
    if output.exists() or output.is_symlink():
        raise LayoutParityError("candidate output must be a new file")
    encoded = json.dumps(data, ensure_ascii=False, sort_keys=True,
                         separators=(",", ":")).encode("utf-8")
    if len(encoded) > MAX_ORACLE_BYTES:
        raise LayoutParityError("metadata-only candidate oracle exceeds 4 MiB")
    # Exclusive creation also protects against an existing PDF or executable
    # changed between the post-audit and this write.
    with output.open("xb") as target:
        target.write(encoded + b"\n")


def run(arguments: argparse.Namespace, report: dict | None = None) -> dict:
    report = initial_report() if report is None else report
    candidate = arguments.candidate_output is not None
    rows, expected = _baseline(arguments.oracle, candidate=candidate)
    requested = _requested_pdfs(arguments)
    if requested is None:
        if candidate:
            raise LayoutParityError("candidate generation requires the corpus and all three PDFs")
        report["reason"] = "external corpus and reference PDFs are unset"
        return report
    corpus = Path(arguments.corpus_dir).resolve(strict=True)
    if not corpus.is_dir():
        raise LayoutParityError("requested corpus directory is missing")
    report["phase"] = "source_precheck"
    report["source_hashes_before"] = _source_audit(rows, corpus)
    report["phase"] = "pdf_precheck"
    for key, path in requested.items():
        report["pdf_hashes_before"][key] = _sha256_file(path, MAX_REFERENCE_PDF)
    if any(report["pdf_hashes_before"][key] != expected_hash
           for key, expected_hash in REFERENCE_PDF_HASHES.items()):
        raise LayoutParityError("requested reference PDF differs from pinned black-box artifact")
    tools, hashes = _tool_paths()
    report["tool_hashes"] = hashes
    if expected is not None and (hashes != expected.get("pdf_tool_hashes")
                                 or report["pdf_hashes_before"] != {
                                     item["case"]: item["pdf_sha256"] for item in expected["cases"]}):
        raise LayoutParityError("PDF tool or requested reference artifact digest differs")
    cases = []
    existing_pages_checked = existing_draws_checked = 0
    try:
        report["phase"] = "source_inventory"
        report["inventory"] = _full_source_inventory(rows, corpus, report["resources"])
        report["malformed_checked"] = len(report["inventory"]["malformed"])
        report["phase"] = "extract"
        for key, source_id, output_pages, draws, mapping in CASE_SPECS:
            source_path = corpus / source_id
            metadata = _case_metadata(key, source_id, source_path, requested[key],
                                      output_pages, draws, mapping, tools, report["resources"])
            cases.append(metadata)
            report["source_pages_checked"] += len(metadata["source_pages"])
            report["output_pages_checked"] += len(metadata["pdf_pages"])
            report["draws_checked"] += sum(len(page["draws"]) for page in metadata["pdf_pages"])
            report["resources"]["max_pdf_bytes"] = max(
                report["resources"]["max_pdf_bytes"], requested[key].stat().st_size)
            report["resources"]["reference_pdf_total_bytes"] += requested[key].stat().st_size
            if key != "hn_b":
                existing_pages_checked += len(metadata["pdf_pages"])
                existing_draws_checked += draws
    finally:
        report["phase"] = "postcheck"
        report["source_hashes_after"] = _source_audit(rows, corpus)
        report["pdf_hashes_after"] = {
            key: _sha256_file(path, MAX_REFERENCE_PDF) for key, path in requested.items()}
        if (report["source_hashes_before"] != report["source_hashes_after"]
                or report["pdf_hashes_before"] != report["pdf_hashes_after"]):
            raise LayoutParityError("source or reference PDF changed during layout comparison")
        _, after_hashes = _tool_paths()
        if after_hashes != hashes:
            raise LayoutParityError("independent PDF tool changed during layout comparison")
        report["resources"]["python_max_rss_kib"] = _linux_vm_hwm_kib()
    if (report["output_pages_checked"] != EXPECTED_PAGES
            or report["draws_checked"] != EXPECTED_DRAWS
            or existing_pages_checked != EXPECTED_EXISTING_PAGES
            or existing_draws_checked != EXPECTED_EXISTING_DRAWS):
        raise LayoutParityError("layout page/draw scope is incomplete")
    report["geometry"] = _geometry(cases)
    report["placement_unknown_extra_draws"] = 50
    candidate_data = {
        "schema_version": 1,
        "matrix_sha256": _sha256_file(MATRIX, MAX_ORACLE_BYTES),
        "pdf_tool_hashes": hashes,
        "cases": cases,
    }
    if candidate:
        _write_candidate(candidate_data, Path(arguments.candidate_output), corpus)
    elif candidate_data != expected:
        (report["mismatched_source_rows"], report["mismatched_pages"],
         report["mismatched_draws"],
         report["mismatched_case_metadata"]) = _mismatch_counts(candidate_data, expected)
        raise LayoutParityError("reference source/PDF layout metadata differs from pinned oracle")
    report["existing_pages_matched"] = existing_pages_checked
    report["existing_draws_matched"] = existing_draws_checked
    report["status"] = "PASS"
    report["phase"] = "complete"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--hn-a-pdf", type=Path)
    parser.add_argument("--c8-pdf", type=Path)
    parser.add_argument("--hn-b-pdf", type=Path)
    parser.add_argument("--oracle", type=Path, default=LAYOUT_ORACLE)
    parser.add_argument("--candidate-output", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report()
    try:
        run(args, report)
    except hnc8_layout_pdf.PdfMetadataUnsupported as error:
        report.update(status="FAIL", failed=report["failed"] + 1,
                      unsupported=report["unsupported"] + 1,
                      first_failure=str(error))
    except (OSError, ValueError, KeyError, TypeError,
            hnc8_layout_source.SourceMetadataError,
            hnc8_layout_pdf.PdfMetadataError) as error:
        report.update(status="FAIL", failed=report["failed"] + 1,
                      first_failure=str(error))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True) if args.json else report)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
