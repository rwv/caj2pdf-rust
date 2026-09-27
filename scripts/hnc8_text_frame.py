#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded diagnostic for the observed HN-A/C8 page-text frame.

This is an original parser for a measured corpus profile, not a published
HN/C8 format specification. It assigns no meaning to the inflated fields.
The caller supplies a checked page-text range and declared image count.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path
import struct
from typing import BinaryIO, Callable
import tempfile
import zlib


HEADER_BYTES = 24
PREFIX_BYTES = 20
IO_CHUNK = 64 * 1024
MAX_SOURCE_BYTES = 1024 * 1024 * 1024


class FrameError(ValueError):
    """The declared range violates the measured frame profile or a limit."""


@dataclass(frozen=True)
class FrameLimits:
    max_span_bytes: int = 1024 * 1024
    max_decoded_bytes: int = 1024 * 1024
    max_records: int = 65_536
    chunk_bytes: int = IO_CHUNK

    def validate(self) -> None:
        values = (self.max_span_bytes, self.max_decoded_bytes,
                  self.max_records, self.chunk_bytes)
        if any(type(value) is not int for value in values):
            raise FrameError("all frame limits must be integers, excluding bool")
        if min(values) <= 0:
            raise FrameError("all frame limits must be positive")
        if self.chunk_bytes > IO_CHUNK:
            raise FrameError("read/output chunk exceeds the 64 KiB diagnostic cap")


@dataclass(frozen=True)
class FrameProfile:
    name: str
    prefix_sha256: str | None = None
    marker_sha256: str | None = None


# Hashes identify the variant-specific 20-byte prefixes and invariant
# three-slot marker fingerprint measured in the 75-page #111 corpus. Their
# original bytes are intentionally not included.
MARKER_SHA256 = "fb97f7415096b42c16397110410235e00ec0e24c95c2e308babd880a7ef2c021"
PROFILES = {
    "hn_a": FrameProfile(
        "hn_a", "1fc8a3a838c4f3a7eeaf7252ccdfe8455f2184713ce9dc383fa91b96934fbb12",
        MARKER_SHA256),
    "c8": FrameProfile(
        "c8", "078c830d36708db41f5bdf3779e422616869092e879c34752ddc91968dd36667",
        MARKER_SHA256),
}


@dataclass(frozen=True)
class RecordLocations:
    first_offset: int
    stride: int
    count: int


@dataclass(frozen=True)
class FrameMetadata:
    variant: str
    source_offset: int
    source_length: int
    declared_image_count: int
    prefix_sha256: str
    zlib_stream_sha256: str
    decoded_sha256: str
    decoded_length: int
    record_count: int
    marker_triplet_sha256: str | None
    opaque_header: RecordLocations
    item_records: RecordLocations
    item_marker_offsets: tuple[int, int, int]
    opaque_separator: RecordLocations
    trailing_records: RecordLocations
    max_source_read_request_bytes: int
    max_decoder_output_chunk_bytes: int
    decoded_spool_bytes: int


def _read_exact(source: BinaryIO, length: int, chunk_bytes: int = IO_CHUNK) -> bytes:
    """Read at most one bounded chunk, accepting short but progressing reads."""
    result = bytearray()
    while len(result) < length:
        remaining = min(length - len(result), chunk_bytes)
        part = source.read(remaining)
        if not isinstance(part, bytes):
            raise FrameError("source read must return bytes")
        if not part:
            raise FrameError("text span is truncated")
        if len(part) > remaining:
            raise FrameError("source returned more bytes than requested")
        result.extend(part)
    return bytes(result)


def _check_range(source: BinaryIO, offset: int, length: int,
                 limits: FrameLimits) -> None:
    if offset < 0 or length < HEADER_BYTES or length > limits.max_span_bytes:
        raise FrameError("text range is negative, too short, or exceeds limit")
    try:
        source.seek(0, 2)
        source_size = source.tell()
    except (OSError, AttributeError) as exc:
        raise FrameError("a seekable source is required") from exc
    if offset > source_size or length > source_size - offset:
        raise FrameError("text range exceeds source size")


def _layout(decoded_length: int, image_count: int,
            limits: FrameLimits) -> tuple[int, int, int]:
    if image_count < 0 or image_count > limits.max_records:
        raise FrameError("declared image count exceeds limit")
    base = 8 + 4 + 28 * image_count
    item_bytes = decoded_length - base
    if item_bytes < 0 or item_bytes % 16:
        raise FrameError("decoded length does not match the 8+16*N+4+28*image_count profile")
    record_count = item_bytes // 16
    if record_count > limits.max_records:
        raise FrameError("item record count exceeds limit")
    separator_at = 8 + 16 * record_count
    trailing_at = separator_at + 4
    return record_count, separator_at, trailing_at


