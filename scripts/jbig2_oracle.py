#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Optional, black-box pixel oracle for externally held HN/C8 type-3 images.

The source inventory comes from the repository's Rust JBIG2 directory reader.
This script never bundles or implements a JBIG2 decoder. It copies one bounded
encoded span at a time into a temporary PDF and compares two external tools'
decoded PBM pixels. Only hashes and metadata may be committed.
"""

from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
from typing import BinaryIO

import conformance


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MATRIX = ROOT / "tests/conformance/matrix.json"
DEFAULT_MANIFEST = ROOT / "tests/conformance/jbig2_oracle.json"
DIRECTORY_SCRIPT = ROOT / "scripts/jbig2_directory_inventory.py"
EXPECTED_TYPE3 = {
    "issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj": 105,
    "issue-58/混凝土道面评价指标分析_谢永亮.caj": 4,
    "issue-66/IDL编译器的实现_词法分析部分_于埴尧.caj": 1,
    "issue-76/基于星载合成孔径雷达干涉测量技术的数字高程模型生成研究_任坤.caj": 208,
    "pull-72/碳_碳复合材料多重环境下的氧化机理研究_李龙.caj": 228,
}
EXPECTED_IMAGES = sum(EXPECTED_TYPE3.values())
ANOMALY_ID = next(sample_id for sample_id in EXPECTED_TYPE3 if sample_id.startswith("issue-43/"))
ANOMALY_COORDINATE = (ANOMALY_ID, 11, 1)
OBSERVED_TEXT_FLAGS = {
    0x840E, 0x880E, 0x8C0E, 0x900E, 0x940E, 0x980E, 0x9C0E,
    0xA00E, 0xA40C, 0xA40E, 0xA80E, 0xAC0E, 0xB00E, 0xB80E, 0xBC0E,
}
HEX_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
COPY_CHUNK = 64 * 1024
MAX_ENCODED_BYTES = 64 * 1024 * 1024
MAX_BITMAP_BYTES = 128 * 1024 * 1024
MAX_LOG_BYTES = 8192
MAX_INVENTORY_JSON_BYTES = 4 * 1024 * 1024
TOOL_TIMEOUT_SECONDS = 60


class OracleError(Exception):
    """A located source, inventory, tool, or pixel comparison failure."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(COPY_CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_span(source: BinaryIO, offset: int, length: int) -> str:
    digest = hashlib.sha256()
    source.seek(offset)
    remaining = length
    while remaining:
        chunk = source.read(min(remaining, COPY_CHUNK))
        if not chunk:
            raise OracleError(f"source changed or ended at offset {offset + length - remaining}")
        digest.update(chunk)
        remaining -= len(chunk)
    return digest.hexdigest()


def validate_sources(matrix_path: Path, corpus_dir: Path) -> tuple[list[dict], dict[str, Path]]:
    """Hash every HN/C8 source before reading any image record."""
    rows = [row for row in conformance.load_matrix(matrix_path) if row["variant"] in ("HN", "C8")]
    if len(rows) != 27:
        raise OracleError(f"expected 27 HN/C8 source rows, found {len(rows)}")
    try:
        root = corpus_dir.resolve(strict=True)
    except OSError as exc:
        raise OracleError(f"external corpus is unavailable: {exc}") from exc
    if not root.is_dir():
        raise OracleError(f"external corpus is not a directory: {root}")
    paths: dict[str, Path] = {}
    for row in rows:
        sample_id = row["id"]
        try:
            path = conformance.contained_file(root, conformance.relative_path(row["path"]))
        except conformance.ConformanceError as exc:
            raise OracleError(f"{sample_id}: {exc}") from exc
        if path.stat().st_size != row["size_bytes"]:
            raise OracleError(f"{sample_id}: source size differs from the pinned matrix")
        if sha256_file(path) != row["sha256"]:
            raise OracleError(f"{sample_id}: source SHA-256 differs from the pinned matrix")
        paths[sample_id] = path
    return rows, paths


def tool_path(value: str) -> Path | None:
    found = shutil.which(value)
    return Path(found).resolve() if found else None


