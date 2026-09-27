#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original synthetic source-metadata and hostile I/O tests for issue #107."""

from __future__ import annotations

import hashlib
from pathlib import Path
import struct
import sys
import tempfile
import unittest


sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from hnc8_layout_source import (  # noqa: E402
    FileInput,
    SourceExtractor,
    SourceLimits,
    SourceMetadataError,
)


PALETTE = b"\xff\xff\xff\x00\x00\x00\x00\x00"


def dib(width: int, height: int) -> bytes:
    payload = bytearray(49)
    struct.pack_into("<IiiHHI", payload, 0, 40, width, height, 1, 1, 0)
    payload[40:48] = PALETTE
    payload[48] = 0xA5
    return bytes(payload)


def jpeg(width: int, height: int) -> bytes:
    frame = bytes([8]) + height.to_bytes(2, "big") + width.to_bytes(2, "big") + b"\x01\x01\x11\x00"
    return b"\xff\xd8\xff\xe0\x00\x04\x01\x02\xff\xc0\x00\x0b" + frame + b"\xff\xd9"


def document(variant: str, pages: int = 2, size: int = 2048) -> tuple[bytearray, int]:
    output = bytearray(size)
    if variant == "C8":
        output[0:4] = b"\xc8\x00\x00\x00"
        struct.pack_into("<i", output, 8, pages)
        index = 0x50
    elif variant == "HN-A":
        output[0:8] = b"HN\x00\x00\x90\x01\x00\x00"
        struct.pack_into("<i", output, 0x90, pages)
        struct.pack_into("<i", output, 0x158, 1)
        index = 0x15C + 308
    else:
        output[0:8] = b"HN\x00\x00\xc8\x00\x00\x00"
        struct.pack_into("<i", output, 0x90, pages)
        index = 0xD8
    return output, index


def row(output: bytearray, index: int, page: int, text_at: int, text: bytes, images: int, raw: tuple[int, int, int] = (0, 0, 0)) -> None:
    at = index + (page - 1) * 20
    struct.pack_into("<iihHII", output, at, text_at, len(text), images, *raw)
    output[text_at:text_at + len(text)] = text


def image(output: bytearray, descriptor_at: int, payload_at: int, record_type: int, payload: bytes) -> int:
    struct.pack_into("<iii", output, descriptor_at, record_type, payload_at, len(payload))
    output[payload_at:payload_at + len(payload)] = payload
    return payload_at + len(payload)


class MemoryInput:
    def __init__(self, data: bytes, max_return: int | None = None):
        self.data = data
        self.size = len(data)
        self.max_return = max_return
        self.calls = 0
        self.mode = "normal"

    def read_at(self, offset: int, count: int) -> bytes:
        self.calls += 1
        if self.mode == "zero":
            return b""
        if self.mode == "overreport":
            return b"x" * (count + 1)
        if self.mode == "error":
            raise OSError("synthetic read failure")
        if self.mode == "wrong_type":
            return "not bytes"  # type: ignore[return-value]
        if self.max_return is not None:
            count = min(count, self.max_return)
        return self.data[offset:offset + count]


def sample(variant: str = "C8") -> tuple[bytes, int]:
    data, index = document(variant, pages=3)
    text_at = max(index + 60, 800)
    row(data, index, 1, text_at, b"abc", 2, (0x81FF, 0x12345678, 0x9ABCDEF0))
    d = text_at + 3
    d = image(data, d, d + 17, 0, dib(2573, 7))
    image(data, d, d + 12, 2, jpeg(155, 1127))
    row(data, index, 2, text_at, b"abc", 0)
    row(data, index, 3, 1800, b"text", 0)
    return bytes(data), index


