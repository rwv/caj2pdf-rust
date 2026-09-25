#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, black-box pixel oracle for the HN/C8 JBIG2 generic region alone.

Only original page-information segment 0 and generic-region segment 4 enter
each temporary PDF. The repository's full-image oracle supplies source,
directory, PDF, PBM, and tool helpers; this script adds no container parser or
decoder. Only hashes and metadata may be committed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from typing import BinaryIO, NamedTuple

import conformance
import jbig2_oracle as full


DEFAULT_MANIFEST = full.ROOT / "tests/conformance/jbig2_generic_oracle.json"
SCHEMA_VERSION = 1
ORACLE_KIND = "page-information-and-generic-region-only"
NORMALIZATION = "P4 PBM, top-to-bottom rows, MSB-first, 1=black, low row-padding bits zero"
CORPUS_REVISION = "7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07"
EXPECTED_IMAGES = full.EXPECTED_IMAGES
MAX_RECORD_BYTES = full.MAX_ENCODED_BYTES
SPOTS = {
    ("issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj", 2, 1):
        ("72170496b556f7628b436b8e924e9bc4aa2815dc8d31106ab64e8ea0dbecde8f", 94_550),
    ("issue-58/混凝土道面评价指标分析_谢永亮.caj", 1, 1):
        ("dae0fec2ea4c15de4b70f590a6bb3629f8bf17c225f0d0d4427743a04084fcb6", 393_170),
}


class OracleError(full.OracleError):
    """An invalid generic-only span, source, tool result, or manifest."""


class SegmentSlice(NamedTuple):
    number: int
    offset: int
    length: int


def selected_spans(case: dict) -> tuple[SegmentSlice, SegmentSlice]:
    """Check exact header-plus-data ranges supplied by the Rust inventory."""
    image_start = case["offset"] + 48
    image_end = case["offset"] + case["length"]
    result = []
    for number in (0, 4):
        segment = case["segments"][number]
        if segment.get("number") != number or segment.get("type") != (48 if number == 0 else 38):
            raise OracleError(f"segment {number} type or number differs from the pinned profile")
        header_length = full._positive_int(segment.get("header_length"), "segment header length")
        data_offset = full._nonnegative_int(segment.get("data_offset"), "segment data offset")
        data_length = full._nonnegative_int(segment.get("data_length"), "segment data length")
        start = data_offset - header_length
        length = header_length + data_length
        if start < image_start or start > image_end or length > image_end - start:
            raise OracleError(f"segment {number} header or data escapes the type-3 image")
        result.append(SegmentSlice(number, start, length))
    page, generic = result
    if page.offset != image_start or page.offset + page.length > generic.offset:
        raise OracleError("page-information segment does not start the embedded image")
    if generic.offset + generic.length != image_end:
        raise OracleError("generic-region segment does not end at the image boundary")
    if 48 + page.length + generic.length > MAX_RECORD_BYTES:
        raise OracleError("generic-only temporary record exceeds the bounded image limit")
    return page, generic


def checked_profile(case: dict) -> dict:
    """Reuse the #43 DIB, page-information, and generic T.88 profile checks."""
    profile, _ = full.profile_and_hash(case)
    if profile["page_flags"] != 1 or profile["generic_flags"] != 4:
        raise OracleError("page or generic flags differ from arithmetic template-2 profile")
    # profile_and_hash also checks the signed AT bytes 0x02,0xff and full-page
    # region dimensions. T.88 §7.4.6: 0x04 is MMR=0, template=2, TPGDON=0.
    return {
        "page_flags": 1,
        "generic_flags": 4,
        "template": 2,
        "typical_prediction": False,
        "adaptive_pixel": [2, -1],
    }


def prehash_selected(case: dict, spans: tuple[SegmentSlice, SegmentSlice]) -> tuple[str, dict[int, str]]:
    with case["source_path"].open("rb") as source:
        wrapper_sha = full.sha256_span(source, case["offset"], 48)
        hashes = {
            span.number: full.sha256_span(source, span.offset, span.length)
            for span in spans
        }
    return wrapper_sha, hashes


