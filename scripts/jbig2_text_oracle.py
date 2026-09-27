#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional black-box pixel oracle for HN/C8 JBIG2 text regions alone.

Each temporary image contains its original DIB and complete segments #0–#3.
The generic region #4 is excluded. Only hashes and metadata may enter Git.
"""

from __future__ import annotations

import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import NamedTuple

import conformance
import jbig2_generic_oracle as generic
import jbig2_oracle as full
import jbig2_text_region_headers as headers


DEFAULT_MANIFEST = full.ROOT / "tests/conformance/jbig2_text_oracle.json"
SCHEMA_VERSION = 1
ORACLE_KIND = "page-information-and-text-region-only"
NORMALIZATION = generic.NORMALIZATION
CORPUS_REVISION = generic.CORPUS_REVISION
EXPECTED_IMAGES = full.EXPECTED_IMAGES
MAX_RECORD_BYTES = full.MAX_ENCODED_BYTES
MAX_PDF_BYTES = MAX_RECORD_BYTES + 4096
MAX_LOG_BYTES = full.MAX_LOG_BYTES
STANDARD = "STANDARD_VALID"
ANOMALY = "INTEROPERABILITY_NONCONFORMING"


class OracleError(full.OracleError):
    """An invalid source, selected record, tool result, or manifest."""


class SegmentSlice(NamedTuple):
    number: int
    offset: int
    length: int


def selected_spans(case: dict) -> tuple[SegmentSlice, ...]:
    """Check exact, contiguous original framing, including omitted segment #4."""
    image_start = case["offset"] + 48
    image_end = case["offset"] + case["length"]
    cursor = image_start
    selected = []
    expected = ((48, ()), (0, ()), (0, (1,)), (6, (2,)), (38, ()))
    with case["source_path"].open("rb") as source:
        for number, (kind, refs) in enumerate(expected):
            segment = case["segments"][number]
            if (segment.get("number"), segment.get("type"),
                    segment.get("page_association"), segment.get("refs")) != (
                        number, kind, 1, list(refs)):
                raise OracleError(f"segment {number} identity differs from the pinned profile")
            header_length = full._positive_int(segment.get("header_length"), "segment header length")
            data_offset = full._nonnegative_int(segment.get("data_offset"), "segment data offset")
            data_length = full._nonnegative_int(segment.get("data_length"), "segment data length")
            if header_length != 11 + len(refs) or data_offset - header_length != cursor:
                raise OracleError(f"segment {number} header or data is not contiguous")
            if data_offset > image_end or data_length > image_end - data_offset:
                raise OracleError(f"segment {number} escapes the type-3 image")
            raw = full.read_span(source, cursor, header_length)
            if (int.from_bytes(raw[:4], "big") != number or raw[4] != kind
                    or raw[5] >> 5 != len(refs)
                    or raw[6:6 + len(refs)] != bytes(refs)
                    or raw[-5] != 1
                    or int.from_bytes(raw[-4:], "big") != data_length):
                raise OracleError(f"segment {number} raw header differs from inventory")
            cursor = data_offset + data_length
            if number < 4:
                selected.append(SegmentSlice(number, data_offset - header_length,
                                             header_length + data_length))
    if cursor != image_end:
        raise OracleError("segment 4 does not end at the image boundary")
    if 48 + sum(span.length for span in selected) > MAX_RECORD_BYTES:
        raise OracleError("text-only temporary record exceeds the bounded image limit")
    return tuple(selected)


def selected_hashes(case: dict, spans: tuple[SegmentSlice, ...]) -> tuple[str, dict[int, str]]:
    with case["source_path"].open("rb") as source:
        wrapper_sha = full.sha256_span(source, case["offset"], 48)
        hashes = {span.number: full.sha256_span(source, span.offset, span.length)
                  for span in spans}
    return wrapper_sha, hashes


def spool_record(case: dict, spans: tuple[SegmentSlice, ...], wrapper_sha: str,
                 hashes: dict[int, str], destination: Path) -> tuple[int, str]:
    """Copy the DIB and four selected source spans through 64 KiB chunks."""
    import hashlib

    digest = hashlib.sha256()
    with case["source_path"].open("rb") as source, destination.open("wb") as output:
        generic.copy_verified(source, output, case["offset"], 48, wrapper_sha, digest)
        for span in spans:
            generic.copy_verified(source, output, span.offset, span.length,
                                  hashes[span.number], digest)
    length = 48 + sum(span.length for span in spans)
    if destination.stat().st_size != length:
        raise OracleError("text-only temporary record has an unexpected size")
    return length, digest.hexdigest()


