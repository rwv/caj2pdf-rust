#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded, metadata-only HN/C8 source-page measurements for issue #107.

The byte fields come from the repository's independent #22/#61 measurements
in docs/research/hnc8-container.md. This is a small, original measurement tool: it
does not decode images or text, infer placement, or read another converter.
Callers must verify the requested source's pinned SHA-256 before and after
using this module. No source bytes are included in returned records.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import struct
from typing import Callable, Iterator, Protocol


class RangedInput(Protocol):
    size: int

    def read_at(self, offset: int, count: int) -> bytes: ...


class FileInput:
    """Read-only positioned file adapter; owns its descriptor until closed."""

    def __init__(self, path: Path | str):
        self._fd = os.open(path, os.O_RDONLY)
        self.size = os.fstat(self._fd).st_size

    def read_at(self, offset: int, count: int) -> bytes:
        return os.pread(self._fd, count, offset)

    def close(self) -> None:
        if self._fd >= 0:
            os.close(self._fd)
            self._fd = -1

    def __enter__(self) -> FileInput:
        return self

    def __exit__(self, _kind: object, _error: object, _traceback: object) -> None:
        self.close()


@dataclass(frozen=True)
class SourceLimits:
    max_source_bytes: int = 1024 * 1024 * 1024
    max_pages: int = 10_000
    max_outline_records: int = 100_000
    max_images_per_page: int = 8_192
    max_images_total: int = 1_000_000
    max_text_span_bytes: int = 64 * 1024 * 1024
    max_image_span_bytes: int = 64 * 1024 * 1024
    max_image_pixels: int = 100_000_000
    max_dimension: int = 100_000
    max_jpeg_header_bytes: int = 1024 * 1024
    max_jpeg_markers: int = 4_096
    max_reader_bytes: int = 2 * 1024 * 1024 * 1024
    max_read_calls: int = 2_000_000
    io_chunk_bytes: int = 64 * 1024

    def validate(self) -> None:
        for name, value in vars(self).items():
            if not isinstance(value, int) or value <= 0:
                raise ValueError(f"{name} must be a positive integer")


class SourceMetadataError(ValueError):
    def __init__(
        self,
        source_id: str,
        offset: int,
        field: str,
        kind: str,
        detail: str,
        page: int | None = None,
        image: int | None = None,
    ):
        self.source_id = source_id
        self.offset = offset
        self.page = page
        self.image = image
        self.field = field
        self.kind = kind
        self.detail = detail
        location = f"{source_id}: byte {offset}"
        if page is not None:
            location += f", page {page}"
        if image is not None:
            location += f", image {image}"
        super().__init__(f"{location}: {kind} {field}: {detail}")