class SourceLayoutTests(unittest.TestCase):
    def test_three_variants_hashes_dimensions_order_and_raw_fields(self) -> None:
        for variant in ("C8", "HN-A", "HN-B"):
            with self.subTest(variant=variant):
                data, index = sample(variant)
                source = MemoryInput(data, max_return=3)
                measured = SourceExtractor(source, "synthetic", SourceLimits(io_chunk_bytes=5))
                self.assertEqual(measured.header["variant"], variant)
                self.assertEqual(measured.header["page_index_offset"], index)
                pages = list(measured.iter_pages())
                self.assertEqual([page["image_count"] for page in pages], [2, 0, 0])
                first = pages[0]
                self.assertEqual(first["row_offset"], index)
                self.assertEqual(first["text_sha256"], hashlib.sha256(b"abc").hexdigest())
                self.assertEqual((first["raw_10"], first["raw_12"], first["raw_16"]), (0x81FF, 0x12345678, 0x9ABCDEF0))
                self.assertEqual([(x["image_number"], x["record_type"]) for x in first["images"]], [(1, 0), (2, 2)])
                self.assertEqual(first["images"][0]["gap_length"], 5)
                self.assertEqual((first["images"][0]["width"], first["images"][0]["height"], first["images"][0]["stride_width"]), (2573, 7, 2592))
                self.assertEqual((first["images"][1]["width"], first["images"][1]["height"], first["images"][1]["dimension_source"]), (155, 1127, "jpeg_sof0"))
                self.assertEqual(first["images"][0]["payload_sha256"], hashlib.sha256(dib(2573, 7)).hexdigest())
                self.assertEqual(pages[1]["text_sha256"], first["text_sha256"])
                self.assertEqual(pages[2]["text_sha256"], hashlib.sha256(b"text").hexdigest())
                self.assertLessEqual(measured.max_request_bytes, 5)
                self.assertGreater(measured.reader_bytes, len(b"abc"))

    def test_cross_page_aliases_keep_both_identities(self) -> None:
        data, index = document("C8")
        row(data, index, 1, 300, b"a", 1)
        row(data, index, 2, 300, b"a", 1)
        image(data, 301, 313, 3, dib(9, 2))
        measured = SourceExtractor(MemoryInput(bytes(data)), "alias")
        first, second = measured.iter_pages()
        self.assertEqual((first["images"][0]["descriptor_offset"], second["images"][0]["descriptor_offset"]), (301, 301))
        self.assertEqual(first["images"][0]["payload_sha256"], second["images"][0]["payload_sha256"])
        self.assertEqual((first["images"][0]["width"], first["images"][0]["stride_width"]), (9, 32))

    def test_unmeasured_type_one_has_no_invented_dimensions(self) -> None:
        data, index = document("HN-B", pages=1)
        row(data, index, 1, 300, b"x", 1)
        image(data, 301, 313, 1, b"opaque")
        result = SourceExtractor(MemoryInput(bytes(data)), "type1").read_page(1)["images"][0]
        self.assertEqual((result["width"], result["height"], result["dimension_source"]), (None, None, "unmeasured_type_1"))

    def test_zero_length_text_hash_is_null(self) -> None:
        data, index = document("C8", pages=1)
        row(data, index, 1, 200, b"", 0)
        result = SourceExtractor(MemoryInput(bytes(data)), "empty").read_page(1)
        self.assertIsNone(result["text_sha256"])
        self.assertEqual(result["images"], [])

    def test_issue_100_structural_error_coordinates(self) -> None:
        data, index = document("HN-B", pages=4, size=13_100)
        data[:12_886] = b"x" * 12_886
        data[0:8] = b"HN\x00\x00\xc8\x00\x00\x00"
        struct.pack_into("<i", data, 0x90, 4)
        row(data, index, 1, 300, b"a", 0)
        struct.pack_into("<iihHII", data, index + 20, 0, 12_886, 1, 0, 0, 0)
        struct.pack_into("<i", data, 12_886, 99)
        row(data, index, 3, 500, b"", -1)
        row(data, index, 4, 13_200, b"bad", 0)
        measured = SourceExtractor(MemoryInput(bytes(data)), "malformed")
        for page, expected in ((2, (12_886, "image type", "unsupported")), (3, (index + 48, "image count", "malformed")), (4, (index + 60, "text span", "truncated"))):
            with self.subTest(page=page), self.assertRaises(SourceMetadataError) as captured:
                measured.read_page(page)
            error = captured.exception
            self.assertEqual((error.offset, error.field, error.kind, error.page), (*expected, page))

    def test_short_reads_zero_progress_overreport_io_failure_and_bad_type(self) -> None:
        data, _ = sample()
        source = MemoryInput(data, max_return=1)
        measured = SourceExtractor(source, "short", SourceLimits(io_chunk_bytes=7))
        self.assertEqual(measured.read_page(1)["image_count"], 2)
        for mode, fragment in (("zero", "zero progress"), ("overreport", "overreported"), ("error", "synthetic read failure"), ("wrong_type", "nonbytes")):
            with self.subTest(mode=mode):
                bad = MemoryInput(data)
                bad.mode = mode
                with self.assertRaises(SourceMetadataError) as captured:
                    SourceExtractor(bad, "bad")
                self.assertEqual((captured.exception.offset, captured.exception.field, captured.exception.kind), (0, "signature", "source"))
                self.assertIn(fragment, str(captured.exception))

    def test_cancellation_at_open_and_during_page(self) -> None:
        data, _ = sample()
        with self.assertRaises(SourceMetadataError) as captured:
            SourceExtractor(MemoryInput(data), "cancel", cancelled=lambda: True)
        self.assertEqual((captured.exception.kind, captured.exception.offset), ("cancelled", 0))
        source = MemoryInput(data)
        state = {"cancel": False}
        measured = SourceExtractor(source, "cancel", cancelled=lambda: state["cancel"])
        state["cancel"] = True
        with self.assertRaises(SourceMetadataError) as captured:
            measured.read_page(1)
        self.assertEqual((captured.exception.kind, captured.exception.page), ("cancelled", 1))

    def test_explicit_work_and_record_limits(self) -> None:
        data, index = sample()
        for limits, field in (
            (SourceLimits(max_source_bytes=100), "source bytes"),
            (SourceLimits(max_pages=2), "pages"),
            (SourceLimits(max_images_per_page=1), "image count"),
            (SourceLimits(max_text_span_bytes=2), "text span bytes"),
            (SourceLimits(max_image_span_bytes=48), "image span bytes"),
            (SourceLimits(max_dimension=100), "image dimension"),
            (SourceLimits(max_image_pixels=10), "image pixels"),
            (SourceLimits(max_reader_bytes=100), "reader bytes"),
            (SourceLimits(max_read_calls=2), "read calls"),
            (SourceLimits(max_jpeg_markers=1), "JPEG markers"),
            (SourceLimits(max_jpeg_header_bytes=10), "JPEG header bytes"),
        ):
            with self.subTest(field=field), self.assertRaises(SourceMetadataError) as captured:
                extractor = SourceExtractor(MemoryInput(data), "limited", limits)
                extractor.read_page(1)
            self.assertEqual(captured.exception.field, field)
        alias, _ = document("C8")
        row(alias, index, 1, 300, b"a", 1)
        row(alias, index, 2, 300, b"a", 1)
        image(alias, 301, 313, 1, b"x")
        with self.assertRaises(SourceMetadataError) as captured:
            list(SourceExtractor(MemoryInput(bytes(alias)), "total", SourceLimits(max_images_total=1)).iter_pages())
        self.assertEqual((captured.exception.field, captured.exception.page), ("images total", 2))

    def test_rejects_truncation_regression_and_invalid_jpeg(self) -> None:
        data, index = sample()
        cases = []
        changed = bytearray(data)
        struct.pack_into("<i", changed, index, len(data) + 1)
        cases.append((changed, "text span", "truncated"))
        changed = bytearray(data)
        struct.pack_into("<i", changed, max(index + 60, 800) + 3 + 4, 1)
        cases.append((changed, "image offset", "malformed"))
        changed = bytearray(data)
        struct.pack_into("<i", changed, max(index + 60, 800) + 3 + 8, 0)
        cases.append((changed, "image length", "malformed"))
        changed = bytearray(data)
        changed[820] ^= 0xFF  # first DIB size byte on this synthetic C8 page
        cases.append((changed, "DIB size", "unsupported"))
        for changed, field, kind in cases:
            with self.subTest(field=field), self.assertRaises(SourceMetadataError) as captured:
                SourceExtractor(MemoryInput(bytes(changed)), "changed").read_page(1)
            self.assertEqual((captured.exception.field, captured.exception.kind), (field, kind))
        changed = bytearray(data)
        changed[881:883] = b"xx"  # second JPEG SOI for sample's first page
        with self.assertRaises(SourceMetadataError) as captured:
            SourceExtractor(MemoryInput(bytes(changed)), "bad-jpeg").read_page(1)
        self.assertEqual(captured.exception.field, "JPEG SOI")

    def test_header_profile_limits_and_file_adapter(self) -> None:
        data, _ = sample("HN-A")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.caj"
            path.write_bytes(data)
            with FileInput(path) as source:
                self.assertEqual(SourceExtractor(source, "file").read_page(1)["image_count"], 2)
        for changed, field in (
            (b"BAD!" + data[4:], "signature"),
            (data[:4] + b"BAD!" + data[8:], "HN marker"),
        ):
            with self.subTest(field=field), self.assertRaises(SourceMetadataError) as captured:
                SourceExtractor(MemoryInput(changed), "bad-header")
            self.assertEqual(captured.exception.field, field)
        changed = bytearray(data)
        struct.pack_into("<i", changed, 0x158, 2)
        with self.assertRaises(SourceMetadataError) as captured:
            SourceExtractor(MemoryInput(bytes(changed)), "outline", SourceLimits(max_outline_records=1))
        self.assertEqual(captured.exception.field, "outline records")

    def test_jpeg_scan_never_reads_across_declared_image_boundary(self) -> None:
        data, index = document("C8", pages=1)
        row(data, index, 1, 300, b"a", 1)
        image(data, 301, 313, 2, b"\xff\xd8\xff\xe0\x00\x10")
        # A valid-looking SOF0 just beyond the six-byte payload must not be used.
        data[319:319 + len(jpeg(8, 9))] = jpeg(8, 9)
        with self.assertRaises(SourceMetadataError) as captured:
            SourceExtractor(MemoryInput(bytes(data)), "bounded-jpeg").read_page(1)
        self.assertEqual((captured.exception.field, captured.exception.kind), ("JPEG segment", "truncated"))


if __name__ == "__main__":
    unittest.main()