def copy_verified(
    source: BinaryIO, output: BinaryIO, offset: int, length: int,
    expected_sha: str, record_hash: object,
) -> None:
    source.seek(offset)
    copied_hash = hashlib.sha256()
    remaining = length
    while remaining:
        chunk = source.read(min(remaining, full.COPY_CHUNK))
        if not chunk:
            raise OracleError(f"source changed or ended while copying byte {offset + length - remaining}")
        output.write(chunk)
        copied_hash.update(chunk)
        record_hash.update(chunk)
        remaining -= len(chunk)
    if copied_hash.hexdigest() != expected_sha:
        raise OracleError(f"source span {offset}+{length} changed between hash and spool")


def spool_record(
    case: dict, spans: tuple[SegmentSlice, SegmentSlice], wrapper_sha: str,
    segment_hashes: dict[int, str], destination: Path,
) -> tuple[int, str]:
    """Write DIB plus only segment 0 and 4, checking each copied source span."""
    record_hash = hashlib.sha256()
    with case["source_path"].open("rb") as source, destination.open("wb") as output:
        copy_verified(source, output, case["offset"], 48, wrapper_sha, record_hash)
        for span in spans:
            copy_verified(
                source, output, span.offset, span.length,
                segment_hashes[span.number], record_hash,
            )
    expected_size = 48 + sum(span.length for span in spans)
    if destination.stat().st_size != expected_size:
        raise OracleError("generic-only temporary record has an unexpected size")
    return expected_size, record_hash.hexdigest()


def decode_generic_case(
    case: dict, spans: tuple[SegmentSlice, SegmentSlice], tools: dict[str, Path],
) -> tuple[dict[int, str], str, int, int, int]:
    """Keep the selected record and the #43 PDF/PBMs within per-image temps."""
    wrapper_sha, segment_hashes = prehash_selected(case, spans)
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-generic-") as directory:
        record = Path(directory) / "generic-record.caj"
        record_bytes, record_sha = spool_record(
            case, spans, wrapper_sha, segment_hashes, record
        )
        temporary_case = {
            "source_path": record, "offset": 0, "length": record_bytes,
            "width": case["width"], "height": case["height"],
        }
        pixel_sha, black_pixels, pdf_bytes = full.decode_case(
            temporary_case, tools, record_sha
        )
    return segment_hashes, pixel_sha, black_pixels, record_bytes, pdf_bytes


def location(case: dict) -> dict:
    """Always include both selected source spans in a case failure."""
    segments = []
    for number in (0, 4):
        segment = case["segments"][number]
        header = segment.get("header_length")
        data_offset = segment.get("data_offset")
        data_length = segment.get("data_length")
        segments.append({
            "number": number,
            "offset": data_offset - header if type(header) is int and type(data_offset) is int else None,
            "length": header + data_length if type(header) is int and type(data_length) is int else None,
        })
    return {
        "id": case["id"], "page": case["page"], "image": case["image"],
        "offset": case["offset"], "length": case["length"], "segments": segments,
    }


