#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, black-box pixel oracle for HN/C8 type-0 images.

No external document, decoded bitmap, or native library is distributed with
this script. The ordinary clean-clone run reports NOT_RUN. A caller supplies
the SHA-256-pinned CAJSamples corpus and, separately, the pinned external
decoder library. The library is loaded only in a timeout-isolated child process.

Hash definitions (version 1):

* ``encoded_sha256`` covers the complete image slice: 40 DIB header bytes,
  eight palette bytes, and the compressed payload.
* ``raw_stride_sha256`` covers the decoder's output buffer as returned:
  ``height`` consecutive rows of ``stride`` bytes. No row reversal occurs.
* ``visible_bits_sha256`` concatenates, in that same memory order, each row's
  first ``ceil(width / 8)`` bytes, clearing unused low bits of its final byte.
  Four-byte stride padding is excluded. The bit order is MSB-first.

The signed DIB height must be positive in this measured subset. These hashes
do not claim that the first decoder-memory row is the top of the page; that
orientation needs an independent viewer or PDF comparison.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import BinaryIO

import conformance


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MATRIX = ROOT / "tests/conformance/matrix.json"
DEFAULT_MANIFEST = ROOT / "tests/conformance/jbig1_oracle.json"
HEX_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
HEX_SHA1 = re.compile(r"[0-9a-f]{40}\Z")
CHUNK_BYTES = 1024 * 1024
PAGE_ROW_BYTES = 20
IMAGE_ROW_BYTES = 12
DIB_AND_PALETTE_BYTES = 48
MAX_PAGES = 100_000
MAX_IMAGES_PER_PAGE = 10_000
MAX_IMAGES_PER_FILE = 100_000
DEFAULT_MAX_ENCODED_BYTES = 64 * 1024 * 1024
DEFAULT_MAX_BITMAP_BYTES = 128 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS = 20.0
BASELINE_TYPE0_IMAGES = 1400
BASELINE_INVALID_RECORDS = 3
C_INT_MAX = 2**31 - 1
REFERENCE_REVISION = "8cbc3c5721acb762f739434eb3d206171dbb022a"
PINNED_LIBRARY_SHA256 = "d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6"
COMPILER_SHA256 = "6b3696e4dcb85e1c949c732a02befa50e3983ecf94ce7e8e58d9d503b954b79d"
COMPILER_VERSION = "Debian g++ 14.2.0 (x86_64-linux-gnu)"
ORACLE_SYMBOL = "jbigDecode"
ORACLE_ABI = (
    "void jbigDecode(void *compressed, int compressed_length, int height, "
    "int width_bits, int stride_bytes, void *output)"
)
PREFILLS = ["00", "a5"]
RAW_LAYOUT = "all emitted rows in decoder memory order, including 32-bit stride padding"
VISIBLE_LAYOUT = (
    "each row cropped to ceil(width/8) bytes, unused low bits of its final byte "
    "masked to zero, no row reversal"
)