def _file_limit(max_bytes: int):
    """Set a per-file output ceiling in POSIX children, including descendants."""
    import resource

    resource.setrlimit(resource.RLIMIT_FSIZE, (max_bytes, max_bytes))


def run_bounded_tool(argv: list[str], log_path: Path, output_cap: int,
                     timeout: int = full.TOOL_TIMEOUT_SECONDS) -> None:
    """Bound elapsed time, captured diagnostics, and POSIX child file sizes."""
    if os.name != "posix":
        raise OracleError("bounded external rendering requires POSIX file-size limits")
    start = time.monotonic()
    try:
        process = subprocess.Popen(
            argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, start_new_session=True,
            preexec_fn=lambda: _file_limit(output_cap),
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise OracleError(f"{Path(argv[0]).name} could not start: {exc}") from exc
    captured = bytearray()
    try:
        assert process.stdout is not None
        with closing(selectors.DefaultSelector()) as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = timeout - (time.monotonic() - start)
                if remaining <= 0 or not selector.select(remaining):
                    raise OracleError(f"{Path(argv[0]).name} timed out")
                chunk = os.read(process.stdout.fileno(), 4096)
                if not chunk:
                    selector.unregister(process.stdout)
                    continue
                captured.extend(chunk)
                if len(captured) > MAX_LOG_BYTES:
                    raise OracleError(f"{Path(argv[0]).name} diagnostic exceeds {MAX_LOG_BYTES} bytes")
        remaining = timeout - (time.monotonic() - start)
        if remaining <= 0:
            raise OracleError(f"{Path(argv[0]).name} timed out")
        result = process.wait(timeout=remaining)
    except (OracleError, subprocess.TimeoutExpired) as exc:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        if isinstance(exc, subprocess.TimeoutExpired):
            raise OracleError(f"{Path(argv[0]).name} timed out") from exc
        raise
    finally:
        if process.stdout is not None:
            process.stdout.close()
    log_path.write_bytes(captured)
    if result != 0:
        detail = captured.decode("utf-8", "replace")
        raise OracleError(f"{Path(argv[0]).name} exited {result}: {detail}")


def tool_metadata(paths: dict[str, Path]) -> dict:
    """Collect identities and linkage evidence with bounded probe output."""
    versions = {}
    links = {"mutool": None, "pdfimages": None}
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-text-tools-") as directory:
        temporary = Path(directory)
        for name, path in paths.items():
            flag = "-v" if name in ("mutool", "pdfimages") else "--version"
            log = temporary / f"{name}.version.log"
            run_bounded_tool([str(path), flag], log, MAX_LOG_BYTES, timeout=10)
            output = log.read_text(encoding="utf-8", errors="replace")
            if not output.strip():
                raise OracleError(f"{name} gave no version")
            versions[name] = {"version": output.splitlines()[0].strip(),
                              "binary_sha256": full.sha256_file(path)}
        ldd = shutil.which("ldd") if sys.platform.startswith("linux") else None
        if ldd is not None:
            for name in ("mutool", "pdfimages"):
                log = temporary / f"{name}.ldd.log"
                try:
                    run_bounded_tool([ldd, str(paths[name])], log, MAX_LOG_BYTES, timeout=10)
                except OracleError:
                    links[name] = None
                else:
                    links[name] = "libjbig2dec.so" in log.read_text(
                        encoding="utf-8", errors="replace")
    return {
        "tools": versions,
        "backend_evidence": {
            "dynamic_libjbig2dec": links,
            "implementation_independence": "UNVERIFIED",
            "meaning": "Matching PBM bytes establish tool agreement, not distinct decoder code.",
        },
    }


def _bounded_files(temporary: Path, record_bytes: int, raster_bytes: int) -> None:
    total = 0
    with os.scandir(temporary) as entries:
        for count, entry in enumerate(entries, 1):
            if count > 8 or entry.is_symlink() or not entry.is_file():
                raise OracleError("external renderer created unexpected temporary entries")
            total += entry.stat().st_size
    ceiling = record_bytes + MAX_PDF_BYTES + 2 * (raster_bytes + 512) + 3 * MAX_LOG_BYTES
    if total > ceiling:
        raise OracleError("text-only temporary files exceed the per-case disk limit")


def decode_case(case: dict, spans: tuple[SegmentSlice, ...], wrapper_sha: str,
                hashes: dict[int, str], tools: dict[str, Path]) -> tuple[str, int, int, int]:
    """Validate a one-image PDF and compare bounded, normalized P4 pixels."""
    raster_bytes = (case["width"] + 7) // 8 * case["height"]
    if raster_bytes > full.MAX_BITMAP_BYTES:
        raise OracleError("text-only PBM raster exceeds configured limit")
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-text-") as directory:
        temporary = Path(directory)
        record = temporary / "text-record.caj"
        record_bytes, record_sha = spool_record(case, spans, wrapper_sha, hashes, record)
        pdf = temporary / "text.pdf"
        pdf_bytes = full.write_pdf({
            "source_path": record, "offset": 0, "length": record_bytes,
            "width": case["width"], "height": case["height"],
        }, pdf, record_sha)
        if pdf_bytes > MAX_PDF_BYTES:
            raise OracleError("text-only PDF exceeds configured limit")
        qpdf_log = temporary / "qpdf.log"
        run_bounded_tool([str(tools["qpdf"]), "--check", str(pdf)], qpdf_log, MAX_LOG_BYTES)
        full.check_qpdf_log(qpdf_log)
        _bounded_files(temporary, record_bytes, raster_bytes)
        prefix = temporary / "poppler"
        run_bounded_tool([str(tools["pdfimages"]), str(pdf), str(prefix)],
                         temporary / "pdfimages.log", raster_bytes + 512)
        poppler = list(temporary.glob("poppler-*"))
        if len(poppler) != 1 or poppler[0].suffix != ".pbm":
            raise OracleError(f"pdfimages emitted {len(poppler)} files instead of one PBM")
        if poppler[0].stat().st_size > raster_bytes + 512:
            raise OracleError("pdfimages PBM exceeds configured limit")
        _bounded_files(temporary, record_bytes, raster_bytes)
        mupdf = temporary / "mupdf.pbm"
        run_bounded_tool([
            str(tools["mutool"]), "draw", "-q", "-F", "pbm", "-r", "72",
            "-o", str(mupdf), str(pdf),
        ], temporary / "mutool.log", raster_bytes + 512)
        if not mupdf.is_file() or mupdf.stat().st_size > raster_bytes + 512:
            raise OracleError("mutool PBM is missing or exceeds configured limit")
        _bounded_files(temporary, record_bytes, raster_bytes)
        pixels, black = full.compare_pbm(poppler[0], mupdf, case["width"], case["height"])
    return pixels, black, record_bytes, pdf_bytes


def classification(case: dict, header: dict) -> str:
    anomaly = case["coordinate"] == full.ANOMALY_COORDINATE
    if anomaly != (header["flags"] == "0xa40c"):
        raise OracleError("text header anomaly moved or changed")
    if anomaly:
        if header["refinement"] != 0:
            raise OracleError("anomalous text header refinement flag changed")
        return ANOMALY
    if header["refinement"] != 1:
        raise OracleError("standard text header refinement flag changed")
    return STANDARD


def preflight(cases: list[dict], recorded: dict, baseline: dict | None = None) -> list[dict]:
    """Validate all 546 source records and #69 aggregates before rendering."""
    prepared = []
    observed = []
    previous = {
        (sample["id"], image["page"], image["image"]): image
        for sample in baseline["samples"] for image in sample["images"]
    } if baseline is not None else {}
    for case in cases:
        key = case["coordinate"]
        if key not in recorded:
            raise OracleError(f"unpinned text region coordinate {key!r}")
        spans = selected_spans(case)
        profile, encoded_sha = full.profile_and_hash(case)
        if profile != recorded[key]["profile"] or encoded_sha != recorded[key]["encoded_sha256"]:
            raise OracleError(f"{key!r}: full-image profile or encoded SHA differs from #43")
        header = headers.inspect_case(case, recorded[key])
        kind = classification(case, header)
        wrapper_sha, hashes = selected_hashes(case, spans)
        if baseline is not None:
            old = previous.get(key)
            if old is None or any(
                old[f"segment_{span.number}"] != {
                    "number": span.number, "offset": span.offset,
                    "length": span.length, "encoded_sha256": hashes[span.number],
                } for span in spans
            ):
                raise OracleError(f"{key!r}: selected segment span or SHA differs from manifest")
            if old["text_header"] != {
                "flags": header["flags"], "instances": header["instances"],
                "classification": kind,
            }:
                raise OracleError(f"{key!r}: text header metadata differs from manifest")
        prepared.append({"case": case, "spans": spans, "header": header,
                         "classification": kind, "wrapper_sha": wrapper_sha,
                         "hashes": hashes})
        observed.append((key, header, case["offset"]))
    if len(prepared) != EXPECTED_IMAGES or {item[0] for item in observed} != set(recorded):
        raise OracleError("text-only preflight lacks a pinned image")
    if headers.aggregate(observed) != headers.EXPECTED_AGGREGATES:
        raise OracleError("text header aggregates differ from #69 pinned profile")
    return prepared


def location(case: dict) -> dict:
    segments = []
    for number in range(4):
        segment = case["segments"][number]
        segments.append({"number": number,
                         "offset": segment["data_offset"] - segment["header_length"],
                         "length": segment["header_length"] + segment["data_length"]})
    return {"id": case["id"], "page": case["page"], "image": case["image"],
            "offset": case["offset"], "length": case["length"], "segments": segments}


def manifest_for_cases(prepared: list[dict], tools: dict[str, Path],
                       matrix_path: Path) -> tuple[dict, dict]:
    toolchains = tool_metadata(tools)
    samples: dict[str, dict] = {}
    failures = []
    maximum_record = 0
    maximum_pdf = 0
    started = time.monotonic()
    for number, item in enumerate(prepared, 1):
        case = item["case"]
        sample = samples.setdefault(case["id"], {
            "id": case["id"], "path": case["path"], "variant": case["variant"],
            "source_sha256": case["source_sha256"], "images": [],
        })
        try:
            pixels, black, record_bytes, pdf_bytes = decode_case(
                case, item["spans"], item["wrapper_sha"], item["hashes"], tools)
            maximum_record = max(maximum_record, record_bytes)
            maximum_pdf = max(maximum_pdf, pdf_bytes)
            image = {
                "page": case["page"], "image": case["image"],
                "offset": case["offset"], "length": case["length"],
                "width": case["width"], "height": case["height"],
            }
            for span in item["spans"]:
                image[f"segment_{span.number}"] = {
                    "number": span.number, "offset": span.offset,
                    "length": span.length, "encoded_sha256": item["hashes"][span.number],
                }
            image["text_header"] = {
                "flags": item["header"]["flags"],
                "instances": item["header"]["instances"],
                "classification": item["classification"],
            }
            image.update(normalized_pixel_sha256=pixels, black_pixels=black, status="PASS")
            sample["images"].append(image)
        except (OSError, full.OracleError) as exc:
            failures.append({**location(case), "error": str(exc), "status": "FAIL"})
        if number % 50 == 0 or number == len(prepared):
            print(f"Text-only oracle progress: {number}/{len(prepared)}; failures={len(failures)}",
                  file=sys.stderr, flush=True)
    agreements = sum(len(sample["images"]) for sample in samples.values())
    manifest = {
        "schema_version": SCHEMA_VERSION, "oracle_kind": ORACLE_KIND,
        "matrix_sha256": full.sha256_file(matrix_path),
        "corpus_revision": CORPUS_REVISION, "normalization": NORMALIZATION,
        "backend_independence": "UNVERIFIED", "toolchains": toolchains,
        "samples": [samples[key] for key in sorted(samples)],
    }
    report = {
        "status": "PASS" if not failures and agreements == EXPECTED_IMAGES else "FAIL",
        "expected_images": EXPECTED_IMAGES, "checked_images": len(prepared),
        "tool_agreements": agreements, "completed": agreements, "failed": len(failures),
        "skipped": len(prepared) - agreements - len(failures), "failures": failures,
        "source_hashes_before": 27, "source_hashes_after": 0,
        "standard_valid": sum(item["classification"] == STANDARD for item in prepared),
        "interoperability_cases": sum(item["classification"] == ANOMALY for item in prepared),
        "rust_text_parity": "NOT_RUN", "rust_text_checked": 0,
        "backend_independence": "UNVERIFIED", "toolchains": toolchains,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "max_temp_record_bytes": maximum_record, "max_temp_pdf_bytes": maximum_pdf,
        "max_single_pbm_raster_bytes": max((item["case"]["width"] + 7) // 8 *
                                            item["case"]["height"] for item in prepared),
    }
    return manifest, report


def require_keys(value: object, allowed: set[str], label: str) -> dict:
    if not isinstance(value, dict):
        raise OracleError(f"{label} must be an object")
    full.reject_unknown_fields(value, allowed, label)
    if set(value) != allowed:
        raise OracleError(f"{label} is missing fields: {', '.join(sorted(allowed - set(value)))}")
    return value


def validate_manifest(manifest: object, expected_images: int = EXPECTED_IMAGES) -> None:
    manifest = require_keys(manifest, {
        "schema_version", "oracle_kind", "matrix_sha256", "corpus_revision",
        "normalization", "backend_independence", "toolchains", "samples",
    }, "text oracle manifest")
    if type(manifest["schema_version"]) is not int or (
            manifest["schema_version"], manifest["oracle_kind"],
            manifest["corpus_revision"], manifest["normalization"],
            manifest["backend_independence"]) != (
                SCHEMA_VERSION, ORACLE_KIND, CORPUS_REVISION, NORMALIZATION, "UNVERIFIED"):
        raise OracleError("text oracle manifest identity or pixel convention differs")
    generic.valid_sha(manifest["matrix_sha256"], "matrix_sha256")
    generic.validate_toolchains(manifest["toolchains"])
    chains = require_keys(manifest["toolchains"], {"tools", "backend_evidence"},
                          "text oracle toolchains")
    evidence = require_keys(chains["backend_evidence"],
                            {"dynamic_libjbig2dec", "implementation_independence", "meaning"},
                            "text oracle backend evidence")
    require_keys(evidence["dynamic_libjbig2dec"], {"mutool", "pdfimages"},
                 "text oracle dynamic linkage evidence")
    if not isinstance(manifest["samples"], list):
        raise OracleError("text oracle samples must be a list")
    reference = {}
    matrix_rows = {}
    if expected_images == EXPECTED_IMAGES:
        if manifest["matrix_sha256"] != full.sha256_file(full.DEFAULT_MATRIX):
            raise OracleError("text oracle matrix SHA-256 differs")
        _, reference = headers.load_baseline()
        matrix_rows = {row["id"]: row for row in conformance.load_matrix(full.DEFAULT_MATRIX)}
    seen_samples = set()
    seen_coordinates = set()
    observed = []
    previous_id = None
    for sample in manifest["samples"]:
        sample = require_keys(sample, {"id", "path", "variant", "source_sha256", "images"},
                              "text oracle sample")
        sample_id = sample["id"]
        if not isinstance(sample_id, str) or sample_id in seen_samples:
            raise OracleError("text oracle has duplicate or invalid source ID")
        if (not isinstance(sample["path"], str) or not sample["path"]
                or sample["variant"] not in ("HN", "C8")):
            raise OracleError("text oracle sample path or variant differs")
        if previous_id is not None and sample_id < previous_id:
            raise OracleError("text oracle samples are not sorted")
        previous_id = sample_id
        seen_samples.add(sample_id)
        generic.valid_sha(sample["source_sha256"], "source_sha256")
        if expected_images == EXPECTED_IMAGES and (
                sample_id not in full.EXPECTED_TYPE3 or
                (sample["path"], sample["variant"], sample["source_sha256"]) !=
                (matrix_rows[sample_id]["path"], matrix_rows[sample_id]["variant"],
                 matrix_rows[sample_id]["sha256"])):
            raise OracleError("text oracle source identity differs from matrix")
        if not isinstance(sample["images"], list):
            raise OracleError("text oracle images must be a list")
        if expected_images == EXPECTED_IMAGES and len(sample["images"]) != full.EXPECTED_TYPE3[sample_id]:
            raise OracleError(f"{sample_id}: text oracle image count differs")
        previous_coordinate = None
        for image in sample["images"]:
            image = require_keys(image, {
                "page", "image", "offset", "length", "width", "height",
                "segment_0", "segment_1", "segment_2", "segment_3", "text_header",
                "normalized_pixel_sha256", "black_pixels", "status",
            }, "text oracle image")
            if image["status"] != "PASS":
                raise OracleError("text oracle manifest contains an unverified image")
            page = full._positive_int(image["page"], "page")
            index = full._positive_int(image["image"], "image index")
            key = (sample_id, page, index)
            if key in seen_coordinates:
                raise OracleError("text oracle has a duplicate image coordinate")
            if previous_coordinate is not None and (page, index) < previous_coordinate:
                raise OracleError("text oracle images are not sorted")
            previous_coordinate = (page, index)
            seen_coordinates.add(key)
            offset = full._nonnegative_int(image["offset"], "image offset")
            length = full._positive_int(image["length"], "image length")
            width = full._positive_int(image["width"], "image width")
            height = full._positive_int(image["height"], "image height")
            if length > full.MAX_ENCODED_BYTES or (width + 7) // 8 * height > full.MAX_BITMAP_BYTES:
                raise OracleError("text oracle image exceeds configured bounds")
            cursor = offset + 48
            spans = []
            for number in range(4):
                segment = require_keys(image[f"segment_{number}"],
                                       {"number", "offset", "length", "encoded_sha256"},
                                       f"text oracle segment {number}")
                if (type(segment["number"]) is not int or segment["number"] != number
                        or type(segment["offset"]) is not int or segment["offset"] != cursor):
                    raise OracleError(f"text oracle segment {number} position differs")
                size = full._positive_int(segment["length"], "segment length")
                if size > offset + length - cursor:
                    raise OracleError(f"text oracle segment {number} escapes image")
                generic.valid_sha(segment["encoded_sha256"], "segment encoded_sha256")
                spans.append(segment)
                cursor += size
            if 48 + sum(segment["length"] for segment in spans) > MAX_RECORD_BYTES:
                raise OracleError("text oracle temporary record exceeds bound")
            header = require_keys(image["text_header"],
                                  {"flags", "instances", "classification"}, "text oracle header")
            if header["flags"] not in headers.EXPECTED_FLAGS:
                raise OracleError("text oracle text flags differ from #69 profile")
            instances = full._positive_int(header["instances"], "text instances")
            anomaly = key == full.ANOMALY_COORDINATE if expected_images == EXPECTED_IMAGES else header["flags"] == "0xa40c"
            if header["classification"] != (ANOMALY if anomaly else STANDARD):
                raise OracleError("text oracle anomaly classification differs")
            if anomaly != (header["flags"] == "0xa40c"):
                raise OracleError("text oracle anomaly flag differs")
            generic.valid_sha(image["normalized_pixel_sha256"], "normalized_pixel_sha256")
            black = full._nonnegative_int(image["black_pixels"], "black pixels")
            if black > width * height:
                raise OracleError("text oracle black pixels exceed image dimensions")
            if expected_images == EXPECTED_IMAGES:
                if key not in reference:
                    raise OracleError("text oracle coordinate lacks #43 baseline")
                old = reference[key]
                if (offset, length, width, height) != (old["offset"], old["length"],
                                                      old["width"], old["height"]):
                    raise OracleError("text oracle image metadata differs from #43")
                if header["flags"] != old["profile"]["text_flags"]:
                    raise OracleError("text oracle text flags differ from #43")
            if offset + length - cursor < 31:
                raise OracleError("text oracle omits an implausibly short segment 4")
            # Segment #3 uses the observed 12-byte referred-to header.
            data_length = spans[3]["length"] - 12
            if data_length < headers.HEADER_BYTES + 2:
                raise OracleError("text oracle text segment is too short")
            observed.append((key, {
                "flags": header["flags"], "refinement": 0 if anomaly else 1,
                "instances": instances, "header_bytes": headers.HEADER_BYTES,
                "data_length": data_length, "body_length": data_length - headers.HEADER_BYTES,
            }, offset))
    if len(seen_coordinates) != expected_images:
        raise OracleError(f"text oracle contains {len(seen_coordinates)} images, expected {expected_images}")
    if expected_images == EXPECTED_IMAGES:
        if seen_samples != set(full.EXPECTED_TYPE3) or seen_coordinates != set(reference):
            raise OracleError("text oracle sample or coordinate set differs")
        if headers.aggregate(observed) != headers.EXPECTED_AGGREGATES:
            raise OracleError("text oracle header aggregates differ from #69")


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
    supplied = argv if argv is not None else sys.argv[1:]
    explicit_manifest = any(value == "--manifest" or value.startswith("--manifest=")
                            for value in supplied)
    explicit_corpus = (any(value == "--corpus-dir" or value.startswith("--corpus-dir=")
                           for value in supplied) or bool(os.environ.get("CAJ2PDF_CORPUS_DIR")))
    report: dict = {
        "status": "NOT_RUN", "expected_images": EXPECTED_IMAGES,
        "checked_images": 0, "tool_agreements": 0,
        "completed": 0, "failed": 0, "skipped": EXPECTED_IMAGES,
        "source_hashes_before": 0, "source_hashes_after": 0,
        "rust_text_parity": "NOT_RUN", "rust_text_checked": 0,
    }
    source_checked = False
    manifest_to_write: dict | None = None
    try:
        if args.manifest.exists():
            baseline = json.loads(args.manifest.read_text(encoding="utf-8"))
            validate_manifest(baseline)
        elif explicit_manifest or not args.write_manifest:
            raise OracleError("metadata manifest is absent; use --write-manifest")
        else:
            baseline = None
        if args.corpus_dir is None:
            report["reason"] = "external CAJSamples corpus is unset"
        elif not args.corpus_dir.is_dir():
            if explicit_corpus:
                raise OracleError("explicit external CAJSamples corpus is unavailable")
            report["reason"] = "external CAJSamples corpus is unavailable"
        else:
            rows, paths = full.validate_sources(full.DEFAULT_MATRIX, args.corpus_dir)
            source_checked = True
            report["source_hashes_before"] = len(rows)
            try:
                tools = {name: full.tool_path(getattr(args, name))
                         for name in ("qpdf", "pdfimages", "mutool")}
                missing = [name for name, path in tools.items() if path is None]
                if missing:
                    raise OracleError(f"external oracle tool unavailable: {', '.join(missing)}")
                _, recorded = headers.load_baseline()
                cases = full.checked_cases(full.run_directory_inventory(args.corpus_dir), rows, paths)
                prepared = preflight(cases, recorded, baseline)
                manifest, result = manifest_for_cases(prepared, tools, full.DEFAULT_MATRIX)
                report = result
                if report["status"] == "PASS":
                    validate_manifest(manifest)
                    if baseline is not None:
                        report["toolchain_drift"] = manifest["toolchains"] != baseline["toolchains"]
                        differences = compare_manifest(manifest, baseline)
                        if differences:
                            report.update(status="FAIL", manifest_differences=differences)
                    if report["status"] == "PASS" and args.write_manifest:
                        manifest_to_write = manifest
            finally:
                if source_checked:
                    try:
                        full.validate_sources(full.DEFAULT_MATRIX, args.corpus_dir)
                    except (OSError, full.OracleError) as exc:
                        report.update(status="FAIL", source_postcheck_error=str(exc))
                    else:
                        report["source_hashes_after"] = len(rows)
            if report["status"] == "PASS" and manifest_to_write is not None:
                temporary = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
                temporary.write_text(json.dumps(manifest_to_write, ensure_ascii=False, indent=2) + "\n",
                                     encoding="utf-8")
                temporary.replace(args.manifest)
    except (OSError, ValueError, TypeError, KeyError, conformance.ConformanceError,
            full.OracleError, headers.InventoryError, json.JSONDecodeError) as exc:
        # This is an operation failure, not a failed image-rendering attempt.
        # Preserve mutually exclusive completed/failed/skipped case counts.
        report.update(status="FAIL", error=str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(f"Text-only JBIG2 oracle [{report['status']}]: "
              f"agreement={report['tool_agreements']}/{EXPECTED_IMAGES}")
        if report.get("reason") or report.get("error"):
            print(report.get("reason") or report.get("error"), file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