def manifest_for_cases(
    cases: list[dict], tools: dict[str, Path], matrix_path: Path,
) -> tuple[dict, dict]:
    samples: dict[str, dict] = {}
    failures = []
    max_record_bytes = 0
    max_pdf_bytes = 0
    max_case_seconds = 0.0
    started = time.monotonic()
    toolchains = full.tool_metadata(tools)
    for number, case in enumerate(cases, 1):
        sample = samples.setdefault(case["id"], {
            "id": case["id"], "path": case["path"],
            "variant": case["variant"], "source_sha256": case["source_sha256"],
            "images": [],
        })
        case_started = time.monotonic()
        try:
            spans = selected_spans(case)
            profile = checked_profile(case)
            hashes, pixels, black, record_bytes, pdf_bytes = decode_generic_case(case, spans, tools)
            if case["coordinate"] in SPOTS and (pixels, black) != SPOTS[case["coordinate"]]:
                raise OracleError("generic-only pixels differ from the independently measured spot")
            sample["images"].append({
                "page": case["page"], "image": case["image"],
                "offset": case["offset"], "length": case["length"],
                "width": case["width"], "height": case["height"],
                "segment_0": {
                    "number": 0, "offset": spans[0].offset, "length": spans[0].length,
                    "encoded_sha256": hashes[0],
                },
                "segment_4": {
                    "number": 4, "offset": spans[1].offset, "length": spans[1].length,
                    "encoded_sha256": hashes[4],
                },
                "generic_profile": profile,
                "normalized_pixel_sha256": pixels, "black_pixels": black,
                "status": "PASS",
            })
            max_record_bytes = max(max_record_bytes, record_bytes)
            max_pdf_bytes = max(max_pdf_bytes, pdf_bytes)
        except (OSError, full.OracleError) as exc:
            failures.append({**location(case), "error": str(exc), "status": "FAIL"})
        max_case_seconds = max(max_case_seconds, time.monotonic() - case_started)
        if number % 50 == 0 or number == len(cases):
            print(
                f"Generic-only oracle progress: {number}/{len(cases)}; failures={len(failures)}",
                file=sys.stderr, flush=True,
            )
    agreements = sum(len(sample["images"]) for sample in samples.values())
    manifest = {
        "schema_version": SCHEMA_VERSION, "oracle_kind": ORACLE_KIND,
        "matrix_sha256": full.sha256_file(matrix_path),
        "corpus_revision": CORPUS_REVISION,
        "normalization": NORMALIZATION,
        "backend_independence": "UNVERIFIED", "toolchains": toolchains,
        "samples": [samples[key] for key in sorted(samples)],
    }
    report = {
        "status": "PASS" if not failures and agreements == EXPECTED_IMAGES else "FAIL",
        "expected_images": EXPECTED_IMAGES, "checked_images": len(cases),
        "tool_agreements": agreements, "failures": failures,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "max_case_seconds": round(max_case_seconds, 3),
        "max_temp_record_bytes": max_record_bytes,
        "max_temp_pdf_bytes": max_pdf_bytes,
        "max_single_pbm_raster_bytes": max((case["width"] + 7) // 8 * case["height"] for case in cases),
        "backend_independence": "UNVERIFIED", "toolchains": toolchains,
    }
    return manifest, report


def require_keys(value: object, allowed: set[str], label: str) -> dict:
    if not isinstance(value, dict):
        raise OracleError(f"{label} must be an object")
    full.reject_unknown_fields(value, allowed, label)
    return value


def valid_sha(value: object, label: str) -> str:
    if not isinstance(value, str) or not full.HEX_SHA256.fullmatch(value):
        raise OracleError(f"{label} must be a SHA-256 hex digest")
    return value


def validate_toolchains(value: object) -> None:
    chains = require_keys(value, {"tools", "backend_evidence"}, "toolchains")
    tools = require_keys(chains.get("tools"), {"qpdf", "pdfimages", "mutool"}, "toolchains.tools")
    if set(tools) != {"qpdf", "pdfimages", "mutool"}:
        raise OracleError("generic oracle manifest lacks three tool identities")
    for name, tool in tools.items():
        tool = require_keys(tool, {"version", "binary_sha256"}, f"toolchains.tools.{name}")
        if not isinstance(tool.get("version"), str) or not tool["version"]:
            raise OracleError(f"toolchains.tools.{name} has no version")
        valid_sha(tool.get("binary_sha256"), f"toolchains.tools.{name}.binary_sha256")
    evidence = require_keys(
        chains.get("backend_evidence"),
        {"dynamic_libjbig2dec", "implementation_independence", "meaning"},
        "toolchains.backend_evidence",
    )
    if evidence.get("implementation_independence") != "UNVERIFIED":
        raise OracleError("generic oracle overstates decoder implementation independence")
    if not isinstance(evidence.get("meaning"), str) or not evidence["meaning"]:
        raise OracleError("toolchains.backend_evidence lacks a meaning")
    dynamic = require_keys(
        evidence.get("dynamic_libjbig2dec"), {"mutool", "pdfimages"},
        "toolchains.backend_evidence.dynamic_libjbig2dec",
    )
    if any(type(linked) not in (bool, type(None)) for linked in dynamic.values()):
        raise OracleError("dynamic decoder linkage must be true, false, or unknown")


def validate_manifest(manifest: object, expected_images: int = EXPECTED_IMAGES) -> None:
    manifest = require_keys(
        manifest,
        {"schema_version", "oracle_kind", "matrix_sha256", "corpus_revision",
         "normalization", "backend_independence", "toolchains", "samples"},
        "generic oracle manifest",
    )
    if manifest.get("schema_version") != SCHEMA_VERSION or manifest.get("oracle_kind") != ORACLE_KIND:
        raise OracleError("generic oracle manifest schema or kind differs")
    if manifest.get("corpus_revision") != CORPUS_REVISION or manifest.get("normalization") != NORMALIZATION:
        raise OracleError("generic oracle manifest corpus revision or pixel layout differs")
    if manifest.get("backend_independence") != "UNVERIFIED":
        raise OracleError("generic oracle overstates decoder implementation independence")
    valid_sha(manifest.get("matrix_sha256"), "matrix_sha256")
    if expected_images == EXPECTED_IMAGES and manifest["matrix_sha256"] != full.sha256_file(full.DEFAULT_MATRIX):
        raise OracleError("generic oracle manifest matrix SHA-256 differs")
    validate_toolchains(manifest.get("toolchains"))
    samples = manifest.get("samples")
    if not isinstance(samples, list):
        raise OracleError("generic oracle manifest samples must be a list")
    rows = {
        row["id"]: row for row in conformance.load_matrix(full.DEFAULT_MATRIX)
    } if expected_images == EXPECTED_IMAGES else {}
    sample_ids = set()
    count = 0
    for sample in samples:
        sample = require_keys(
            sample, {"id", "path", "variant", "source_sha256", "images"},
            "generic oracle sample",
        )
        sample_id = sample.get("id")
        if not isinstance(sample_id, str) or sample_id in sample_ids:
            raise OracleError("generic oracle has duplicate or invalid source ID")
        sample_ids.add(sample_id)
        valid_sha(sample.get("source_sha256"), "source_sha256")
        if expected_images == EXPECTED_IMAGES and (
            sample_id not in full.EXPECTED_TYPE3
            or sample.get("path") != rows[sample_id]["path"]
            or sample.get("variant") != rows[sample_id]["variant"]
            or sample["source_sha256"] != rows[sample_id]["sha256"]
        ):
            raise OracleError("generic oracle source identity differs from matrix")
        images = sample.get("images")
        if not isinstance(images, list):
            raise OracleError("generic oracle sample images must be a list")
        if expected_images == EXPECTED_IMAGES and len(images) != full.EXPECTED_TYPE3[sample_id]:
            raise OracleError(f"{sample_id}: generic image count differs")
        coordinates = set()
        for image in images:
            image = require_keys(
                image,
                {"page", "image", "offset", "length", "width", "height", "segment_0",
                 "segment_4", "generic_profile", "normalized_pixel_sha256",
                 "black_pixels", "status"},
                "generic oracle image",
            )
            if image.get("status") != "PASS":
                raise OracleError("generic oracle manifest contains an unverified image")
            page = full._positive_int(image.get("page"), "page")
            index = full._positive_int(image.get("image"), "image index")
            if (page, index) in coordinates:
                raise OracleError("generic oracle has a duplicate image coordinate")
            coordinates.add((page, index))
            offset = full._nonnegative_int(image.get("offset"), "image offset")
            length = full._positive_int(image.get("length"), "image length")
            width = full._positive_int(image.get("width"), "image width")
            height = full._positive_int(image.get("height"), "image height")
            if length > full.MAX_ENCODED_BYTES or (width + 7) // 8 * height > full.MAX_BITMAP_BYTES:
                raise OracleError("generic oracle image exceeds selected bounds")
            selected = []
            for number in (0, 4):
                segment = require_keys(
                    image.get(f"segment_{number}"),
                    {"number", "offset", "length", "encoded_sha256"},
                    f"generic oracle segment {number}",
                )
                if segment.get("number") != number:
                    raise OracleError(f"generic oracle segment {number} number differs")
                start = full._nonnegative_int(segment.get("offset"), "segment offset")
                size = full._positive_int(segment.get("length"), "segment length")
                if start < offset + 48 or start > offset + length or size > offset + length - start:
                    raise OracleError(f"generic oracle segment {number} escapes image")
                valid_sha(segment.get("encoded_sha256"), "segment encoded_sha256")
                selected.append((start, size))
            if selected[0][0] != offset + 48 or selected[0][0] + selected[0][1] > selected[1][0]:
                raise OracleError("generic oracle page-information span differs")
            if selected[1][0] + selected[1][1] != offset + length:
                raise OracleError("generic oracle generic-region span differs")
            profile = require_keys(
                image.get("generic_profile"),
                {"page_flags", "generic_flags", "template", "typical_prediction", "adaptive_pixel"},
                "generic oracle profile",
            )
            if profile != {
                "page_flags": 1, "generic_flags": 4, "template": 2,
                "typical_prediction": False, "adaptive_pixel": [2, -1],
            }:
                raise OracleError("generic oracle profile differs from observed template-2 subset")
            pixel_sha = valid_sha(image.get("normalized_pixel_sha256"), "normalized_pixel_sha256")
            black = full._nonnegative_int(image.get("black_pixels"), "black pixel count")
            if black > width * height:
                raise OracleError("generic oracle black-pixel count exceeds image dimensions")
            if expected_images == EXPECTED_IMAGES and (sample_id, page, index) in SPOTS:
                if (pixel_sha, black) != SPOTS[(sample_id, page, index)]:
                    raise OracleError("generic oracle independently measured spot differs")
            count += 1
    if count != expected_images:
        raise OracleError(f"generic oracle manifest contains {count} images, expected {expected_images}")
    if expected_images == EXPECTED_IMAGES and sample_ids != set(full.EXPECTED_TYPE3):
        raise OracleError("generic oracle manifest lacks pinned source IDs")


def compare_manifest(observed: dict, baseline: dict) -> list[str]:
    differences = full.compare_manifest(observed, baseline)
    if observed.get("oracle_kind") != baseline.get("oracle_kind"):
        differences.insert(0, "oracle_kind")
    return differences


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path,
                        default=Path(os.environ["CAJ2PDF_CORPUS_DIR"])
                        if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--qpdf", default="qpdf")
    parser.add_argument("--pdfimages", default="pdfimages")
    parser.add_argument("--mutool", default="mutool")
    parser.add_argument("--write-manifest", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.manifest.exists():
            validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
        if args.corpus_dir is None:
            report = {"status": "NOT_RUN", "reason": "external CAJSamples corpus is unset",
                      "expected_images": EXPECTED_IMAGES, "tool_agreements": 0}
        elif not args.corpus_dir.is_dir():
            report = {"status": "NOT_RUN", "reason": "external CAJSamples corpus is unavailable",
                      "expected_images": EXPECTED_IMAGES, "tool_agreements": 0}
        else:
            rows, paths = full.validate_sources(full.DEFAULT_MATRIX, args.corpus_dir)
            tools = {name: full.tool_path(getattr(args, name)) for name in ("qpdf", "pdfimages", "mutool")}
            missing = [name for name, path in tools.items() if path is None]
            if missing:
                report = {"status": "NOT_RUN", "reason": f"external oracle tool unavailable: {', '.join(missing)}",
                          "expected_images": EXPECTED_IMAGES, "tool_agreements": 0,
                          "source_hashes_checked": len(rows)}
            else:
                inventory = full.run_directory_inventory(args.corpus_dir)
                cases = full.checked_cases(inventory, rows, paths)
                manifest, report = manifest_for_cases(cases, tools, full.DEFAULT_MATRIX)
                report["source_hashes_checked"] = len(rows)
                try:
                    full.validate_sources(full.DEFAULT_MATRIX, args.corpus_dir)
                except full.OracleError as exc:
                    report.update(status="FAIL", source_postcheck_error=str(exc))
                if report["status"] == "PASS":
                    validate_manifest(manifest)
                    if args.write_manifest:
                        temporary = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
                        temporary.write_text(
                            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
                            encoding="utf-8",
                        )
                        temporary.replace(args.manifest)
                    elif args.manifest.exists():
                        baseline = json.loads(args.manifest.read_text(encoding="utf-8"))
                        report["toolchain_drift"] = manifest["toolchains"] != baseline["toolchains"]
                        differences = compare_manifest(manifest, baseline)
                        if differences:
                            report.update(status="FAIL", manifest_differences=differences)
                    else:
                        report.update(status="FAIL", reason="metadata manifest is absent; use --write-manifest")
    except (OSError, ValueError, conformance.ConformanceError, full.OracleError,
            json.JSONDecodeError) as exc:
        report = {"status": "FAIL", "error": str(exc), "expected_images": EXPECTED_IMAGES,
                  "tool_agreements": 0}
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(f"Generic-only JBIG2 oracle [{report['status']}]: "
              f"agreement={report.get('tool_agreements', 0)}/{EXPECTED_IMAGES}")
        if report.get("reason") or report.get("error"):
            print(report.get("reason") or report.get("error"), file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