def _inflate(source: BinaryIO, compressed_length: int, declared_length: int,
             spool: BinaryIO, limits: FrameLimits) -> tuple[str, str, int, int]:
    inflater = zlib.decompressobj(wbits=zlib.MAX_WBITS)
    compressed_hash = hashlib.sha256()
    decoded_hash = hashlib.sha256()
    total = 0
    max_output = 0
    remaining = compressed_length
    while remaining:
        block = _read_exact(source, min(remaining, limits.chunk_bytes), limits.chunk_bytes)
        remaining -= len(block)
        compressed_hash.update(block)
        pending = block
        while pending:
            before = len(pending)
            try:
                chunk = inflater.decompress(pending, limits.chunk_bytes)
            except zlib.error as exc:
                raise FrameError("invalid zlib stream or checksum") from exc
            pending = inflater.unconsumed_tail
            max_output = max(max_output, len(chunk))
            total += len(chunk)
            if total > declared_length or total > limits.max_decoded_bytes:
                raise FrameError("decoded output exceeds declared length or limit")
            spool.write(chunk)
            decoded_hash.update(chunk)
            if inflater.eof:
                if inflater.unused_data or pending or remaining:
                    raise FrameError("zlib stream has trailing bytes")
                break
            if len(pending) == before and not chunk:
                raise FrameError("zlib decoder made no progress")
    if not inflater.eof:
        raise FrameError("zlib stream did not reach EOF")
    if total != declared_length:
        raise FrameError("declared decoded length differs from actual output")
    return compressed_hash.hexdigest(), decoded_hash.hexdigest(), total, max_output


def _check_markers(spool: BinaryIO, record_count: int,
                   profile: FrameProfile) -> str | None:
    spool.seek(8)
    first: bytes | None = None
    for index in range(record_count):
        record = _read_exact(spool, 16)
        triplet = record[0:2] + record[4:6] + record[8:10]
        if first is None:
            first = triplet
            if (profile.marker_sha256 is not None and
                    hashlib.sha256(first).hexdigest() != profile.marker_sha256):
                raise FrameError("marker triplet differs from pinned profile")
        elif triplet != first:
            raise FrameError(f"marker triplet differs at item record {index}")
    return hashlib.sha256(first).hexdigest() if first is not None else None


def inspect_frame(source: BinaryIO, *, offset: int, length: int,
                  image_count: int, profile: FrameProfile,
                  limits: FrameLimits = FrameLimits(),
                  decoded_callback: Callable[[BinaryIO, FrameMetadata], None] | None = None,
                  ) -> FrameMetadata:
    """Validate one checked page-text span using bounded reads and disk spool.

    The zlib stream must start at text-relative +24 and end at span end.
    Returned offsets are relative to the *decoded* buffer except
    ``source_offset``. No decoded document bytes are returned. An optional
    callback receives the validated private spool at position zero; it is
    closed as soon as the callback returns. Metadata describes its original
    contents. Callers must keep any copied document bytes outside Git.
    The caller is responsible for pinning and rechecking source identity.
    """
    limits.validate()
    _check_range(source, offset, length, limits)
    source.seek(offset)
    header = _read_exact(source, HEADER_BYTES, limits.chunk_bytes)
    prefix_hash = hashlib.sha256(header[:PREFIX_BYTES]).hexdigest()
    if profile.prefix_sha256 is not None and prefix_hash != profile.prefix_sha256:
        raise FrameError("20-byte frame prefix differs from pinned profile")
    declared_length = struct.unpack_from("<I", header, PREFIX_BYTES)[0]
    if declared_length > limits.max_decoded_bytes:
        raise FrameError("declared decoded length exceeds limit")
    record_count, separator_at, trailing_at = _layout(
        declared_length, image_count, limits)
    with tempfile.TemporaryFile(mode="w+b") as spool:
        compressed_hash, decoded_hash, actual_length, max_output = _inflate(
            source, length - HEADER_BYTES, declared_length, spool, limits)
        marker_hash = _check_markers(spool, record_count, profile)
        result = FrameMetadata(
            variant=profile.name, source_offset=offset, source_length=length,
            declared_image_count=image_count, prefix_sha256=prefix_hash,
            zlib_stream_sha256=compressed_hash, decoded_sha256=decoded_hash,
            decoded_length=actual_length, record_count=record_count,
            marker_triplet_sha256=marker_hash,
            opaque_header=RecordLocations(0, 8, 1),
            item_records=RecordLocations(8, 16, record_count),
            item_marker_offsets=(0, 4, 8),
            opaque_separator=RecordLocations(separator_at, 4, 1),
            trailing_records=RecordLocations(trailing_at, 28, image_count),
            max_source_read_request_bytes=max(
                min(HEADER_BYTES, limits.chunk_bytes),
                min(length - HEADER_BYTES, limits.chunk_bytes)),
            max_decoder_output_chunk_bytes=max_output,
            decoded_spool_bytes=actual_length,
        )
        if decoded_callback is not None:
            spool.seek(0)
            decoded_callback(spool, result)
        return result