def tool_metadata(paths: dict[str, Path]) -> dict:
    versions = {}
    for name, path in paths.items():
        flag = "-v" if name in ("mutool", "pdfimages") else "--version"
        try:
            result = subprocess.run(
                [str(path), flag], capture_output=True, timeout=10, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise OracleError(f"cannot identify {name}: {exc}") from exc
        combined = (result.stdout + result.stderr).decode("utf-8", "replace")
        if result.returncode != 0 or not combined.strip():
            raise OracleError(f"cannot identify {name}: {combined[:MAX_LOG_BYTES]}")
        versions[name] = {
            "version": combined.splitlines()[0].strip(),
            "binary_sha256": sha256_file(path),
        }
    links = {}
    if sys.platform.startswith("linux") and shutil.which("ldd"):
        for name in ("mutool", "pdfimages"):
            result = subprocess.run(
                ["ldd", str(paths[name])], capture_output=True, timeout=10, check=False
            )
            links[name] = (
                "libjbig2dec.so" in result.stdout.decode("utf-8", "replace")
                if result.returncode == 0
                else None
            )
    return {
        "tools": versions,
        "backend_evidence": {
            "dynamic_libjbig2dec": links,
            "implementation_independence": "UNVERIFIED",
            "meaning": "Matching PBM bytes establish tool agreement, not distinct decoder code.",
        },
    }


def run_directory_inventory(corpus_dir: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-directory-json-") as directory:
        output_path = Path(directory) / "inventory.json"
        error_path = Path(directory) / "inventory.log"
        try:
            with output_path.open("wb") as output, error_path.open("wb") as error:
                result = subprocess.run(
                    [sys.executable, str(DIRECTORY_SCRIPT), "--corpus-dir", str(corpus_dir), "--json"],
                    stdout=output, stderr=error, timeout=300, check=False,
                )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise OracleError(f"JBIG2 directory inventory could not run: {exc}") from exc
        if result.returncode != 0:
            with error_path.open("rb") as error:
                message = error.read(MAX_LOG_BYTES).decode("utf-8", "replace")
            raise OracleError(f"JBIG2 directory inventory failed: {message}")
        if output_path.stat().st_size > MAX_INVENTORY_JSON_BYTES:
            raise OracleError("JBIG2 directory JSON exceeds 4 MiB metadata limit")
        try:
            inventory = json.loads(output_path.read_text(encoding="utf-8"))
        except (UnicodeError, json.JSONDecodeError) as exc:
            raise OracleError(f"JBIG2 directory inventory emitted invalid JSON: {exc}") from exc
    if inventory.get("status") != "PASS":
        raise OracleError(f"JBIG2 directory inventory status is {inventory.get('status')!r}")
    return inventory


def _positive_int(value: object, label: str) -> int:
    if type(value) is not int or value <= 0:
        raise OracleError(f"{label} must be a positive integer")
    return value


def _nonnegative_int(value: object, label: str) -> int:
    if type(value) is not int or value < 0:
        raise OracleError(f"{label} must be a nonnegative integer")
    return value


def checked_cases(inventory: dict, rows: list[dict], paths: dict[str, Path]) -> list[dict]:
    """Trust neither the optional inventory command nor externally held bytes."""
    if not isinstance(inventory, dict) or inventory.get("status") != "PASS":
        raise OracleError("directory inventory did not pass")
    if (
        inventory.get("expected_images") != EXPECTED_IMAGES
        or inventory.get("checked_images") != EXPECTED_IMAGES
    ):
        raise OracleError("directory inventory has an incomplete image count")
    samples = inventory.get("samples")
    if not isinstance(samples, list) or len(samples) != len(EXPECTED_TYPE3):
        raise OracleError("directory inventory has an incomplete sample set")
    matrix = {row["id"]: row for row in rows}
    seen: set[str] = set()
    cases = []
    for sample in samples:
        if not isinstance(sample, dict):
            raise OracleError("directory sample must be an object")
        sample_id = sample.get("id")
        if sample_id not in EXPECTED_TYPE3 or sample_id in seen:
            raise OracleError(f"unexpected or duplicate directory sample: {sample_id!r}")
        seen.add(sample_id)
        row = matrix[sample_id]
        if (
            sample.get("path") != row["path"]
            or sample.get("source_sha256") != row["sha256"]
            or sample.get("variant") != row["variant"]
        ):
            raise OracleError(f"{sample_id}: directory source identity differs from matrix")
        images = sample.get("images")
        if not isinstance(images, list) or len(images) != EXPECTED_TYPE3[sample_id]:
            raise OracleError(f"{sample_id}: unexpected type-3 image count")
        seen_images = set()
        source_size = paths[sample_id].stat().st_size
        for image in images:
            if not isinstance(image, dict):
                raise OracleError(f"{sample_id}: image must be an object")
            page = _positive_int(image.get("page"), "page")
            index = _positive_int(image.get("image"), "image index")
            coordinate = (sample_id, page, index)
            if (page, index) in seen_images:
                raise OracleError(f"{sample_id}: duplicate page {page} image {index}")
            seen_images.add((page, index))
            offset = _nonnegative_int(image.get("offset"), "image offset")
            length = _positive_int(image.get("length"), "image length")
            width = _positive_int(image.get("width"), "image width")
            height = _positive_int(image.get("height"), "image height")
            if (
                length <= 48
                or length > MAX_ENCODED_BYTES
                or offset > source_size
                or length > source_size - offset
            ):
                raise OracleError(f"{sample_id} page {page} image {index}: image span is invalid or exceeds limit")
            stride = (width + 7) // 8
            if stride * height > MAX_BITMAP_BYTES:
                raise OracleError(f"{sample_id} page {page} image {index}: bitmap exceeds limit")
            segments = image.get("segments")
            if not isinstance(segments, list) or len(segments) != 5:
                raise OracleError(f"{sample_id} page {page} image {index}: expected five JBIG2 segments")
            expected_segments = zip(segments, (48, 0, 0, 6, 38), ([], [], [1], [2], []))
            for number, (segment, kind, refs) in enumerate(expected_segments):
                if not isinstance(segment, dict) or (
                    segment.get("number") != number
                    or segment.get("type") != kind
                    or segment.get("page_association") != 1
                    or segment.get("refs") != refs
                ):
                    raise OracleError(f"{sample_id} page {page} image {index}: unexpected segment {number} profile")
                data_offset = _nonnegative_int(segment.get("data_offset"), "segment data offset")
                data_length = _nonnegative_int(segment.get("data_length"), "segment data length")
                if (
                    data_offset < offset + 48
                    or data_offset > offset + length
                    or data_length > offset + length - data_offset
                ):
                    raise OracleError(f"{sample_id} page {page} image {index}: segment {number} data escapes image")
            if segments[-1]["data_offset"] + segments[-1]["data_length"] != offset + length:
                raise OracleError(f"{sample_id} page {page} image {index}: last segment does not reach image end")
            cases.append({
                "id": sample_id, "path": row["path"], "source_sha256": row["sha256"],
                "variant": row["variant"], "page": page, "image": index,
                "offset": offset, "length": length, "width": width, "height": height,
                "segments": segments, "coordinate": coordinate,
                "source_path": paths[sample_id],
            })
    if seen != set(EXPECTED_TYPE3):
        raise OracleError("directory sample set differs from expected five sources")
    return sorted(cases, key=lambda item: (item["id"], item["page"], item["image"]))


def read_span(source: BinaryIO, offset: int, length: int) -> bytes:
    source.seek(offset)
    data = source.read(length)
    if len(data) != length:
        raise OracleError(f"source changed or ended at offset {offset}")
    return data


def profile_and_hash(case: dict) -> tuple[dict, str]:
    """Read public DIB and T.88 profile fields, not compressed pixel data."""
    segments = case["segments"]
    with case["source_path"].open("rb") as source:
        dib = read_span(source, case["offset"], 48)
        if struct.unpack_from("<I", dib)[0] != 40:
            raise OracleError("type-3 DIB header is not 40 bytes")
        width, height, planes, depth, compression = struct.unpack_from("<iiHHI", dib, 4)
        if (
            (width, height) != (case["width"], case["height"])
            or planes != 1 or depth != 1 or compression != 0
        ):
            raise OracleError("type-3 DIB geometry or bi-level format differs from inventory")
        if dib[40:48] != b"\xff\xff\xff\x00\x00\x00\x00\x00":
            raise OracleError("type-3 palette differs from observed white/black palette")
        page = read_span(source, segments[0]["data_offset"], 19)
        if segments[0]["data_length"] != 19 or struct.unpack_from(">II", page) != (width, height):
            raise OracleError("JBIG2 page information disagrees with DIB dimensions")
        if page[16] != 1 or page[17:19] != b"\x00\x00":
            raise OracleError("JBIG2 page flags or striping differ from observed profile")
        symbol_flags = []
        symbol_counts = []
        for index, expected in ((1, 0x0800), (2, 0x1802)):
            if segments[index]["data_length"] < 12:
                raise OracleError(f"JBIG2 symbol dictionary {index} is too short")
            data = read_span(source, segments[index]["data_offset"], 12)
            flags = int.from_bytes(data[:2], "big")
            if flags != expected or data[2:4] != b"\x02\xff":
                raise OracleError(f"JBIG2 symbol dictionary {index} profile differs from observed subset")
            symbol_flags.append(flags)
            symbol_counts.append(list(struct.unpack_from(">II", data, 4)))
        if segments[3]["data_length"] < 23 or segments[4]["data_length"] < 20:
            raise OracleError("JBIG2 region segment is too short")
        text = read_span(source, segments[3]["data_offset"], 19)
        text_flags = int.from_bytes(text[17:19], "big")
        generic = read_span(source, segments[4]["data_offset"], 20)
        if generic[17] != 4 or generic[18:20] != b"\x02\xff":
            raise OracleError("JBIG2 generic region profile differs from observed subset")
        for label, region in (("text", text), ("generic", generic)):
            if struct.unpack_from(">IIII", region) != (width, height, 0, 0) or region[16] != 0:
                raise OracleError(f"JBIG2 {label} region differs from full-page profile")
        anomaly = case["coordinate"] == ANOMALY_COORDINATE
        if anomaly != (text_flags == 0xA40C):
            raise OracleError("JBIG2 text flag anomaly moved or changed")
        if text_flags not in OBSERVED_TEXT_FLAGS:
            raise OracleError(f"unexpected JBIG2 text flags 0x{text_flags:04x}")
        encoded_sha = sha256_span(source, case["offset"], case["length"])
    return {
        "page_flags": page[16],
        "symbol_flags": symbol_flags,
        "symbol_counts": symbol_counts,
        "text_flags": f"0x{text_flags:04x}",
        "generic_flags": generic[17],
        "text_flag_anomaly": "SBREFINE=0 with SBRTEMPLATE=1" if anomaly else None,
    }, encoded_sha


def write_pdf(case: dict, destination: Path, expected_encoded_sha256: str) -> int:
    """Write one image, verifying the exact bytes copied after the profile read."""
    width, height = case["width"], case["height"]
    payload_offset = case["offset"] + 48
    payload_length = case["length"] - 48
    commands = f"q\n{width} 0 0 {height} 0 0 cm\n/Im Do\nQ\n".encode("ascii")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] "
            "/Resources << /XObject << /Im 4 0 R >> >> /Contents 5 0 R >>"
        ).encode("ascii"),
    ]
    offsets = [0]
    copied_hash = hashlib.sha256()
    with case["source_path"].open("rb") as source, destination.open("wb") as output:
        source.seek(case["offset"])
        wrapper = source.read(48)
        if len(wrapper) != 48:
            raise OracleError("source changed or ended while reading type-3 wrapper")
        copied_hash.update(wrapper)
        output.write(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        for number, body in enumerate(objects, 1):
            offsets.append(output.tell())
            output.write(f"{number} 0 obj\n".encode("ascii"))
            output.write(body)
            output.write(b"\nendobj\n")
        offsets.append(output.tell())
        output.write(b"4 0 obj\n")
        output.write(
            f"<< /Type /XObject /Subtype /Image /Width {width} /Height {height} "
            f"/ColorSpace /DeviceGray /BitsPerComponent 1 /Decode [0 1] /Filter /JBIG2Decode "
            f"/Length {payload_length} >>\nstream\n".encode("ascii")
        )
        source.seek(payload_offset)
        remaining = payload_length
        while remaining:
            chunk = source.read(min(remaining, COPY_CHUNK))
            if not chunk:
                raise OracleError("source changed or ended while spooling JBIG2 payload")
            output.write(chunk)
            copied_hash.update(chunk)
            remaining -= len(chunk)
        if copied_hash.hexdigest() != expected_encoded_sha256:
            raise OracleError("encoded image span changed between profile read and PDF spool")
        output.write(b"\nendstream\nendobj\n")
        offsets.append(output.tell())
        output.write(b"5 0 obj\n")
        output.write(f"<< /Length {len(commands)} >>\nstream\n".encode("ascii"))
        output.write(commands)
        output.write(b"endstream\nendobj\n")
        xref = output.tell()
        output.write(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("ascii"))
        for offset in offsets[1:]:
            output.write(f"{offset:010d} 00000 n \n".encode("ascii"))
        output.write(f"trailer\n<< /Root 1 0 R /Size {len(offsets)} >>\nstartxref\n{xref}\n%%EOF\n".encode("ascii"))
    return destination.stat().st_size


def run_tool(argv: list[str], log_path: Path, timeout: int = TOOL_TIMEOUT_SECONDS) -> None:
    with log_path.open("wb") as log:
        try:
            result = subprocess.run(
                argv, stdin=subprocess.DEVNULL, stdout=log,
                stderr=subprocess.STDOUT, timeout=timeout, check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise OracleError(f"{Path(argv[0]).name} failed or timed out: {exc}") from exc
    if result.returncode != 0:
        with log_path.open("rb") as log:
            detail = log.read(MAX_LOG_BYTES).decode("utf-8", "replace")
        raise OracleError(f"{Path(argv[0]).name} exited {result.returncode}: {detail}")


def _pbm_token(source: BinaryIO) -> bytes:
    token = bytearray()
    while True:
        char = source.read(1)
        if not char:
            raise OracleError("PBM header ended before width and height")
        if char == b"#" and not token:
            source.readline(512)
            continue
        if char in b" \t\r\n":
            if token:
                return bytes(token)
            continue
        if len(token) >= 32:
            raise OracleError("PBM header token exceeds 32 bytes")
        token.extend(char)


def pbm_rows(path: Path, width: int, height: int):
    """Yield canonical top-to-bottom P4 rows; bit 1 means black, pad bits zero."""
    stride = (width + 7) // 8
    expected = stride * height
    if expected > MAX_BITMAP_BYTES:
        raise OracleError("PBM bitmap exceeds configured limit")
    try:
        size = path.stat().st_size
    except OSError as exc:
        raise OracleError(f"PBM output missing: {path.name}") from exc
    if size < expected or size > expected + 512:
        raise OracleError(f"PBM output size {size} differs from expected {expected} plus header")
    with path.open("rb") as source:
        magic = _pbm_token(source)
        try:
            actual_width = int(_pbm_token(source))
            actual_height = int(_pbm_token(source))
        except ValueError as exc:
            raise OracleError("PBM dimensions are not decimal integers") from exc
        if magic != b"P4" or (actual_width, actual_height) != (width, height):
            raise OracleError(f"PBM magic or dimensions differ: {magic!r} {actual_width}x{actual_height}")
        if source.tell() > 512:
            raise OracleError("PBM header exceeds 512 bytes")
        mask = 0xFF << (8 - width % 8) & 0xFF if width % 8 else 0xFF
        for row in range(height):
            data = source.read(stride)
            if len(data) != stride:
                raise OracleError(f"PBM row {row} is truncated")
            if mask != 0xFF:
                data = data[:-1] + bytes((data[-1] & mask,))
            yield data
        if source.read(1):
            raise OracleError("PBM has trailing bytes")


def compare_pbm(poppler: Path, mupdf: Path, width: int, height: int) -> tuple[str, int]:
    digest = hashlib.sha256()
    black_bits = 0
    with (
        closing(pbm_rows(poppler, width, height)) as left_rows,
        closing(pbm_rows(mupdf, width, height)) as right_rows,
    ):
        for row in range(height):
            left, right = next(left_rows), next(right_rows)
            if left != right:
                raise OracleError(f"normalized PBM pixel mismatch at row {row}")
            digest.update(left)
            black_bits += sum(byte.bit_count() for byte in left)
        for rows in (left_rows, right_rows):
            try:
                next(rows)
            except StopIteration:
                pass
            else:
                raise OracleError("PBM has extra rows")
    return digest.hexdigest(), black_bits


def check_qpdf_log(path: Path) -> None:
    """Require qpdf exit zero and no warning diagnostic before pixel use."""
    with path.open("rb") as source:
        for line in source:
            if re.search(rb"\bwarning\s*:", line, re.IGNORECASE):
                warning = line[:MAX_LOG_BYTES].decode("utf-8", "replace").strip()
                raise OracleError(f"qpdf warned while checking temporary PDF: {warning}")


def decode_case(
    case: dict, tools: dict[str, Path], expected_encoded_sha256: str
) -> tuple[str, int, int]:
    """Validate and decode one image; the temporary directory is always removed."""
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-oracle-") as directory:
        temporary = Path(directory)
        pdf = temporary / "image.pdf"
        pdf_bytes = write_pdf(case, pdf, expected_encoded_sha256)
        qpdf_log = temporary / "qpdf.log"
        run_tool([str(tools["qpdf"]), "--check", str(pdf)], qpdf_log)
        check_qpdf_log(qpdf_log)
        prefix = temporary / "poppler"
        run_tool([str(tools["pdfimages"]), str(pdf), str(prefix)], temporary / "pdfimages.log")
        poppler_files = list(temporary.glob("poppler-*"))
        if len(poppler_files) != 1 or poppler_files[0].suffix != ".pbm":
            raise OracleError(f"pdfimages emitted {len(poppler_files)} files instead of one PBM")
        mupdf = temporary / "mupdf.pbm"
        run_tool(
            [str(tools["mutool"]), "draw", "-q", "-F", "pbm", "-r", "72", "-o", str(mupdf), str(pdf)],
            temporary / "mutool.log",
        )
        pixel_sha, black_bits = compare_pbm(poppler_files[0], mupdf, case["width"], case["height"])
        return pixel_sha, black_bits, pdf_bytes


def manifest_for_cases(cases: list[dict], tools: dict[str, Path], matrix_path: Path) -> tuple[dict, list[dict], dict]:
    metadata = tool_metadata(tools)
    samples: dict[str, dict] = {}
    failures = []
    max_pdf_bytes = 0
    max_case_seconds = 0.0
    started = time.monotonic()
    for number, case in enumerate(cases, 1):
        sample = samples.setdefault(case["id"], {"id": case["id"], "path": case["path"], "variant": case["variant"], "source_sha256": case["source_sha256"], "images": []})
        case_started = time.monotonic()
        try:
            profile, encoded_sha = profile_and_hash(case)
            pixel_sha, black_bits, pdf_bytes = decode_case(case, tools, encoded_sha)
            max_pdf_bytes = max(max_pdf_bytes, pdf_bytes)
            sample["images"].append({"page": case["page"], "image": case["image"], "offset": case["offset"], "length": case["length"], "width": case["width"], "height": case["height"], "encoded_sha256": encoded_sha, "normalized_pixel_sha256": pixel_sha, "black_pixels": black_bits, "profile": profile, "status": "PASS"})
        except (OSError, OracleError) as exc:
            failures.append({"id": case["id"], "page": case["page"], "image": case["image"], "offset": case["offset"], "length": case["length"], "error": str(exc), "status": "FAIL"})
        max_case_seconds = max(max_case_seconds, time.monotonic() - case_started)
        if number % 50 == 0 or number == len(cases):
            print(f"JBIG2 oracle progress: {number}/{len(cases)} images; failures={len(failures)}", file=sys.stderr, flush=True)
    manifest = {"schema_version": 1, "matrix_sha256": sha256_file(matrix_path), "corpus_revision": "7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07", "normalization": "P4 PBM, top-to-bottom rows, MSB-first, 1=black, low row-padding bits zero", "backend_independence": "UNVERIFIED", "toolchains": metadata, "samples": [samples[key] for key in sorted(samples)]}
    report = {"status": "PASS" if not failures and sum(len(sample["images"]) for sample in samples.values()) == EXPECTED_IMAGES else "FAIL", "checked_images": len(cases), "tool_agreements": sum(len(sample["images"]) for sample in samples.values()), "failures": failures, "elapsed_seconds": round(time.monotonic() - started, 3), "max_case_seconds": round(max_case_seconds, 3), "max_temp_pdf_bytes": max_pdf_bytes, "backend_independence": "UNVERIFIED"}
    return manifest, failures, report


def reject_unknown_fields(value: dict, allowed: set[str], label: str) -> None:
    unexpected = value.keys() - allowed
    if unexpected:
        fields = ", ".join(sorted(map(str, unexpected)))
        raise OracleError(f"{label} has unknown fields: {fields}")


def validate_manifest(manifest: object, expected_images: int = EXPECTED_IMAGES) -> None:
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise OracleError("JBIG2 oracle manifest must have schema_version 1")
    reject_unknown_fields(
        manifest,
        {"schema_version", "matrix_sha256", "corpus_revision", "normalization",
         "backend_independence", "toolchains", "samples"},
        "JBIG2 oracle manifest",
    )
    if not isinstance(manifest.get("matrix_sha256"), str) or not HEX_SHA256.fullmatch(manifest["matrix_sha256"]):
        raise OracleError("JBIG2 oracle manifest has invalid matrix SHA-256")
    if manifest.get("backend_independence") != "UNVERIFIED":
        raise OracleError("JBIG2 oracle manifest overstates backend independence")
    if not isinstance(manifest.get("toolchains"), dict) or not isinstance(manifest.get("samples"), list):
        raise OracleError("JBIG2 oracle manifest lacks toolchains or samples")
    toolchains = manifest["toolchains"]
    reject_unknown_fields(toolchains, {"tools", "backend_evidence"}, "toolchains")
    if "tools" in toolchains:
        tools = toolchains["tools"]
        if not isinstance(tools, dict):
            raise OracleError("toolchains.tools must be an object")
        reject_unknown_fields(tools, {"qpdf", "pdfimages", "mutool"}, "toolchains.tools")
        for name, tool in tools.items():
            if not isinstance(tool, dict):
                raise OracleError(f"toolchains.tools.{name} must be an object")
            reject_unknown_fields(
                tool, {"version", "binary_sha256"}, f"toolchains.tools.{name}"
            )
    if "backend_evidence" in toolchains:
        evidence = toolchains["backend_evidence"]
        if not isinstance(evidence, dict):
            raise OracleError("toolchains.backend_evidence must be an object")
        reject_unknown_fields(
            evidence,
            {"dynamic_libjbig2dec", "implementation_independence", "meaning"},
            "toolchains.backend_evidence",
        )
        if "dynamic_libjbig2dec" in evidence:
            dynamic = evidence["dynamic_libjbig2dec"]
            if not isinstance(dynamic, dict):
                raise OracleError("toolchains.backend_evidence.dynamic_libjbig2dec must be an object")
            reject_unknown_fields(
                dynamic, {"mutool", "pdfimages"},
                "toolchains.backend_evidence.dynamic_libjbig2dec",
            )
    if expected_images == EXPECTED_IMAGES:
        if manifest["matrix_sha256"] != sha256_file(DEFAULT_MATRIX):
            raise OracleError("JBIG2 oracle manifest differs from current matrix SHA-256")
        if manifest.get("corpus_revision") != "7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07":
            raise OracleError("JBIG2 oracle manifest has an unexpected corpus revision")
        if manifest.get("normalization") != "P4 PBM, top-to-bottom rows, MSB-first, 1=black, low row-padding bits zero":
            raise OracleError("JBIG2 oracle manifest has an unexpected pixel layout")
        tools = manifest["toolchains"].get("tools")
        if not isinstance(tools, dict) or set(tools) != {"qpdf", "pdfimages", "mutool"}:
            raise OracleError("JBIG2 oracle manifest lacks tool versions")
        for name, tool in tools.items():
            if not isinstance(tool, dict) or not isinstance(tool.get("version"), str) or not tool["version"]:
                raise OracleError(f"JBIG2 oracle manifest has invalid {name} version")
            if not isinstance(tool.get("binary_sha256"), str) or not HEX_SHA256.fullmatch(tool["binary_sha256"]):
                raise OracleError(f"JBIG2 oracle manifest has invalid {name} binary SHA-256")
        if manifest["toolchains"].get("backend_evidence", {}).get("implementation_independence") != "UNVERIFIED":
            raise OracleError("JBIG2 oracle manifest overstates backend independence")
    count = 0
    keys = set()
    matrix_rows = {row["id"]: row for row in conformance.load_matrix(DEFAULT_MATRIX)} if expected_images == EXPECTED_IMAGES else {}
    for sample in manifest["samples"]:
        if not isinstance(sample, dict) or not isinstance(sample.get("id"), str) or sample["id"] in keys:
            raise OracleError("JBIG2 oracle manifest has duplicate or invalid sample ID")
        reject_unknown_fields(
            sample, {"id", "path", "variant", "source_sha256", "images"},
            "JBIG2 oracle manifest sample",
        )
        keys.add(sample["id"])
        if not isinstance(sample.get("source_sha256"), str) or not HEX_SHA256.fullmatch(sample["source_sha256"]):
            raise OracleError("JBIG2 oracle manifest has invalid source SHA-256")
        if expected_images == EXPECTED_IMAGES and (
            sample["id"] not in EXPECTED_TYPE3
            or sample["source_sha256"] != matrix_rows[sample["id"]]["sha256"]
            or sample.get("path") != matrix_rows[sample["id"]]["path"]
            or sample.get("variant") != matrix_rows[sample["id"]]["variant"]
        ):
            raise OracleError("JBIG2 oracle manifest source identity differs from matrix")
        images = sample.get("images")
        if not isinstance(images, list):
            raise OracleError("JBIG2 oracle manifest sample lacks images")
        if expected_images == EXPECTED_IMAGES and len(images) != EXPECTED_TYPE3[sample["id"]]:
            raise OracleError(f"{sample['id']}: JBIG2 oracle manifest image count differs")
        coords = set()
        for image in images:
            if not isinstance(image, dict) or image.get("status") != "PASS":
                raise OracleError("JBIG2 oracle manifest includes an unverified image")
            reject_unknown_fields(
                image,
                {"page", "image", "offset", "length", "width", "height",
                 "encoded_sha256", "normalized_pixel_sha256", "black_pixels",
                 "profile", "status"},
                "JBIG2 oracle manifest image",
            )
            coordinate = (image.get("page"), image.get("image"))
            if coordinate in coords:
                raise OracleError("JBIG2 oracle manifest has duplicate image coordinate")
            coords.add(coordinate)
            for field in ("encoded_sha256", "normalized_pixel_sha256"):
                if not isinstance(image.get(field), str) or not HEX_SHA256.fullmatch(image[field]):
                    raise OracleError(f"JBIG2 oracle manifest has invalid {field}")
            for field in ("page", "image", "length", "width", "height"):
                _positive_int(image.get(field), field)
            _nonnegative_int(image.get("offset"), "offset")
            if not isinstance(image.get("profile"), dict):
                raise OracleError("JBIG2 oracle manifest image lacks profile")
            reject_unknown_fields(
                image["profile"],
                {"page_flags", "symbol_flags", "symbol_counts", "text_flags",
                 "generic_flags", "text_flag_anomaly"},
                "JBIG2 oracle manifest image profile",
            )
            if expected_images == EXPECTED_IMAGES:
                anomaly = (sample["id"], image["page"], image["image"]) == ANOMALY_COORDINATE
                profile = image["profile"]
                if (profile.get("text_flags") == "0xa40c") != anomaly or (
                    profile.get("text_flag_anomaly") is not None
                ) != anomaly:
                    raise OracleError("JBIG2 oracle manifest text flag anomaly is missing or misplaced")
                black = _nonnegative_int(image.get("black_pixels"), "black_pixels")
                if black > image["width"] * image["height"]:
                    raise OracleError("JBIG2 oracle manifest black pixel count exceeds dimensions")
            count += 1
    if count != expected_images:
        raise OracleError(f"JBIG2 oracle manifest contains {count} images, expected {expected_images}")
    if expected_images == EXPECTED_IMAGES and keys != set(EXPECTED_TYPE3):
        raise OracleError("JBIG2 oracle manifest lacks expected source IDs")


def compare_manifest(observed: dict, baseline: dict) -> list[str]:
    """Report semantic changes without requiring identical decoder builds."""
    differences = []
    for field in ("schema_version", "matrix_sha256", "corpus_revision", "normalization", "backend_independence"):
        if observed.get(field) != baseline.get(field):
            differences.append(field)
    expected_samples = {sample["id"]: sample for sample in baseline["samples"]}
    current_samples = {sample["id"]: sample for sample in observed["samples"]}
    for sample_id in sorted(expected_samples.keys() | current_samples.keys()):
        old, new = expected_samples.get(sample_id), current_samples.get(sample_id)
        if old is None or new is None:
            differences.append(sample_id)
            continue
        for field in ("path", "variant", "source_sha256"):
            if old.get(field) != new.get(field):
                differences.append(f"{sample_id}: {field}")
        old_images = {(image["page"], image["image"]): image for image in old["images"]}
        new_images = {(image["page"], image["image"]): image for image in new["images"]}
        for page, index in sorted(old_images.keys() | new_images.keys()):
            if old_images.get((page, index)) != new_images.get((page, index)):
                differences.append(f"{sample_id}: page {page} image {index}")
    return differences


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--corpus-dir", type=Path, default=Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None)
    parser.add_argument("--qpdf", default="qpdf")
    parser.add_argument("--pdfimages", default="pdfimages")
    parser.add_argument("--mutool", default="mutool")
    parser.add_argument("--write-manifest", action="store_true", help="replace the metadata manifest only after all 546 images agree")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report: dict
    try:
        if args.manifest.exists():
            validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
        if args.corpus_dir is None:
            report = {"status": "NOT_RUN", "reason": "external CAJSamples corpus is unset", "tool_agreements": 0, "expected_images": EXPECTED_IMAGES}
        elif not args.corpus_dir.is_dir():
            report = {"status": "NOT_RUN", "reason": "external CAJSamples corpus is unavailable", "tool_agreements": 0, "expected_images": EXPECTED_IMAGES}
        else:
            rows, paths = validate_sources(DEFAULT_MATRIX, args.corpus_dir)
            tools = {name: tool_path(getattr(args, name)) for name in ("qpdf", "pdfimages", "mutool")}
            missing = [name for name, path in tools.items() if path is None]
            if missing:
                report = {"status": "NOT_RUN", "reason": f"external oracle tool unavailable: {', '.join(missing)}", "tool_agreements": 0, "expected_images": EXPECTED_IMAGES, "source_hashes_checked": len(rows)}
            else:
                inventory = run_directory_inventory(args.corpus_dir)
                cases = checked_cases(inventory, rows, paths)
                manifest, failures, report = manifest_for_cases(cases, tools, DEFAULT_MATRIX)
                report["toolchains"] = manifest["toolchains"]
                report["source_hashes_checked"] = len(rows)
                report["expected_images"] = EXPECTED_IMAGES
                if not failures and report["status"] == "PASS":
                    try:
                        validate_sources(DEFAULT_MATRIX, args.corpus_dir)
                    except OracleError as exc:
                        report.update(status="FAIL", source_postcheck_error=str(exc))
                    else:
                        validate_manifest(manifest)
                        if args.write_manifest:
                            temporary = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
                            temporary.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                            temporary.replace(args.manifest)
                        elif args.manifest.exists():
                            baseline = json.loads(args.manifest.read_text(encoding="utf-8"))
                            report["toolchain_drift"] = manifest["toolchains"] != baseline.get("toolchains")
                            differences = compare_manifest(manifest, baseline)
                            if differences:
                                report.update(status="FAIL", manifest_differences=differences)
                        else:
                            report.update(status="FAIL", reason="metadata manifest is absent; use --write-manifest after review")
    except (OSError, ValueError, conformance.ConformanceError, OracleError, json.JSONDecodeError) as exc:
        report = {"status": "FAIL", "error": str(exc), "tool_agreements": 0, "expected_images": EXPECTED_IMAGES}
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(f"JBIG2 pixel oracle [{report['status']}]: agreement={report.get('tool_agreements', 0)}/{EXPECTED_IMAGES}")
        if report.get("reason") or report.get("error"):
            print(report.get("reason") or report.get("error"), file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