class OracleError(Exception):
    """A checked container, manifest, or decoder operation failed."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest()


def require_span(offset: int, length: int, file_size: int, context: str) -> None:
    if offset < 0 or length < 0 or offset > file_size or length > file_size - offset:
        raise OracleError(f"{context} is outside the source file")


def read_exact(source: BinaryIO, offset: int, length: int, file_size: int, context: str) -> bytes:
    require_span(offset, length, file_size, context)
    source.seek(offset)
    data = source.read(length)
    if len(data) != length:
        raise OracleError(f"{context} changed or ended during reading")
    return data


def hash_range(source: BinaryIO, offset: int, length: int, file_size: int) -> str:
    require_span(offset, length, file_size, "image bytes")
    source.seek(offset)
    digest = hashlib.sha256()
    remaining = length
    while remaining:
        chunk = source.read(min(remaining, CHUNK_BYTES))
        if not chunk:
            raise OracleError("image bytes changed or ended during hashing")
        digest.update(chunk)
        remaining -= len(chunk)
    return digest.hexdigest()


def u16(data: bytes, offset: int) -> int:
    return int.from_bytes(data[offset : offset + 2], "little")


def i16(data: bytes, offset: int) -> int:
    return int.from_bytes(data[offset : offset + 2], "little", signed=True)


def u32(data: bytes, offset: int) -> int:
    return int.from_bytes(data[offset : offset + 4], "little")


def i32(data: bytes, offset: int) -> int:
    return int.from_bytes(data[offset : offset + 4], "little", signed=True)


def validate_image_dimensions(header: bytes, max_bitmap_bytes: int) -> tuple[int, int, int]:
    if len(header) != DIB_AND_PALETTE_BYTES or u32(header, 0) != 40:
        raise OracleError("type-0 image lacks a 40-byte BITMAPINFOHEADER and eight-byte palette")
    width = i32(header, 4)
    height = i32(header, 8)
    if width <= 0 or height <= 0:
        raise OracleError("type-0 DIB width and signed height must be positive")
    if u16(header, 12) != 1 or u16(header, 14) != 1:
        raise OracleError("type-0 DIB must have one plane and one bit per pixel")
    if u32(header, 16) != 0:
        raise OracleError("type-0 DIB compression must be zero")
    if header[40:48] != b"\xff\xff\xff\x00\x00\x00\x00\x00":
        raise OracleError("type-0 DIB palette differs from the observed white/black palette")
    stride = ((width + 31) // 32) * 4
    bitmap_bytes = stride * height
    if max(width, height, stride, bitmap_bytes) > C_INT_MAX:
        raise OracleError("type-0 DIB dimensions exceed the oracle C ABI")
    if bitmap_bytes > max_bitmap_bytes:
        raise OracleError(
            f"type-0 decoded bitmap exceeds limit ({bitmap_bytes} > {max_bitmap_bytes})"
        )
    return width, height, stride


def page_table(source: BinaryIO, file_size: int) -> tuple[str, int, int]:
    signature = read_exact(source, 0, 8, file_size, "format signature")
    if signature[:4] == b"\xc8\x00\x00\x00":
        variant, count_at, table_at = "C8", 0x08, 0x50
    elif signature[:4] == b"HN\x00\x00":
        variant, count_at = "HN", 0x90
        marker = signature[4:8]
        if marker == b"\x90\x01\x00\x00":
            outline_count = i32(
                read_exact(source, 0x158, 4, file_size, "HN outline count"), 0
            )
            if not 0 <= outline_count <= MAX_PAGES:
                raise OracleError("HN outline count is outside the supported range")
            table_at = 0x15C + 308 * outline_count
        elif marker == b"\xc8\x00\x00\x00":
            table_at = 0xD8
        else:
            raise OracleError(f"unsupported HN variant marker {marker.hex()}")
    else:
        raise OracleError("source is neither the observed HN nor C8 variant")
    count = i32(read_exact(source, count_at, 4, file_size, "page count"), 0)
    if not 0 < count <= MAX_PAGES:
        raise OracleError("page count is outside the supported range")
    require_span(table_at, count * PAGE_ROW_BYTES, file_size, "page table")
    return variant, count, table_at


def discovery_failure(
    sample_id: str, page: int | None, kind: str, detail: str, image: int | None = None
) -> dict:
    failure = {"sample_id": sample_id, "page": page, "kind": kind, "detail": detail}
    if image is not None:
        failure["image"] = image
    return failure


def discover_sample(
    row: dict, path: Path, max_encoded_bytes: int, max_bitmap_bytes: int
) -> tuple[dict, list[dict], dict[int, int]]:
    """Walk one verified source without materializing an encoded image."""
    sample = {
        "id": row["id"],
        "path": row["path"],
        "source_sha256": row["sha256"],
        "images": [],
    }
    failures: list[dict] = []
    image_types: dict[int, int] = {}
    with path.open("rb") as source:
        before = os.fstat(source.fileno())
        file_size = before.st_size
        if file_size != row["size_bytes"]:
            return sample, [discovery_failure(row["id"], None, "container", "source size changed")], {}
        try:
            variant, page_count, table_at = page_table(source, file_size)
            if variant != row["variant"]:
                raise OracleError(f"detected {variant}, expected matrix variant {row['variant']}")
        except OracleError as exc:
            return sample, [discovery_failure(row["id"], None, "container", str(exc))], {}
        table_end = table_at + page_count * PAGE_ROW_BYTES
        total_images = 0
        for page in range(1, page_count + 1):
            try:
                page_row = read_exact(
                    source,
                    table_at + (page - 1) * PAGE_ROW_BYTES,
                    PAGE_ROW_BYTES,
                    file_size,
                    f"page {page} index",
                )
                text_offset = i32(page_row, 0)
                text_length = i32(page_row, 4)
                image_count = i16(page_row, 8)
                require_span(text_offset, text_length, file_size, f"page {page} text")
                if image_count < 0:
                    raise OracleError(f"page {page} image count is negative")
                if image_count > MAX_IMAGES_PER_PAGE:
                    raise OracleError(f"page {page} image count exceeds {MAX_IMAGES_PER_PAGE}")
                total_images += image_count
                if total_images > MAX_IMAGES_PER_FILE:
                    raise OracleError(f"image count exceeds {MAX_IMAGES_PER_FILE} per file")
                if image_count and text_offset + text_length < table_end:
                    raise OracleError(f"page {page} image records overlap the page table")
                image_record_at = text_offset + text_length
            except OracleError as exc:
                failures.append(discovery_failure(row["id"], page, "page_index", str(exc)))
                continue
            for image in range(1, image_count + 1):
                try:
                    record = read_exact(
                        source,
                        image_record_at,
                        IMAGE_ROW_BYTES,
                        file_size,
                        f"page {page} image {image} record",
                    )
                    image_type = i32(record, 0)
                    image_offset = i32(record, 4)
                    image_length = i32(record, 8)
                    if image_type < 0:
                        raise OracleError(f"page {page} image {image} type is negative")
                    require_span(image_offset, image_length, file_size, f"page {page} image {image}")
                    if image_offset < image_record_at + IMAGE_ROW_BYTES or image_length == 0:
                        raise OracleError(f"page {page} image {image} overlaps its record or is empty")
                    image_record_at = image_offset + image_length
                    image_types[image_type] = image_types.get(image_type, 0) + 1
                    if image_type != 0:
                        continue
                    if image_length <= DIB_AND_PALETTE_BYTES:
                        raise OracleError(f"page {page} image {image} has no compressed payload")
                    if image_length > max_encoded_bytes:
                        raise OracleError(
                            f"page {page} image {image} exceeds encoded limit "
                            f"({image_length} > {max_encoded_bytes})"
                        )
                    header = read_exact(
                        source,
                        image_offset,
                        DIB_AND_PALETTE_BYTES,
                        file_size,
                        f"page {page} image {image} DIB",
                    )
                    width, height, stride = validate_image_dimensions(header, max_bitmap_bytes)
                    sample["images"].append(
                        {
                            "page": page,
                            "image": image,
                            "offset": image_offset,
                            "length": image_length,
                            "encoded_sha256": hash_range(
                                source, image_offset, image_length, file_size
                            ),
                            "width": width,
                            "height": height,
                            "stride": stride,
                            "decoder_result": "NOT_RUN",
                            "raw_stride_sha256": None,
                            "visible_bits_sha256": None,
                        }
                    )
                except OracleError as exc:
                    failures.append(
                        discovery_failure(row["id"], page, "image_record", str(exc), image)
                    )
                    break
        after = os.fstat(source.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ino) != (
            after.st_size,
            after.st_mtime_ns,
            after.st_ino,
        ):
            failures.append(
                discovery_failure(row["id"], None, "container", "source changed during discovery")
            )
    return sample, failures, image_types


def oracle_metadata(timeout_seconds: float) -> dict:
    return {
        "reference_revision": REFERENCE_REVISION,
        "library_sha256": PINNED_LIBRARY_SHA256,
        "compiler_sha256": COMPILER_SHA256,
        "compiler_version": COMPILER_VERSION,
        "symbol": ORACLE_SYMBOL,
        "abi": ORACLE_ABI,
        "prefills": PREFILLS,
        "timeout_seconds": timeout_seconds,
        "hash_layout": {
            "raw_stride_sha256": RAW_LAYOUT,
            "visible_bits_sha256": VISIBLE_LAYOUT,
        },
    }


def _is_sha256(value: object) -> bool:
    return isinstance(value, str) and HEX_SHA256.fullmatch(value) is not None


def _positive_int(value: object) -> bool:
    return type(value) is int and value > 0


def load_manifest(path: Path) -> dict:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise OracleError(f"cannot read JBIG1 manifest {path}: {exc}") from exc
    if (
        not isinstance(manifest, dict)
        or type(manifest.get("schema_version")) is not int
        or manifest["schema_version"] != 1
    ):
        raise OracleError("JBIG1 manifest must have schema_version 1")
    if not _is_sha256(manifest.get("corpus_matrix_sha256")):
        raise OracleError("JBIG1 manifest has no valid corpus_matrix_sha256")
    oracle = manifest.get("oracle")
    if not isinstance(oracle, dict):
        raise OracleError("JBIG1 manifest has no oracle metadata")
    for key in ("reference_revision", "library_sha256", "compiler_sha256"):
        pattern = HEX_SHA1 if key == "reference_revision" else HEX_SHA256
        value = oracle.get(key)
        if not isinstance(value, str) or pattern.fullmatch(value) is None:
            raise OracleError(f"JBIG1 manifest has invalid oracle.{key}")
    for key, expected in (
        ("reference_revision", REFERENCE_REVISION),
        ("library_sha256", PINNED_LIBRARY_SHA256),
        ("compiler_sha256", COMPILER_SHA256),
        ("compiler_version", COMPILER_VERSION),
        ("symbol", ORACLE_SYMBOL),
        ("abi", ORACLE_ABI),
        ("prefills", PREFILLS),
        (
            "hash_layout",
            {
                "raw_stride_sha256": RAW_LAYOUT,
                "visible_bits_sha256": VISIBLE_LAYOUT,
            },
        ),
    ):
        if oracle.get(key) != expected:
            raise OracleError(f"JBIG1 manifest oracle.{key} does not match the pinned protocol")
    timeout = oracle.get("timeout_seconds")
    if type(timeout) not in (int, float) or not 0 < timeout <= 300:
        raise OracleError("JBIG1 manifest oracle.timeout_seconds must be in (0, 300]")

    samples = manifest.get("samples")
    if not isinstance(samples, list) or not samples:
        raise OracleError("JBIG1 manifest must contain a nonempty samples array")
    seen_ids: set[str] = set()
    for sample in samples:
        if not isinstance(sample, dict):
            raise OracleError("JBIG1 manifest sample is not an object")
        sample_id = sample.get("id")
        if not isinstance(sample_id, str) or not sample_id or sample_id in seen_ids:
            raise OracleError(f"JBIG1 manifest has an invalid or duplicate sample ID {sample_id!r}")
        seen_ids.add(sample_id)
        conformance.relative_path(sample.get("path"))
        if not _is_sha256(sample.get("source_sha256")):
            raise OracleError(f"{sample_id}: invalid source_sha256")
        images = sample.get("images")
        if not isinstance(images, list):
            raise OracleError(f"{sample_id}: images must be an array")
        previous = (0, 0)
        for image in images:
            if not isinstance(image, dict):
                raise OracleError(f"{sample_id}: image record is not an object")
            if not all(_positive_int(image.get(key)) for key in ("page", "image", "length", "width", "height", "stride")):
                raise OracleError(f"{sample_id}: invalid image page/index/length/dimensions")
            if type(image.get("offset")) is not int or image["offset"] < 0:
                raise OracleError(f"{sample_id}: invalid image offset")
            if not _is_sha256(image.get("encoded_sha256")):
                raise OracleError(f"{sample_id}: invalid encoded_sha256")
            if image["stride"] != ((image["width"] + 31) // 32) * 4:
                raise OracleError(f"{sample_id}: inconsistent image stride")
            key = (image["page"], image["image"])
            if key <= previous:
                raise OracleError(f"{sample_id}: image records must be unique and page ordered")
            previous = key
            result = image.get("decoder_result")
            if result not in ("PASS", "FAIL", "NOT_RUN"):
                raise OracleError(f"{sample_id}: invalid decoder_result")
            hashes = (image.get("raw_stride_sha256"), image.get("visible_bits_sha256"))
            if result == "PASS" and not all(_is_sha256(value) for value in hashes):
                raise OracleError(f"{sample_id}: PASS image lacks decoded hashes")
            if result != "PASS" and any(value is not None for value in hashes):
                raise OracleError(f"{sample_id}: undecoded image must not claim decoded hashes")
    failures = manifest.get("discovery_failures")
    if not isinstance(failures, list):
        raise OracleError("JBIG1 manifest discovery_failures must be an array")
    for failure in failures:
        if (
            not isinstance(failure, dict)
            or failure.get("sample_id") not in seen_ids
            or failure.get("kind") not in ("container", "page_index", "image_record")
            or not isinstance(failure.get("detail"), str)
            or not failure["detail"]
            or ("page" not in failure)
            or (failure["page"] is not None and not _positive_int(failure["page"]))
            or ("image" in failure and not _positive_int(failure["image"]))
        ):
            raise OracleError("JBIG1 manifest contains an invalid discovery failure")
    return manifest


DISCOVERY_IMAGE_FIELDS = (
    "page",
    "image",
    "offset",
    "length",
    "encoded_sha256",
    "width",
    "height",
    "stride",
)


def compare_discovery(observed: dict, expected: dict) -> list[str]:
    problems: list[str] = []
    if observed["corpus_matrix_sha256"] != expected["corpus_matrix_sha256"]:
        problems.append("corpus matrix SHA-256 differs from the manifest")
    expected_samples = {sample["id"]: sample for sample in expected["samples"]}
    observed_samples = {sample["id"]: sample for sample in observed["samples"]}
    if expected_samples.keys() != observed_samples.keys():
        problems.append("HN/C8 sample IDs differ from the manifest")
    for sample_id in expected_samples.keys() & observed_samples.keys():
        actual, wanted = observed_samples[sample_id], expected_samples[sample_id]
        if any(actual[key] != wanted[key] for key in ("path", "source_sha256")):
            problems.append(f"{sample_id}: source path or hash differs from the manifest")
            continue
        actual_images = actual["images"]
        wanted_images = wanted["images"]
        if len(actual_images) != len(wanted_images):
            problems.append(
                f"{sample_id}: found {len(actual_images)} type-0 images, "
                f"manifest has {len(wanted_images)}"
            )
            continue
        for actual_image, wanted_image in zip(actual_images, wanted_images, strict=True):
            if any(actual_image[field] != wanted_image[field] for field in DISCOVERY_IMAGE_FIELDS):
                problems.append(
                    f"{sample_id}: type-0 image metadata/hash differs at "
                    f"page {actual_image['page']} image {actual_image['image']}"
                )
                break
    actual_failed = sorted(
        observed["discovery_failures"], key=lambda item: (item["sample_id"], item["page"] or 0, item.get("image", 0))
    )
    wanted_failed = sorted(
        expected["discovery_failures"], key=lambda item: (item["sample_id"], item["page"] or 0, item.get("image", 0))
    )
    if actual_failed != wanted_failed:
        problems.append("discovery failures differ from the manifest")
    return problems


def validate_manifest_inventory(
    manifest: dict, matrix_rows: list[dict], required_image_count: int | None
) -> int:
    """Check the committed baseline before an optional corpus can yield NOT_RUN."""
    matrix_by_id = {row["id"]: row for row in matrix_rows}
    manifest_by_id = {sample["id"]: sample for sample in manifest["samples"]}
    if matrix_by_id.keys() != manifest_by_id.keys():
        raise OracleError("JBIG1 manifest sample IDs differ from the HN/C8 matrix rows")
    for sample_id, row in matrix_by_id.items():
        sample = manifest_by_id[sample_id]
        if sample["path"] != row["path"] or sample["source_sha256"] != row["sha256"]:
            raise OracleError(f"{sample_id}: JBIG1 manifest path or SHA-256 differs from the matrix")
    image_count = sum(len(sample["images"]) for sample in manifest["samples"])
    if required_image_count is not None and image_count != required_image_count:
        raise OracleError(
            f"JBIG1 manifest has {image_count} type-0 images; expected {required_image_count}"
        )
    if required_image_count is not None and any(
        image["decoder_result"] != "PASS"
        for sample in manifest["samples"]
        for image in sample["images"]
    ):
        raise OracleError("committed JBIG1 manifest must mark all decoded images PASS")
    if required_image_count is not None and len(manifest["discovery_failures"]) != BASELINE_INVALID_RECORDS:
        raise OracleError(
            "committed JBIG1 manifest has "
            f"{len(manifest['discovery_failures'])} expected invalid records; "
            f"expected {BASELINE_INVALID_RECORDS}"
        )
    return image_count


def bitmap_hashes(raw: bytes | memoryview, width: int, height: int, stride: int) -> tuple[str, str]:
    if len(raw) != stride * height or stride != ((width + 31) // 32) * 4:
        raise OracleError("decoder output has inconsistent bitmap dimensions")
    raw_hash = hashlib.sha256(raw).hexdigest()
    visible_hash = hashlib.sha256()
    visible_bytes = (width + 7) // 8
    remainder = width % 8
    mask = (0xFF << (8 - remainder)) & 0xFF if remainder else 0xFF
    for row in range(height):
        start = row * stride
        visible_hash.update(raw[start : start + visible_bytes - 1])
        visible_hash.update(bytes((raw[start + visible_bytes - 1] & mask,)))
    return raw_hash, visible_hash.hexdigest()


def decode_worker_once(arguments: dict) -> dict:
    """Call the black-box ABI once, only inside the isolated worker process."""
    import ctypes

    library_path = Path(arguments["library"])
    source_path = Path(arguments["source"])
    offset = arguments["offset"]
    length = arguments["length"]
    width = arguments["width"]
    height = arguments["height"]
    stride = arguments["stride"]
    prefill = arguments["prefill"]
    max_encoded_bytes = arguments["max_encoded_bytes"]
    max_bitmap_bytes = arguments["max_bitmap_bytes"]
    if any(type(value) is not int for value in (
        offset, length, width, height, stride, prefill, max_encoded_bytes, max_bitmap_bytes
    )):
        raise OracleError("worker arguments must be integers")
    if prefill not in (0, 0xA5):
        raise OracleError("worker prefill is outside the pinned protocol")
    if length <= DIB_AND_PALETTE_BYTES or length > max_encoded_bytes:
        raise OracleError("worker encoded image exceeds the bounded input profile")
    if length - DIB_AND_PALETTE_BYTES > C_INT_MAX:
        raise OracleError("compressed input length exceeds the oracle C ABI")
    with source_path.open("rb") as source:
        file_size = os.fstat(source.fileno()).st_size
        image_bytes = read_exact(source, offset, length, file_size, "worker image")
    if hashlib.sha256(image_bytes).hexdigest() != arguments["encoded_sha256"]:
        raise OracleError("encoded image changed after inventory/discovery")
    dimensions = validate_image_dimensions(image_bytes[:DIB_AND_PALETTE_BYTES], max_bitmap_bytes)
    if dimensions != (width, height, stride):
        raise OracleError("DIB dimensions changed after discovery")

    input_buffer = ctypes.create_string_buffer(image_bytes, len(image_bytes))
    del image_bytes
    compressed_length = length - DIB_AND_PALETTE_BYTES
    bitmap_bytes = stride * height
    guard_bytes = 4096
    storage = (ctypes.c_ubyte * (bitmap_bytes + 2 * guard_bytes))()
    address = ctypes.addressof(storage)
    output_address = address + guard_bytes
    ctypes.memset(address, 0x5A, guard_bytes)
    ctypes.memset(output_address, prefill, bitmap_bytes)
    ctypes.memset(output_address + bitmap_bytes, 0xC3, guard_bytes)
    try:
        library = ctypes.CDLL(str(library_path))
        decoder = getattr(library, ORACLE_SYMBOL)
    except (OSError, AttributeError) as exc:
        raise OracleError(f"cannot load pinned oracle symbol: {exc}") from exc
    decoder.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_void_p,
    ]
    decoder.restype = None
    decoder(
        ctypes.c_void_p(ctypes.addressof(input_buffer) + DIB_AND_PALETTE_BYTES),
        compressed_length,
        height,
        width,
        stride,
        ctypes.c_void_p(output_address),
    )
    if ctypes.string_at(address, guard_bytes) != b"\x5A" * guard_bytes or ctypes.string_at(
        output_address + bitmap_bytes, guard_bytes
    ) != b"\xC3" * guard_bytes:
        raise OracleError("oracle wrote beyond the bounded output buffer")
    raw = memoryview(storage).cast("B")[guard_bytes : guard_bytes + bitmap_bytes]
    raw_hash, visible_hash = bitmap_hashes(raw, width, height, stride)
    return {
        "status": "PASS",
        "raw_stride_sha256": raw_hash,
        "visible_bits_sha256": visible_hash,
    }


def worker_main(argument: str) -> int:
    try:
        payload = json.loads(argument)
        if not isinstance(payload, dict):
            raise OracleError("worker payload must be an object")
        result = decode_worker_once(payload)
    except (OracleError, OSError, TypeError, ValueError, KeyError, MemoryError) as exc:
        print(json.dumps({"status": "FAIL", "reason": str(exc)}))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


def worker_process(arguments: dict, timeout_seconds: float) -> dict:
    command = [sys.executable, str(Path(__file__).resolve()), "--worker", json.dumps(arguments)]
    try:
        completed = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {"status": "FAIL", "reason": f"oracle timed out after {timeout_seconds}s"}
    except OSError as exc:
        return {"status": "FAIL", "reason": f"oracle worker could not start: {exc}"}
    if completed.returncode < 0:
        return {
            "status": "FAIL",
            "reason": f"oracle worker crashed (signal {-completed.returncode})",
        }
    if len(completed.stdout) > 4096:
        return {"status": "FAIL", "reason": "oracle worker response exceeds 4096 bytes"}
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return {
            "status": "FAIL",
            "reason": f"oracle worker exited {completed.returncode} without a JSON result",
        }
    if not isinstance(result, dict) or result.get("status") not in ("PASS", "FAIL"):
        return {"status": "FAIL", "reason": "oracle worker returned an invalid result"}
    if completed.returncode != 0:
        return {
            "status": "FAIL",
            "reason": result.get("reason") or f"oracle worker exited {completed.returncode}",
        }
    if result["status"] != "PASS" or any(
        not _is_sha256(result.get(key))
        for key in ("raw_stride_sha256", "visible_bits_sha256")
    ):
        return {"status": "FAIL", "reason": "oracle worker returned invalid pixel hashes"}
    return result


def decode_image(
    library: Path,
    source: Path,
    image: dict,
    timeout_seconds: float,
    max_encoded_bytes: int,
    max_bitmap_bytes: int,
) -> dict:
    """Use two fresh workers so prefills cannot share native decoder state."""
    results = []
    for prefill in (0, 0xA5):
        result = worker_process(
            {
                "library": str(library),
                "source": str(source),
                **{key: image[key] for key in DISCOVERY_IMAGE_FIELDS if key not in ("page", "image")},
                "prefill": prefill,
                "max_encoded_bytes": max_encoded_bytes,
                "max_bitmap_bytes": max_bitmap_bytes,
            },
            timeout_seconds,
        )
        if result["status"] != "PASS":
            return result
        results.append(result)
    for key in ("raw_stride_sha256", "visible_bits_sha256"):
        if results[0][key] != results[1][key]:
            return {"status": "FAIL", "reason": "oracle output depends on the buffer prefill"}
    return results[0]


def _not_run(expected: int, reason: str) -> dict:
    return {
        "status": "NOT_RUN",
        "expected": expected,
        "passed": 0,
        "failed": 0,
        "not_run": expected,
        "reason": reason,
        "failures": [],
    }


def _overall_status(report: dict) -> str:
    statuses = (report[key]["status"] for key in ("inventory", "discovery", "decode"))
    states = set(statuses)
    if "FAIL" in states:
        return "FAIL"
    return "PASS" if states == {"PASS"} else "NOT_RUN"


def run(
    matrix_path: Path = DEFAULT_MATRIX,
    corpus_dir: Path | None = None,
    manifest_path: Path = DEFAULT_MANIFEST,
    oracle_lib: Path | None = None,
    timeout_seconds: float | None = None,
    max_encoded_bytes: int = DEFAULT_MAX_ENCODED_BYTES,
    max_bitmap_bytes: int = DEFAULT_MAX_BITMAP_BYTES,
) -> tuple[dict, dict | None]:
    """Audit every matrix source before HN/C8 discovery and optional decoding."""
    if max_encoded_bytes <= DIB_AND_PALETTE_BYTES or max_bitmap_bytes <= 0:
        raise OracleError("encoded and decoded byte limits must be positive")
    matrix_samples = conformance.load_matrix(matrix_path)
    selected = [
        row for row in matrix_samples if row["detected_type"] in ("HN", "C8")
    ]
    if not selected:
        raise OracleError("corpus matrix contains no HN/C8 inputs")
    manifest = load_manifest(manifest_path)
    timeout = (
        timeout_seconds if timeout_seconds is not None else manifest["oracle"]["timeout_seconds"]
    )
    if not 0 < timeout <= 300:
        raise OracleError("oracle timeout must be in (0, 300] seconds")
    matrix_sha = sha256_file(matrix_path)
    if matrix_sha != manifest["corpus_matrix_sha256"]:
        raise OracleError("corpus matrix SHA-256 differs from the committed JBIG1 manifest")
    committed_profile = (
        matrix_path.resolve() == DEFAULT_MATRIX.resolve()
        and manifest_path.resolve() == DEFAULT_MANIFEST.resolve()
    )
    expected_images = validate_manifest_inventory(
        manifest, selected, BASELINE_TYPE0_IMAGES if committed_profile else None
    )
    baseline_images = BASELINE_TYPE0_IMAGES if committed_profile else expected_images
    report = {
        "schema_version": 1,
        "status": "NOT_RUN",
        "corpus_matrix_sha256": matrix_sha,
        "manifest": str(manifest_path),
        "baseline_type0_observation": baseline_images,
        "inventory": _not_run(len(matrix_samples), "CAJ2PDF_CORPUS_DIR is unset"),
        "discovery": _not_run(expected_images, "corpus inventory has not passed"),
        "decode": _not_run(expected_images, "corpus discovery has not run"),
    }
    if corpus_dir is None:
        if oracle_lib is not None:
            report["decode"].update(
                status="FAIL", reason="requested external oracle library requires a corpus directory"
            )
            report["status"] = _overall_status(report)
        return report, None
    report["inventory"] = conformance.audit_inventory(matrix_samples, corpus_dir)
    if report["inventory"]["status"] != "PASS":
        report["discovery"]["reason"] = "full corpus inventory did not pass"
        report["decode"]["reason"] = "full corpus inventory did not pass"
        report["status"] = _overall_status(report)
        return report, None

    root = corpus_dir.resolve(strict=True)
    samples: list[dict] = []
    discovery_failures: list[dict] = []
    type_counts: dict[int, int] = {}
    for row in selected:
        source_path = conformance.contained_file(root, conformance.relative_path(row["path"]))
        sample, failures, types = discover_sample(
            row, source_path, max_encoded_bytes, max_bitmap_bytes
        )
        samples.append(sample)
        discovery_failures.extend(failures)
        for image_type, count in types.items():
            type_counts[image_type] = type_counts.get(image_type, 0) + count
    observed = {
        "schema_version": 1,
        "corpus_matrix_sha256": matrix_sha,
        "oracle": oracle_metadata(timeout),
        "samples": samples,
        "discovery_failures": discovery_failures,
    }
    found_images = sum(len(sample["images"]) for sample in samples)
    problems = compare_discovery(observed, manifest)
    report["discovery"] = {
        "status": "FAIL" if problems else "PASS",
        "samples": len(samples),
        "found_type0_images": found_images,
        "expected_type0_images": expected_images,
        "baseline_difference": found_images - baseline_images,
        "image_type_counts": {str(key): count for key, count in sorted(type_counts.items())},
        "expected_invalid_records": len(manifest["discovery_failures"]),
        "invalid_records": discovery_failures,
        "failed": len(problems),
        "failures": problems,
        "comparison_problems": problems,
    }
    report["decode"] = _not_run(
        found_images,
        "no type-0 images were discovered" if found_images == 0 else "external oracle library is unset",
    )
    if oracle_lib is None:
        report["status"] = _overall_status(report)
        return report, observed
    if not oracle_lib.is_file():
        report["decode"].update(
            status="FAIL", reason=f"requested external oracle library is missing: {oracle_lib}"
        )
        report["status"] = _overall_status(report)
        return report, observed
    resolved_library = oracle_lib.resolve(strict=True)
    actual_library_hash = sha256_file(resolved_library)
    if actual_library_hash != PINNED_LIBRARY_SHA256:
        report["decode"].update(
            status="FAIL",
            reason="external oracle library SHA-256 differs from the pinned binary",
        )
        report["status"] = _overall_status(report)
        return report, observed
    if problems:
        report["decode"]["reason"] = "discovery differs from the committed manifest"
        report["status"] = _overall_status(report)
        return report, observed
    if found_images == 0:
        report["status"] = _overall_status(report)
        return report, observed

    expected_samples = {sample["id"]: sample for sample in manifest["samples"]}
    decoded_passed = 0
    decoded_failed = 0
    decoded_not_run = 0
    decode_failures: list[dict] = []
    for sample in samples:
        source_path = conformance.contained_file(root, conformance.relative_path(sample["path"]))
        expected_by_location = {
            (image["page"], image["image"]): image
            for image in expected_samples.get(sample["id"], {}).get("images", [])
        }
        for image in sample["images"]:
            location = (image["page"], image["image"])
            result = decode_image(
                resolved_library,
                source_path,
                image,
                timeout,
                max_encoded_bytes,
                max_bitmap_bytes,
            )
            if result["status"] != "PASS":
                image["decoder_result"] = "FAIL"
                decoded_failed += 1
                decode_failures.append(
                    {
                        "sample_id": sample["id"],
                        "page": location[0],
                        "image": location[1],
                        "reason": result["reason"],
                    }
                )
                continue
            image["decoder_result"] = "PASS"
            image["raw_stride_sha256"] = result["raw_stride_sha256"]
            image["visible_bits_sha256"] = result["visible_bits_sha256"]
            expected_image = expected_by_location.get(location)
            if expected_image is None or expected_image["decoder_result"] != "PASS":
                decoded_not_run += 1
                continue
            if any(
                image[key] != expected_image[key]
                for key in ("raw_stride_sha256", "visible_bits_sha256")
            ):
                decoded_failed += 1
                decode_failures.append(
                    {
                        "sample_id": sample["id"],
                        "page": location[0],
                        "image": location[1],
                        "reason": "decoded pixel SHA-256 differs from the committed manifest",
                    }
                )
            else:
                decoded_passed += 1
    report["decode"] = {
        "status": (
            "FAIL" if decoded_failed else "NOT_RUN" if decoded_not_run else "PASS"
        ),
        "expected": found_images,
        "passed": decoded_passed,
        "failed": decoded_failed,
        "not_run": decoded_not_run,
        "failures": decode_failures,
        "timeout_seconds_per_prefill": timeout,
        "prefills": PREFILLS,
    }
    report["status"] = _overall_status(report)
    return report, observed


def print_text(report: dict) -> None:
    inventory = report["inventory"]
    print(
        f"Corpus inventory [{inventory['status']}]: "
        f"PASS={inventory['passed']} FAIL={inventory['failed']} "
        f"NOT_RUN={inventory['not_run']}"
    )
    discovery = report["discovery"]
    print(
        f"HN/C8 type-0 discovery [{discovery['status']}]: "
        f"FOUND={discovery.get('found_type0_images', 0)} "
        f"EXPECTED_INVALID={discovery.get('expected_invalid_records', 0)} "
        f"OBSERVED_INVALID={len(discovery.get('invalid_records', []))} "
        f"FAIL={discovery.get('failed', 0)}"
    )
    decode = report["decode"]
    print(
        f"JBIG1 pixel oracle [{decode['status']}]: "
        f"PASS={decode['passed']} FAIL={decode['failed']} NOT_RUN={decode['not_run']}"
    )
    for phase in (inventory, discovery, decode):
        if phase.get("reason"):
            print(f"  {phase['reason']}")
    print(f"Overall: {report['status']}")


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args[:1] == ["--worker"]:
        if len(args) != 2:
            print(json.dumps({"status": "FAIL", "reason": "worker expects one JSON payload"}))
            return 1
        return worker_main(args[1])
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    corpus_env = os.environ.get("CAJ2PDF_CORPUS_DIR")
    library_env = os.environ.get("CAJ2PDF_JBIG1_ORACLE_LIB")
    parser.add_argument("--corpus-dir", type=Path, default=Path(corpus_env) if corpus_env else None)
    parser.add_argument("--oracle-lib", type=Path, default=Path(library_env) if library_env else None)
    parser.add_argument("--timeout-seconds", type=float)
    parser.add_argument("--max-encoded-bytes", type=int, default=DEFAULT_MAX_ENCODED_BYTES)
    parser.add_argument("--max-bitmap-bytes", type=int, default=DEFAULT_MAX_BITMAP_BYTES)
    parser.add_argument("--json", action="store_true", help="print machine-readable phase statuses")
    options = parser.parse_args(args)
    try:
        report, _observed = run(
            options.matrix,
            options.corpus_dir,
            options.manifest,
            options.oracle_lib,
            options.timeout_seconds,
            options.max_encoded_bytes,
            options.max_bitmap_bytes,
        )
    except (OracleError, conformance.ConformanceError, OSError) as exc:
        report = {"status": "FAIL", "reason": str(exc)}
        print(json.dumps(report, ensure_ascii=False) if options.json else f"JBIG1 oracle [FAIL]: {exc}")
        return 1
    if options.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print_text(report)
    return int(report["status"] == "FAIL")


if __name__ == "__main__":
    raise SystemExit(main())
