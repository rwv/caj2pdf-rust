#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, bounded verification of a native source-derived HN-A/C8 profile.

The native tool receives only an original source path. This diagnostic
independently checks its metadata against pinned source and PDF observations;
it never invokes the external converter or supplies reference geometry to
the native calculation. A pass concerns these two already inspected documents,
not unseen layouts, authoritative source units or production conversion.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import sys
from typing import Any, Mapping

import hnc8_layout_pdf as pdf
import hnc8_layout_reference as reference
import hnc8_placement_analysis as analysis
import hnc8_placement_probe as shared
import hnc8_text_frame as frame


MAX_OUTPUT_BYTES = 1024 * 1024
MAX_LINE_BYTES = 1024
MAX_PAGES = 4096
MAX_IMAGES = 16_384
MAX_IMAGES_PER_PAGE = 8192
MAX_NATIVE_REQUEST_BYTES = 4096
MAX_NATIVE_WORKING_BYTES = 1024 * 1024
DECODER_RESERVATION_BYTES = 128 * 1024
FIXED_TEXT_SCRATCH_BYTES = 4096
NATIVE_TIMEOUT_SECONDS = 30
MAX_NATIVE_BYTES = 256 * 1024 * 1024
MAX_SOURCE_FILES = 512
MAX_SOURCE_FILE_BYTES = 1024 * 1024
MAX_SOURCE_TREE_BYTES = 16 * 1024 * 1024
TOLERANCE_PT = 0.00005
SUPPORTED_PROFILES = ("hn_a", "c8")
EXPECTED_COMPARISONS = {"page_boxes": 75, "first_draws": 75,
                        "discovery_draws": 36, "validation_draws": 14}
REQUIRED_PATHS = {"corpus", "reference_repo", "python", "pydeps", "libjbigdec",
                  "native_tool", "git", "qpdf", "mutool", "pdfinfo", "pdfimages"}
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
_INTEGER = re.compile(r"(?:0|[1-9][0-9]{0,19})\Z")
_NUMBER = re.compile(r"[-+]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][-+]?[0-9]+)?\Z")


class RuleError(ValueError):
    """An explicitly requested input, native protocol or comparison failed."""


def _integer(value: object, label: str, low: int = 0, high: int = 2**64 - 1) -> int:
    if type(value) is int:
        result = value
    elif type(value) is str and _INTEGER.fullmatch(value):
        result = int(value)
    else:
        raise RuleError(f"{label} is not an unsigned integer")
    if not low <= result <= high:
        raise RuleError(f"{label} is outside the accepted range")
    return result


def _number(value: object, label: str) -> float:
    if type(value) in (int, float):
        try:
            result = float(value)
        except OverflowError as exc:
            raise RuleError(f"{label} exceeds the coordinate cap") from exc
    elif type(value) is str and len(value) <= 64 and _NUMBER.fullmatch(value):
        result = float(value)
    else:
        raise RuleError(f"{label} is not a finite number")
    if not math.isfinite(result) or abs(result) > 1e9:
        raise RuleError(f"{label} is nonfinite or exceeds the coordinate cap")
    return result


def _sha(value: object, label: str) -> str:
    if type(value) is not str or _SHA256.fullmatch(value) is None:
        raise RuleError(f"{label} is not a lowercase SHA-256")
    return value