def _file_sha256(path: Path) -> str:
    """Hash a bounded full input with at most 64 KiB per read request."""
    size = path.stat().st_size
    if size > MAX_SOURCE_BYTES:
        raise FrameError("source file exceeds the 1 GiB diagnostic limit")
    digest = hashlib.sha256()
    total = 0
    with path.open("rb") as source:
        while block := source.read(IO_CHUNK):
            total += len(block)
            if total > MAX_SOURCE_BYTES:
                raise FrameError("source file grew beyond the 1 GiB diagnostic limit")
            digest.update(block)
    if total != size or path.stat().st_size != total:
        raise FrameError("source file size changed during hashing")
    return digest.hexdigest()


def _sha256_argument(value: str) -> str:
    if len(value) != 64 or any(character not in "0123456789abcdefABCDEF" for character in value):
        raise argparse.ArgumentTypeError("SHA-256 must contain exactly 64 hexadecimal digits")
    return value.lower()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, help="external source file")
    parser.add_argument("--input-sha256", type=_sha256_argument,
                        help="required full-file SHA-256, checked before and after parsing")
    parser.add_argument("--variant", choices=sorted(PROFILES))
    parser.add_argument("--offset", type=int, help="absolute page-text offset")
    parser.add_argument("--length", type=int, help="page-text span length")
    parser.add_argument("--image-count", type=int, help="declared image count")
    parser.add_argument("--json", action="store_true", help="emit compact JSON instead of indented JSON")
    args = parser.parse_args()
    values = (args.input, args.input_sha256, args.variant,
              args.offset, args.length, args.image_count)
    if all(value is None for value in values):
        print(json.dumps({"status": "NOT_RUN", "private_comparisons": 0},
                         sort_keys=True, indent=None if args.json else 2))
        return 0
    if any(value is None for value in values):
        parser.error("--input, --input-sha256, --variant, --offset, --length and --image-count are required together")
    before_hash = after_hash = None
    result = None
    parse_attempted = False
    errors = []
    try:
        before_hash = _file_sha256(args.input)
        if before_hash != args.input_sha256:
            raise FrameError("full-source SHA-256 differs before parsing")
        parse_attempted = True
        with args.input.open("rb") as source:
            result = inspect_frame(source, offset=args.offset, length=args.length,
                                   image_count=args.image_count, profile=PROFILES[args.variant])
    except (OSError, FrameError) as exc:
        errors.append(str(exc))
    try:
        after_hash = _file_sha256(args.input)
        if after_hash != args.input_sha256:
            raise FrameError("full-source SHA-256 differs after parsing")
    except (OSError, FrameError) as exc:
        errors.append(str(exc))
    identity_valid = before_hash == after_hash == args.input_sha256
    structural_status = ("PASS" if identity_valid else "INVALIDATED") if result is not None else (
        "FAIL" if parse_attempted else "NOT_RUN")
    report = {
        "status": "FAIL" if errors else "VALIDATED",
        "scope": "FRAME_PROFILE_ONLY",
        "structural_validation": structural_status,
        "converter_compatibility": "NOT_RUN",
        "private_comparisons": 0,
        "source_audit": {
            "status": "PASS" if identity_valid else "FAIL",
            "expected_sha256": args.input_sha256,
            "before_sha256": before_hash, "after_sha256": after_hash,
            "max_hash_request_bytes": IO_CHUNK,
            "max_source_bytes": MAX_SOURCE_BYTES,
        },
        "errors": errors,
    }
    if result is not None and not errors:
        report["frame"] = asdict(result)
    print(json.dumps(report,
                     sort_keys=True, indent=None if args.json else 2))
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