class SourceExtractor:
    """Read one requested page at a time, retaining no document-sized data."""

    def __init__(
        self,
        source: RangedInput,
        source_id: str,
        limits: SourceLimits = SourceLimits(),
        cancelled: Callable[[], bool] = lambda: False,
    ):
        limits.validate()
        self.source = source
        self.source_id = source_id
        self.limits = limits
        self.cancelled = cancelled
        self.reader_bytes = 0
        self.max_request_bytes = 0
        self.read_calls = 0
        if not isinstance(source.size, int) or source.size < 0:
            self._fail(0, "source size", "malformed", "negative or noninteger size")
        if source.size > limits.max_source_bytes:
            self._fail(0, "source bytes", "limit", f"{source.size} > {limits.max_source_bytes}")
        self.header = self._header()

    def _fail(
        self,
        offset: int,
        field: str,
        kind: str,
        detail: str,
        page: int | None = None,
        image: int | None = None,
    ) -> None:
        raise SourceMetadataError(self.source_id, offset, field, kind, detail, page, image)

    def _cancel(self, offset: int, page: int | None = None, image: int | None = None) -> None:
        if self.cancelled():
            self._fail(offset, "cancellation", "cancelled", "requested", page, image)

    def _span(
        self,
        offset: int,
        length: int,
        field: str,
        error_offset: int,
        page: int | None = None,
        image: int | None = None,
    ) -> int:
        if offset < 0 or length < 0:
            self._fail(error_offset, field, "malformed", "negative offset or length", page, image)
        end = offset + length
        if end > self.source.size:
            self._fail(error_offset, field, "truncated", f"end {end} > size {self.source.size}", page, image)
        return end

    def _read(
        self,
        offset: int,
        length: int,
        field: str,
        page: int | None = None,
        image: int | None = None,
    ) -> bytes:
        self._span(offset, length, field, offset, page, image)
        if self.reader_bytes + length > self.limits.max_reader_bytes:
            self._fail(offset, "reader bytes", "limit", f"needs {length} more bytes", page, image)
        output = bytearray(length)
        done = 0
        while done < length:
            at = offset + done
            self._cancel(at, page, image)
            if self.read_calls >= self.limits.max_read_calls:
                self._fail(at, "read calls", "limit", "call ceiling reached", page, image)
            count = min(length - done, self.limits.io_chunk_bytes)
            self.max_request_bytes = max(self.max_request_bytes, count)
            self.read_calls += 1
            try:
                chunk = self.source.read_at(at, count)
            except OSError as error:
                self._fail(at, field, "source", str(error), page, image)
            if not isinstance(chunk, bytes):
                self._fail(at, field, "source", "read_at returned nonbytes", page, image)
            if len(chunk) > count:
                self._fail(at, field, "source", "read_at overreported", page, image)
            if not chunk:
                self._fail(at, field, "source", "read_at made zero progress", page, image)
            output[done:done + len(chunk)] = chunk
            done += len(chunk)
            self.reader_bytes += len(chunk)
        return bytes(output)

    def _hash(
        self,
        offset: int,
        length: int,
        field: str,
        page: int | None = None,
        image: int | None = None,
    ) -> str:
        self._span(offset, length, field, offset, page, image)
        digest = hashlib.sha256()
        done = 0
        while done < length:
            size = min(length - done, self.limits.io_chunk_bytes)
            digest.update(self._read(offset + done, size, field, page, image))
            done += size
        return digest.hexdigest()

    def _header(self) -> dict:
        self._cancel(0)
        signature = self._read(0, 4, "signature")
        if signature == b"\xc8\x00\x00\x00":
            variant, count_at, index_at = "C8", 8, 0x50
        elif signature == b"HN\x00\x00":
            marker = self._read(4, 4, "HN marker")
            if marker == b"\x90\x01\x00\x00":
                variant, count_at, index_at = "HN-A", 0x90, 0x15C
            elif marker == b"\xc8\x00\x00\x00":
                variant, count_at, index_at = "HN-B", 0x90, 0xD8
            else:
                self._fail(4, "HN marker", "unsupported", marker.hex())
        else:
            self._fail(0, "signature", "unsupported", signature.hex())
        count = struct.unpack("<i", self._read(count_at, 4, "page count"))[0]
        if count <= 0:
            self._fail(count_at, "page count", "malformed", "must be positive")
        if count > self.limits.max_pages:
            self._fail(count_at, "pages", "limit", f"{count} > {self.limits.max_pages}")
        if variant == "HN-A":
            outlines = struct.unpack("<i", self._read(0x158, 4, "outline count"))[0]
            if outlines < 0:
                self._fail(0x158, "outline count", "malformed", "negative signed count")
            if outlines > self.limits.max_outline_records:
                self._fail(0x158, "outline records", "limit", f"{outlines} > {self.limits.max_outline_records}")
            index_at += outlines * 308
        index_length = count * 20
        self._span(index_at, index_length, "page index", index_at)
        return {
            "source_id": self.source_id,
            "variant": variant,
            "page_count": count,
            "page_index_offset": index_at,
            "page_index_length": index_length,
        }

    def _dib_dimensions(self, offset: int, length: int, page: int, image: int) -> dict:
        if length <= 48:
            self._fail(offset, "DIB wrapper", "truncated", "needs wrapper and coded bytes", page, image)
        dib = self._read(offset, 48, "DIB wrapper", page, image)
        size, width, height, planes, depth, compression = struct.unpack_from("<IiiHHI", dib)
        if size != 40:
            self._fail(offset, "DIB size", "unsupported", str(size), page, image)
        if width <= 0 or height <= 0:
            self._fail(offset + 4, "DIB dimensions", "malformed", f"{width} x {height}", page, image)
        if width > self.limits.max_dimension or height > self.limits.max_dimension:
            self._fail(offset + 4, "image dimension", "limit", f"{width} x {height}", page, image)
        if width * height > self.limits.max_image_pixels:
            self._fail(offset + 4, "image pixels", "limit", f"{width * height} > {self.limits.max_image_pixels}", page, image)
        if (planes, depth, compression) != (1, 1, 0):
            self._fail(offset + 12, "DIB profile", "unsupported", f"{planes}/{depth}/{compression}", page, image)
        if dib[40:48] != b"\xff\xff\xff\x00\x00\x00\x00\x00":
            self._fail(offset + 40, "DIB palette", "unsupported", "outside measured two-color profile", page, image)
        stride = ((width + 31) // 32) * 4
        return {"width": width, "height": height, "dimension_source": "dib", "dib_stride": stride, "stride_width": stride * 8}

    def _jpeg_read(self, offset: int, count: int, end: int, field: str, page: int, image: int) -> bytes:
        if offset + count > end:
            self._fail(offset, field, "truncated", "past image payload", page, image)
        return self._read(offset, count, field, page, image)

    def _jpeg_dimensions(self, offset: int, length: int, page: int, image: int) -> dict:
        end = offset + length
        if length < 4 or self._jpeg_read(offset, 2, end, "JPEG SOI", page, image) != b"\xff\xd8":
            self._fail(offset, "JPEG SOI", "malformed", "missing start marker", page, image)
        at = offset + 2
        markers = 0
        while at < end:
            self._cancel(at, page, image)
            if at - offset >= self.limits.max_jpeg_header_bytes:
                self._fail(at, "JPEG header bytes", "limit", "scan ceiling reached", page, image)
            if markers >= self.limits.max_jpeg_markers:
                self._fail(at, "JPEG markers", "limit", "marker ceiling reached", page, image)
            marker_start = at
            if self._jpeg_read(at, 1, end, "JPEG marker", page, image) != b"\xff":
                self._fail(at, "JPEG marker", "malformed", "missing prefix", page, image)
            at += 1
            marker = 0xFF
            while marker == 0xFF:
                if at - offset >= self.limits.max_jpeg_header_bytes:
                    self._fail(at, "JPEG header bytes", "limit", "scan ceiling reached", page, image)
                marker = self._jpeg_read(at, 1, end, "JPEG marker", page, image)[0]
                at += 1
            markers += 1
            if marker in (0x00, 0x01, 0xD8, 0xD9, 0xDA) or 0xD0 <= marker <= 0xD7:
                self._fail(marker_start, "JPEG SOF0", "unsupported", f"marker {marker:02x} before SOF0", page, image)
            segment_length = int.from_bytes(self._jpeg_read(at, 2, end, "JPEG segment length", page, image), "big")
            if segment_length < 2:
                self._fail(at, "JPEG segment length", "malformed", "below two", page, image)
            next_at = at + segment_length
            if next_at > end:
                self._fail(at, "JPEG segment", "truncated", "past image payload", page, image)
            if next_at - offset > self.limits.max_jpeg_header_bytes:
                self._fail(at, "JPEG header bytes", "limit", "scan ceiling reached", page, image)
            if marker == 0xC0:
                if segment_length < 8:
                    self._fail(at, "JPEG SOF0", "malformed", "short frame", page, image)
                frame = self._jpeg_read(at + 2, 6, end, "JPEG SOF0", page, image)
                precision = frame[0]
                height = int.from_bytes(frame[1:3], "big")
                width = int.from_bytes(frame[3:5], "big")
                components = frame[5]
                if not width or not height or not components:
                    self._fail(at + 2, "JPEG dimensions", "malformed", "zero width, height, or components", page, image)
                if width > self.limits.max_dimension or height > self.limits.max_dimension or width * height > self.limits.max_image_pixels:
                    self._fail(at + 2, "image dimensions", "limit", f"{width} x {height}", page, image)
                if segment_length != 8 + 3 * components:
                    self._fail(at, "JPEG SOF0", "malformed", "component records do not fit frame", page, image)
                return {"width": width, "height": height, "dimension_source": "jpeg_sof0", "precision": precision, "components": components}
            at = next_at
        self._fail(end, "JPEG SOF0", "truncated", "not found within payload", page, image)

    def read_page(self, page_number: int, *, _remaining_images: int | None = None) -> dict:
        count = self.header["page_count"]
        if not isinstance(page_number, int) or page_number < 1 or page_number > count:
            self._fail(self.header["page_index_offset"], "page number", "malformed", f"outside 1..{count}")
        row_at = self.header["page_index_offset"] + (page_number - 1) * 20
        self._cancel(row_at, page_number)
        row = self._read(row_at, 20, "page row", page_number)
        text_offset, text_length, image_count = struct.unpack_from("<iih", row)
        if text_offset < 0:
            self._fail(row_at, "text offset", "malformed", "negative signed value", page_number)
        if text_length < 0:
            self._fail(row_at + 4, "text length", "malformed", "negative signed value", page_number)
        text_end = self._span(text_offset, text_length, "text span", row_at, page_number)
        if text_length > self.limits.max_text_span_bytes:
            self._fail(row_at + 4, "text span bytes", "limit", f"{text_length} > {self.limits.max_text_span_bytes}", page_number)
        if image_count < 0:
            self._fail(row_at + 8, "image count", "malformed", "negative signed value", page_number)
        if image_count > self.limits.max_images_per_page or image_count > self.limits.max_images_total:
            self._fail(row_at + 8, "image count", "limit", f"{image_count} exceeds configured ceiling", page_number)
        if _remaining_images is not None and image_count > _remaining_images:
            self._fail(row_at + 8, "images total", "limit", f"{image_count} exceeds remaining {_remaining_images}", page_number)
        text_hash = self._hash(text_offset, text_length, "text span", page_number) if text_length else None
        page = {
            "source_id": self.source_id,
            "page_number": page_number,
            "row_offset": row_at,
            "text_offset": text_offset,
            "text_length": text_length,
            "text_sha256": text_hash,
            "image_count": image_count,
            "raw_10": int.from_bytes(row[10:12], "little"),
            "raw_12": int.from_bytes(row[12:16], "little"),
            "raw_16": int.from_bytes(row[16:20], "little"),
            "images": [],
        }
        descriptor_at = text_end
        protected_end = self.header["page_index_offset"] + self.header["page_index_length"]
        for image_number in range(1, image_count + 1):
            self._cancel(descriptor_at, page_number, image_number)
            if descriptor_at < protected_end:
                self._fail(descriptor_at, "image descriptor", "malformed", "overlaps header or page index", page_number, image_number)
            raw = self._read(descriptor_at, 12, "image descriptor", page_number, image_number)
            record_type, payload_offset, payload_length = struct.unpack("<iii", raw)
            if record_type < 0:
                self._fail(descriptor_at, "image type", "malformed", "negative signed value", page_number, image_number)
            if record_type > 3:
                self._fail(descriptor_at, "image type", "unsupported", str(record_type), page_number, image_number)
            if payload_offset < descriptor_at + 12:
                self._fail(descriptor_at + 4, "image offset", "malformed", "overlaps descriptor or regresses", page_number, image_number)
            if payload_length <= 0:
                self._fail(descriptor_at + 8, "image length", "malformed", "must be positive", page_number, image_number)
            payload_end = self._span(payload_offset, payload_length, "image payload", descriptor_at + 4, page_number, image_number)
            if payload_length > self.limits.max_image_span_bytes:
                self._fail(descriptor_at + 8, "image span bytes", "limit", f"{payload_length} > {self.limits.max_image_span_bytes}", page_number, image_number)
            if record_type in (0, 3):
                dims = self._dib_dimensions(payload_offset, payload_length, page_number, image_number)
            elif record_type == 2:
                dims = self._jpeg_dimensions(payload_offset, payload_length, page_number, image_number)
            else:
                dims = {"width": None, "height": None, "dimension_source": "unmeasured_type_1"}
            page["images"].append({
                "image_number": image_number,
                "record_type": record_type,
                "descriptor_offset": descriptor_at,
                "gap_start": descriptor_at + 12,
                "gap_length": payload_offset - (descriptor_at + 12),
                "payload_offset": payload_offset,
                "payload_length": payload_length,
                "payload_sha256": self._hash(payload_offset, payload_length, "image payload", page_number, image_number),
                **dims,
            })
            descriptor_at = payload_end
        return page

    def iter_pages(self) -> Iterator[dict]:
        total_images = 0
        for page_number in range(1, self.header["page_count"] + 1):
            page = self.read_page(page_number, _remaining_images=self.limits.max_images_total - total_images)
            total_images += page["image_count"]
            yield page


__all__ = ["FileInput", "RangedInput", "SourceExtractor", "SourceLimits", "SourceMetadataError"]