def parse_native_tsv(data: bytes, source_size: int) -> dict[str, Any]:
    """Parse exact H/P/B/I/R order without accepting partial native output."""
    source_size = _integer(source_size, "source size", 1, frame.MAX_SOURCE_BYTES)
    if type(data) is not bytes or not data or len(data) > MAX_OUTPUT_BYTES:
        raise RuleError("native output is absent, nonbytes or exceeds 1 MiB")
    if not data.endswith(b"\n") or b"\r" in data or b"\x00" in data:
        raise RuleError("native TSV must use complete LF-terminated rows")
    try:
        lines = data[:-1].decode("ascii").split("\n")
    except UnicodeDecodeError as exc:
        raise RuleError("native TSV is not ASCII") from exc
    if any(not line or len(line) > MAX_LINE_BYTES for line in lines):
        raise RuleError("native TSV has an empty or oversized row")
    cursor = 0

    def row(kind: str, columns: int) -> list[str]:
        nonlocal cursor
        if cursor >= len(lines):
            raise RuleError(f"native TSV is truncated before {kind}")
        fields = lines[cursor].split("\t")
        cursor += 1
        if len(fields) != columns or fields[0] != kind:
            raise RuleError(f"native TSV has unexpected {kind} row order or columns")
        return fields

    header = row("H", 3)
    if header[1] not in ("HN-A", "C8"):
        raise RuleError("native variant is outside the HN-A/C8 profile")
    page_count = _integer(header[2], "page count", 1, MAX_PAGES)
    pages = []
    image_total = 0
    minimum_read_bytes = 0
    for page_number in range(1, page_count + 1):
        fields = row("P", 13)
        if _integer(fields[1], "page number", 1, page_count) != page_number:
            raise RuleError("native pages are duplicated, missing or out of order")
        image_count = _integer(fields[2], "image count", 1, MAX_IMAGES_PER_PAGE)
        image_total += image_count
        if image_total > MAX_IMAGES:
            raise RuleError("native image count exceeds the diagnostic cap")
        offset = _integer(fields[3], "text offset", 0, source_size)
        length = _integer(fields[4], "text length", 30, frame.FrameLimits().max_span_bytes)
        if length > source_size - offset:
            raise RuleError("native text span exceeds the original source")
        decoded = _integer(fields[5], "decoded length", 1, frame.FrameLimits().max_decoded_bytes)
        records = _integer(fields[6], "record count", 0, frame.FrameLimits().max_records)
        if decoded != 12 + 16 * records + 28 * image_count:
            raise RuleError("native decoded length and record counts disagree")
        owned = _integer(fields[11], "owned buffer bytes", 1, MAX_NATIVE_WORKING_BYTES)
        working = _integer(fields[12], "working memory bytes", 1, MAX_NATIVE_WORKING_BYTES)
        if working != owned + DECODER_RESERVATION_BYTES + FIXED_TEXT_SCRATCH_BYTES:
            raise RuleError("native working memory disagrees with its decoder and scratch reservations")
        page = {"page_number": page_number, "image_count": image_count,
                "text_offset": offset, "text_length": length,
                "decoded_length": decoded, "record_count": records,
                "encoded_sha256": _sha(fields[7], "encoded frame SHA-256"),
                "decoded_sha256": _sha(fields[8], "decoded SHA-256"),
                "max_source_request_bytes": _integer(fields[9], "text read request", 1,
                                                       MAX_NATIVE_REQUEST_BYTES),
                "max_decoder_output_chunk_bytes": _integer(fields[10], "decoder chunk", 1,
                                                            MAX_NATIVE_REQUEST_BYTES),
                "owned_buffer_bytes": owned, "working_memory_bytes": working,
                "images": []}
        box = row("B", 6)
        if _integer(box[1], "box page", 1, page_count) != page_number:
            raise RuleError("native box page differs from its source page")
        page["media_box"] = [_number(value, "MediaBox") for value in box[2:]]
        if page["media_box"][0] >= page["media_box"][2] or page["media_box"][1] >= page["media_box"][3]:
            raise RuleError("native MediaBox is collapsed or reversed")
        minimum_read_bytes += length
        for image_number in range(1, image_count + 1):
            image = row("I", 19)
            if (_integer(image[1], "image page", 1, page_count) != page_number or
                    _integer(image[2], "image number", 1, image_count) != image_number):
                raise RuleError("native image identities are duplicated, missing or out of order")
            kind = _integer(image[3], "record type", 0, 2)
            if kind not in (0, 2):
                raise RuleError("native image type is unsupported")
            descriptor = _integer(image[4], "descriptor offset", 0, source_size)
            payload = _integer(image[5], "payload offset", 0, source_size)
            payload_length = _integer(image[6], "payload length", 1, 64 * 1024 * 1024)
            if descriptor > source_size - 12 or payload_length > source_size - payload:
                raise RuleError("native image span exceeds the original source")
            visible_width = _integer(image[7], "visible width", 1, 100_000)
            display_width = _integer(image[8], "display width", 1, 100_032)
            height = _integer(image[9], "image height", 1, 100_000)
            expected_width = ((visible_width + 31) // 32) * 32 if kind == 0 else visible_width
            if display_width != expected_width:
                raise RuleError("native display width disagrees with type-specific row padding")
            page["images"].append({
                "image_number": image_number, "record_type": kind,
                "descriptor_offset": descriptor, "payload_offset": payload,
                "payload_length": payload_length, "width": visible_width,
                "display_width": display_width, "height": height,
                "x_word": _integer(image[10], "raw x word", 0, 65535),
                "y_word": _integer(image[11], "raw y word", 0, 65535),
                "payload_sha256": _sha(image[12], "image payload SHA-256"),
                "pdf_ctm": [_number(value, "image CTM") for value in image[13:]],
            })
            minimum_read_bytes += payload_length
        pages.append(page)
    values = row("R", 6)
    resources = {
        "source_read_bytes": _integer(values[1], "source bytes read", minimum_read_bytes,
                                       2 * frame.MAX_SOURCE_BYTES),
        "max_request_bytes": _integer(values[2], "source request", 1, MAX_NATIVE_REQUEST_BYTES),
        "max_text_owned_buffer_bytes": _integer(values[3], "maximum text owned", 1,
                                                 MAX_NATIVE_WORKING_BYTES),
        "max_text_working_memory_bytes": _integer(values[4], "maximum text working", 1,
                                                   MAX_NATIVE_WORKING_BYTES),
        "temporary_bytes": _integer(values[5], "native temporary bytes", 0, 0),
    }
    if (resources["max_request_bytes"] < max(page["max_source_request_bytes"] for page in pages) or
            resources["max_text_owned_buffer_bytes"] != max(page["owned_buffer_bytes"] for page in pages) or
            resources["max_text_working_memory_bytes"] != max(page["working_memory_bytes"] for page in pages)):
        raise RuleError("native resource summary disagrees with its page records")
    if cursor != len(lines):
        raise RuleError("native TSV has trailing or duplicate records")
    return {"variant": header[1], "page_count": page_count, "draw_count": image_total,
            "pages": pages, "resources": resources}


def source_fingerprint(root: Path = shared.ROOT) -> dict[str, Any]:
    """Pin original Rust sources and Cargo inputs, independently of this script."""
    root = root.resolve()
    files = {root / name for name in ("Cargo.toml", "Cargo.lock", "crates/caj2pdf-core/Cargo.toml")}
    for pattern in ("*.rs", "Cargo.toml"):
        for path in (root / "crates").rglob(pattern):
            files.add(path)
            if len(files) > MAX_SOURCE_FILES:
                raise RuleError("Rust/Cargo provenance file count exceeds its bounded manifest")
    if (root / "rust-toolchain.toml").exists():
        files.add(root / "rust-toolchain.toml")
    if not files or len(files) > MAX_SOURCE_FILES:
        raise RuleError("Rust/Cargo provenance file count exceeds its bounded manifest")
    digest = hashlib.sha256()
    records = []
    total = 0
    for path in sorted(files, key=lambda value: value.relative_to(root).as_posix()):
        resolved = path.resolve(strict=True)
        if path.is_symlink() or not resolved.is_relative_to(root):
            raise RuleError("Rust/Cargo provenance path escapes the original repository")
        sha, size = pdf._file_hash(resolved, MAX_SOURCE_FILE_BYTES)
        total += size
        if total > MAX_SOURCE_TREE_BYTES:
            raise RuleError("Rust/Cargo provenance bytes exceed the tree cap")
        name = path.relative_to(root).as_posix()
        digest.update(name.encode("utf-8") + b"\x00" + bytes.fromhex(sha))
        records.append({"path": name, "sha256": sha, "size_bytes": size})
    return {"sha256": digest.hexdigest(), "file_count": len(records), "total_bytes": total,
            "algorithm": "sorted UTF-8 relative path + NUL + raw per-file SHA-256",
            "files": records}


def _environment_pin() -> dict[str, Any]:
    effective = dict(os.environ, LC_ALL="C", TZ="UTC")
    digest = hashlib.sha256(json.dumps(effective, sort_keys=True, separators=(",", ":"),
                                       ensure_ascii=True).encode("ascii")).hexdigest()
    return {"sha256": digest, "variable_names": sorted(effective),
            "overrides": {"LC_ALL": "C", "TZ": "UTC"}}


def _inspect_frame(source: Path, profile: str, page: dict, resources: dict) -> dict:
    coordinates = []

    def collect(spool: Any, metadata: frame.FrameMetadata) -> None:
        spool.seek(metadata.trailing_records.first_offset)
        for _ in range(metadata.declared_image_count):
            record = frame._read_exact(spool, 28)
            coordinates.append(list(struct.unpack_from("<HH", record)))

    with source.open("rb") as stream:
        stream.seek(page["text_offset"])
        digest = hashlib.sha256()
        remaining = page["text_length"]
        while remaining:
            block = frame._read_exact(stream, min(remaining, frame.IO_CHUNK))
            digest.update(block)
            remaining -= len(block)
        if digest.hexdigest() != page["text_sha256"]:
            raise RuleError("original page-text SHA-256 differs from source oracle")
        metadata = frame.inspect_frame(
            stream, offset=page["text_offset"], length=page["text_length"],
            image_count=len(page["images"]), profile=frame.PROFILES[profile],
            decoded_callback=collect)
    resources["max_frame_source_request_bytes"] = max(
        resources["max_frame_source_request_bytes"], metadata.max_source_read_request_bytes,
        min(page["text_length"], frame.IO_CHUNK))
    resources["max_frame_decoder_output_chunk_bytes"] = max(
        resources["max_frame_decoder_output_chunk_bytes"], metadata.max_decoder_output_chunk_bytes)
    resources["max_frame_decoded_spool_bytes"] = max(
        resources["max_frame_decoded_spool_bytes"], metadata.decoded_spool_bytes)
    resources["total_frame_decoded_spool_bytes"] += metadata.decoded_spool_bytes
    return {"encoded_sha256": metadata.zlib_stream_sha256,
            "decoded_sha256": metadata.decoded_sha256,
            "decoded_length": metadata.decoded_length, "record_count": metadata.record_count,
            "coordinates": coordinates}


def _same_numbers(actual: list, expected: list, size: int) -> bool:
    if type(actual) is not list or type(expected) is not list or len(actual) != size or len(expected) != size:
        raise RuleError("geometry has an unexpected component count")
    left = [_number(value, "native geometry") for value in actual]
    right = [_number(value, "reference geometry") for value in expected]
    return all(abs(a - b) <= TOLERANCE_PT for a, b in zip(left, right))


def _count(counts: dict, kind: str, passing: bool) -> None:
    counts[f"{kind}_attempted"] += 1
    counts[f"{kind}_{'passing' if passing else 'failing'}"] += 1


def compare_native(native: dict, case: dict, profile: str, source: Path,
                   counts: dict, resources: dict) -> dict:
    """Check ordered identities and independently measured frame/geometry facts."""
    source_pages, pdf_pages = case["source_pages"], case["pdf_pages"]
    if (native["variant"] != case["source_variant"] or
            native["page_count"] != len(source_pages) or len(pdf_pages) != len(source_pages) or
            len(native["pages"]) != len(source_pages) or
            case["output_page_to_source_page"] != list(range(1, len(source_pages) + 1)) or
            native["draw_count"] != sum(len(page["images"]) for page in source_pages)):
        raise RuleError("native variant, page mapping or ordered totals differ from source oracle")
    summaries = []
    all_passing = True
    for actual, expected_source, expected_pdf in zip(native["pages"], source_pages, pdf_pages):
        page_number = actual["page_number"]
        if (page_number != _integer(expected_source["page_number"], "oracle source page", 1) or
                page_number != _integer(expected_pdf["page_number"], "oracle PDF page", 1) or
                actual["text_offset"] != _integer(expected_source["text_offset"], "oracle text offset") or
                actual["text_length"] != _integer(expected_source["text_length"], "oracle text length", 1) or
                actual["image_count"] != len(expected_source["images"]) or
                actual["image_count"] != len(expected_pdf["draws"])):
            raise RuleError("native page identity, spans or ordered counts differ from source oracle")
        independent = _inspect_frame(source, profile, expected_source, resources)
        if (any(actual[key] != independent[key] for key in (
                "encoded_sha256", "decoded_sha256", "decoded_length", "record_count")) or
                len(independent["coordinates"]) != actual["image_count"]):
            raise RuleError("native text frame metadata differs from independent bounded parser")
        counts["source_frames_compared"] += 1
        box_pass = _same_numbers(actual["media_box"], expected_pdf["media_box"], 4)
        _count(counts, "page_boxes", box_pass)
        page_result = {"page_number": page_number, "media_box": actual["media_box"],
                       "reference_media_box": expected_pdf["media_box"], "box_passing": box_pass,
                       "frame": independent, "draws": []}
        all_passing &= box_pass
        for image, source_image, draw, coordinate in zip(
                actual["images"], expected_source["images"], expected_pdf["draws"],
                independent["coordinates"]):
            integer_keys = ("image_number", "record_type", "descriptor_offset", "payload_offset",
                            "payload_length", "width", "height")
            if (any(image[key] != _integer(source_image[key], f"oracle {key}") for key in integer_keys) or
                    image["payload_sha256"] != _sha(source_image["payload_sha256"], "oracle payload SHA") or
                    image["display_width"] != _integer(source_image.get("stride_width", source_image["width"]),
                                                       "oracle display width", 1) or
                    coordinate != [image["x_word"], image["y_word"]] or
                    image["image_number"] != _integer(draw["draw_number"], "oracle draw number", 1) or
                    image["display_width"] != _integer(draw["width"], "oracle PDF width", 1) or
                    image["height"] != _integer(draw["height"], "oracle PDF height", 1)):
                raise RuleError("native image identity, dimensions, hashes or raw words differ")
            if image["record_type"] == 2 and image["payload_sha256"] != draw["raw_stream_sha256"]:
                raise RuleError("type-2 original payload hash differs from reference PDF stream")
            counts["source_images_compared"] += 1
            if image["image_number"] == 1:
                group = "first_draws"
            else:
                groups = [mode for mode, pages in analysis.PAGE_SPLITS.items()
                          if page_number in pages[profile]]
                if len(groups) != 1 or image["record_type"] != 2:
                    raise RuleError("supplemental draw is outside the predeclared type-2 split")
                group = f"{groups[0]}_draws"
            passing = _same_numbers(image["pdf_ctm"], draw["pdf_ctm"], 6)
            _count(counts, group, passing)
            all_passing &= passing
            page_result["draws"].append({**image, "reference_pdf_ctm": draw["pdf_ctm"],
                                          "group": group, "passing": passing})
        summaries.append(page_result)
    return {"profile": profile, "source_id": case["source_id"],
            "status": "PASS" if all_passing else "FAIL", "pages": summaries,
            "native_resources": native["resources"]}


def _report() -> dict[str, Any]:
    return {
        "schema_version": 1, "protocol": "hnc8-source-placement-rule-v1", "status": "NOT_RUN",
        "placement_rule_status": "UNKNOWN_NOT_TESTED",
        "scope": "HN-A/C8 empirical profile; same documents with previously inspected public reference values",
        "tolerance_pt": TOLERANCE_PT, "production_composition": "NOT_ENABLED",
        "planned": {"profiles": 2, "source_audit_rows": 27, "baseline_pdf_files": 6,
                    "unsupported_hn_b_source_rows": 6, **EXPECTED_COMPARISONS},
        "counts": {**{name: 0 for name in (
            "profiles_attempted", "profiles_completed", "profiles_passing", "profiles_failing",
            "profiles_skipped", "unsupported_source_rows", "private_source_checks_before",
            "private_source_checks_after", "baseline_pdf_checks_before", "baseline_pdf_checks_after",
            "baseline_pdf_metadata_compared", "native_launches", "native_processes_completed", "converter_launches",
            "source_frames_compared", "source_images_compared")},
            **{f"{kind}_{outcome}": 0 for kind in EXPECTED_COMPARISONS
               for outcome in ("attempted", "passing", "failing", "skipped")}},
        "source_audit": {"status": "NOT_RUN"}, "environment_audit": {"status": "NOT_RUN"},
        "input_audit": {"status": "NOT_RUN"}, "native_audit": {"status": "NOT_RUN"},
        "profiles": [], "unsupported_profiles": [], "errors": [],
        "resources": {**{name: 0 for name in (
            "max_source_hash_read_request_bytes", "max_native_provenance_hash_read_request_bytes",
            "max_native_output_bytes", "total_native_output_bytes",
            "max_native_child_rss_kib", "max_native_source_request_bytes", "native_source_read_bytes",
            "max_native_owned_buffer_bytes", "max_native_working_memory_bytes", "native_temporary_bytes",
            "max_frame_source_request_bytes", "max_frame_decoder_output_chunk_bytes",
            "max_frame_decoded_spool_bytes", "total_frame_decoded_spool_bytes",
            "retained_frame_spool_bytes", "max_pdf_tool_output_bytes", "max_pdf_tool_child_rss_kib",
            "harness_vmhwm_kib")},
            "native_stdout_limit_bytes": MAX_OUTPUT_BYTES, "native_timeout_seconds": NATIVE_TIMEOUT_SECONDS,
            "native_child_virtual_limit_bytes": reference.MAX_CHILD_VIRTUAL_BYTES,
            "native_process_resource_scope": "stdout totals and child RSS cover successful subprocesses; failed-process RSS is unavailable",
            "scope": "native reported parser storage, child RSS, independent decoded spool and harness RSS are distinct"},
    }


def run(paths: Mapping[str, Path] | None = None, reference_report: Path | None = None,
        *, native_sha256: str | None = None, native_source_sha256: str | None = None) -> dict[str, Any]:
    report = _report()
    if paths is None and reference_report is None and native_sha256 is None and native_source_sha256 is None:
        return report
    rows = before_sources = before_environment = before_inputs = before_native = None
    resolved = baseline_pdfs = None
    native_usage = pdf._Usage()
    try:
        if (paths is None or set(paths) != REQUIRED_PATHS or reference_report is None or
                native_sha256 is None or native_source_sha256 is None):
            raise RuleError("all external paths, reference report and native binary/source SHA-256 pins are required together")
        native_sha256 = _sha(native_sha256, "requested native SHA-256")
        native_source_sha256 = _sha(native_source_sha256, "requested Rust/Cargo SHA-256")
        resolved = {name: Path(path).expanduser().resolve() for name, path in paths.items()}
        resolved["native_tool"] = pdf._tool_path(resolved["native_tool"], "native placement tool")
        source_record = source_fingerprint()
        binary_sha, binary_size = pdf._file_hash(resolved["native_tool"], MAX_NATIVE_BYTES)
        before_native = {"binary_sha256": binary_sha, "binary_size_bytes": binary_size,
                         "source": source_record, "environment": _environment_pin()}
        report["native_audit"] = {"status": "BEFORE_PASS", "path": str(resolved["native_tool"]),
                                  **before_native, "invocation": ["native_tool", "SOURCE_PATH"],
                                  "expected_binary_sha256": native_sha256,
                                  "expected_source_sha256": native_source_sha256,
                                  "timeout_seconds": NATIVE_TIMEOUT_SECONDS}
        report["resources"]["max_native_provenance_hash_read_request_bytes"] = 65536
        if binary_sha != native_sha256 or source_record["sha256"] != native_source_sha256:
            raise RuleError("native binary or Rust/Cargo source differs from its requested provenance pin")
        roots = (shared.ROOT.resolve(), resolved["corpus"], resolved["reference_repo"])
        matrix_sha, rows = reference.load_source_rows()
        report["matrix_sha256"] = matrix_sha
        oracle = shared._load_oracle()
        before_sources = reference.audit_sources(resolved["corpus"], rows)
        report["source_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before_sources),
                                  "after_checked": 0, "sources": before_sources}
        report["counts"]["private_source_checks_before"] = len(before_sources)
        # reference.sha256_file always requests this chunk, including at EOF.
        report["resources"]["max_source_hash_read_request_bytes"] = reference.READ_CHUNK
        before_environment = reference.audit_environment(resolved)
        report["environment_audit"] = shared._environment_record(before_environment, "BEFORE_PASS")
        reference_report = shared._external_file(Path(reference_report), roots, "reference report")
        baseline_report, baseline_pdfs = shared._read_reference_report(
            reference_report, rows, before_environment, before_sources, roots)
        before_inputs = {"matrix": shared._file_digest(reference.MATRIX),
                         "oracle": shared._file_digest(shared.ORACLE),
                         "reference_report": shared._file_digest(reference_report),
                         **{key: shared._file_digest(path) for key, path in baseline_pdfs.items()}}
        report["input_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before_inputs),
                                 "after_checked": 0, "sha256": before_inputs}
        report["counts"]["baseline_pdf_checks_before"] = len(baseline_pdfs)
        profiles = {profile.name: profile for profile in reference.PROFILES}
        for name in SUPPORTED_PROFILES:
            profile = profiles[name]
            parsed_pdf = reference.pdf_metadata(baseline_pdfs[f"{name}-run1"], resolved)
            if (parsed_pdf["pdf_sha256"] != profile.expected_pdf_sha256 or
                    parsed_pdf["page_count"] != profile.expected_pages or
                    parsed_pdf["draw_count"] != profile.expected_draws or
                    parsed_pdf["pages"] != oracle[name]["pdf_pages"] or
                    any(parsed_pdf["tools"][tool]["sha256"] != reference.PINNED_HASHES[tool]
                        for tool in ("qpdf", "mutool", "pdfimages"))):
                raise RuleError("pinned PDF cross-check differs from #107 ordered metadata")
            report["counts"]["baseline_pdf_metadata_compared"] += 1
            report["resources"]["max_pdf_tool_output_bytes"] = max(
                report["resources"]["max_pdf_tool_output_bytes"], parsed_pdf["resources"]["max_tool_output_bytes"])
            report["resources"]["max_pdf_tool_child_rss_kib"] = max(
                report["resources"]["max_pdf_tool_child_rss_kib"], parsed_pdf["resources"]["max_child_rss_kib"])
        if len(oracle["hn_b"]["source_pages"]) != 6:
            raise RuleError("unsupported HN-B source-row accounting differs from its six-row pin")
        report["counts"]["unsupported_source_rows"] = 6
        report["unsupported_profiles"] = [{"profile": "hn_b", "source_id": oracle["hn_b"]["source_id"],
                                           "source_rows": 6, "reason": "Different text framing; not executed"}]
        source_rows = {row["id"]: row for row in rows}
        native_limits = pdf.PdfMetadataLimits(timeout_seconds=NATIVE_TIMEOUT_SECONDS,
                                               max_child_virtual_bytes=reference.MAX_CHILD_VIRTUAL_BYTES)
        for name in SUPPORTED_PROFILES:
            profile = profiles[name]
            source = (resolved["corpus"] / source_rows[profile.source_id]["path"]).resolve(strict=True)
            if not source.is_relative_to(resolved["corpus"]):
                raise RuleError("native source escaped the audited corpus")
            command = [str(resolved["native_tool"]), str(source)]
            report["counts"]["profiles_attempted"] += 1
            report["counts"]["native_launches"] += 1
            native = data = None
            try:
                data, output_bytes = pdf._run(command, "native placement tool", native_limits,
                                               native_usage, MAX_OUTPUT_BYTES)
                report["counts"]["native_processes_completed"] += 1
                native = parse_native_tsv(data, source_rows[profile.source_id]["size_bytes"])
                report["counts"]["profiles_completed"] += 1
                resources = native["resources"]
                report["resources"]["native_source_read_bytes"] += resources["source_read_bytes"]
                for output_key, native_key in (
                    ("max_native_source_request_bytes", "max_request_bytes"),
                    ("max_native_owned_buffer_bytes", "max_text_owned_buffer_bytes"),
                    ("max_native_working_memory_bytes", "max_text_working_memory_bytes")):
                    report["resources"][output_key] = max(report["resources"][output_key], resources[native_key])
                result = compare_native(native, oracle[name], name, source,
                                        report["counts"], report["resources"])
                result.update({"command": command, "output_bytes": output_bytes,
                               "output_sha256": hashlib.sha256(data).hexdigest(),
                               "source_sha256": source_rows[profile.source_id]["sha256"]})
                report["profiles"].append(result)
                report["counts"][f"profiles_{'passing' if result['status'] == 'PASS' else 'failing'}"] += 1
                if result["status"] != "PASS":
                    report["errors"].append(f"{name}: source-derived geometry differs from reference")
            except (RuleError, frame.FrameError, pdf.PdfMetadataError, OSError, KeyError, TypeError, ValueError) as exc:
                report["counts"]["profiles_failing"] += 1
                failed = {"profile": name, "status": "FAIL", "command": command,
                          "error_kind": type(exc).__name__, "error": str(exc)}
                if data is not None:
                    failed.update(output_bytes=len(data), output_sha256=hashlib.sha256(data).hexdigest())
                if native is not None:
                    failed["native_metadata"] = native
                report["profiles"].append(failed)
                report["errors"].append(f"{name}: {exc}")
        for kind, expected in EXPECTED_COMPARISONS.items():
            if report["counts"][f"{kind}_attempted"] != expected:
                report["errors"].append(f"{kind}: incomplete ordered comparison count")
        report["baseline_reference_report_status"] = baseline_report["status"]
        report["status"] = "FAIL" if report["errors"] else "PASS"
        report["placement_rule_status"] = ("EMPIRICAL_PROFILE_VALIDATED_SAME_DOCUMENTS"
                                            if not report["errors"] else "UNKNOWN_FAILED_RULE_GATE")
    except (RuleError, reference.ReferenceError, shared.ProbeError, pdf.PdfMetadataError,
            frame.FrameError, OSError, KeyError, TypeError, ValueError) as exc:
        report["status"] = "FAIL"
        report["placement_rule_status"] = "UNKNOWN_FAILED_RULE_GATE"
        report["errors"].append(str(exc))
    finally:
        report["counts"]["profiles_skipped"] = max(0, 2 - report["counts"]["profiles_attempted"])
        for kind, expected in EXPECTED_COMPARISONS.items():
            report["counts"][f"{kind}_skipped"] = max(0, expected - report["counts"][f"{kind}_attempted"])
        if resolved is not None and rows is not None and before_sources is not None:
            try:
                after_sources = reference.audit_sources(resolved["corpus"], rows)
                if after_sources != before_sources:
                    raise RuleError("original source set changed between audits")
                report["source_audit"]["status"] = "PASS"
                report["source_audit"]["after_checked"] = len(after_sources)
                report["counts"]["private_source_checks_after"] = len(after_sources)
            except (reference.ReferenceError, RuleError, OSError) as exc:
                report["source_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run source audit: {exc}")
        if resolved is not None and before_environment is not None:
            try:
                if reference.audit_environment(resolved) != before_environment:
                    raise RuleError("pinned reference environment changed between audits")
                report["environment_audit"]["status"] = "PASS"
            except (reference.ReferenceError, RuleError, OSError) as exc:
                report["environment_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run environment audit: {exc}")
        if before_inputs is not None:
            try:
                after_inputs = {"matrix": shared._file_digest(reference.MATRIX),
                                "oracle": shared._file_digest(shared.ORACLE),
                                "reference_report": shared._file_digest(reference_report),
                                **{key: shared._file_digest(path) for key, path in baseline_pdfs.items()}}
                if after_inputs != before_inputs:
                    raise RuleError("matrix/oracle/reference report or baseline PDF changed between audits")
                report["input_audit"]["status"] = "PASS"
                report["input_audit"]["after_checked"] = len(after_inputs)
                report["counts"]["baseline_pdf_checks_after"] = len(baseline_pdfs)
            except (RuleError, shared.ProbeError, OSError) as exc:
                report["input_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run input audit: {exc}")
        if before_native is not None:
            try:
                sha, size = pdf._file_hash(resolved["native_tool"], MAX_NATIVE_BYTES)
                after_native = {"binary_sha256": sha, "binary_size_bytes": size,
                                "source": source_fingerprint(), "environment": _environment_pin()}
                if after_native != before_native:
                    raise RuleError("native executable, Rust/Cargo inputs or inherited environment changed")
                if sha != native_sha256 or after_native["source"]["sha256"] != native_source_sha256:
                    raise RuleError("native requested provenance pin failed after verification")
                report["native_audit"]["status"] = "PASS"
                report["native_audit"]["after_binary_sha256"] = sha
                report["native_audit"]["after_source_sha256"] = after_native["source"]["sha256"]
            except (RuleError, pdf.PdfMetadataError, OSError) as exc:
                report["native_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run native audit: {exc}")
        report["resources"]["max_native_output_bytes"] = native_usage.max_tool_output_bytes
        report["resources"]["total_native_output_bytes"] = native_usage.total_tool_output_bytes
        report["resources"]["max_native_child_rss_kib"] = native_usage.max_child_rss_kib
        report["resources"]["harness_vmhwm_kib"] = reference._read_rss()["harness_vmhwm_kib"]
        if report["errors"]:
            report["status"] = "FAIL"
            report["placement_rule_status"] = "UNKNOWN_FAILED_RULE_GATE"
    return report


def _paths(args: argparse.Namespace) -> Mapping[str, Path] | None:
    external = {"corpus": args.corpus_dir, "reference_repo": args.reference_repo,
                "python": args.python_bin, "pydeps": args.pydeps_dir, "libjbigdec": args.jbig_lib,
                "native_tool": args.native_tool}
    requested = (*external.values(), args.reference_report, args.native_sha256,
                 args.native_source_sha256, args.git, args.qpdf, args.mutool, args.pdfinfo, args.pdfimages)
    if all(value is None for value in requested):
        return None
    if any(value is None for value in (*external.values(), args.reference_report,
                                     args.native_sha256, args.native_source_sha256)):
        raise RuleError("all external path and native pin flags are required when any one is supplied")
    for name in ("git", "qpdf", "mutool", "pdfinfo", "pdfimages"):
        value = getattr(args, name) or shutil.which(name)
        if value is None:
            raise RuleError(f"required {name} tool is unavailable")
        external[name] = Path(value)
    return external


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("corpus-dir", "reference-repo", "python-bin", "pydeps-dir", "jbig-lib",
                 "reference-report", "native-tool", "git", "qpdf", "mutool", "pdfinfo", "pdfimages"):
        parser.add_argument(f"--{name}", type=Path)
    parser.add_argument("--native-sha256")
    parser.add_argument("--native-source-sha256")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = run(_paths(args), args.reference_report, native_sha256=args.native_sha256,
                     native_source_sha256=args.native_source_sha256)
    except RuleError as exc:
        report = run({}, args.reference_report, native_sha256=args.native_sha256,
                     native_source_sha256=args.native_source_sha256)
        report["errors"] = [str(exc)]
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        print(f"HN-A/C8 source placement [{report['status']}]: "
              f"{report['counts']['profiles_passing']}/2 profiles; {report['placement_rule_status']}")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())
