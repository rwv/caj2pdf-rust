# SPDX-License-Identifier: MIT
"""Original synthetic checks of the bounded page-text frame diagnostic."""

from dataclasses import asdict
import gzip
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_text_frame as frame  # noqa: E402


PREFIX = bytes(range(20))
MARKERS = b"A0B1C2"
SYNTHETIC = frame.FrameProfile(
    "synthetic", hashlib.sha256(PREFIX).hexdigest(),
    hashlib.sha256(MARKERS).hexdigest())


def decoded_bytes(record_count: int = 3, image_count: int = 2) -> bytes:
    """Invented opaque fields, independent of any external document."""
    records = []
    for index in range(record_count):
        record = bytearray(16)
        record[0:2], record[4:6], record[8:10] = b"A0", b"B1", b"C2"
        record[2:4] = struct.pack("<H", index)
        record[6:8] = struct.pack("<H", index + 1)
        record[10:16] = b"opaque"
        records.append(bytes(record))
    return b"header!!" + b"".join(records) + b"sep!" + b"opaque-image-record-bytes!!!" * image_count


def text_frame(decoded: bytes, *, declared_length: int | None = None,
               compressed: bytes | None = None) -> bytes:
    if declared_length is None:
        declared_length = len(decoded)
    if compressed is None:
        compressed = frame.zlib.compress(decoded)
    return PREFIX + struct.pack("<I", declared_length) + compressed


class ShortReads(io.BytesIO):
    def __init__(self, data: bytes, short_limit: int = 5) -> None:
        super().__init__(data)
        self.requests: list[int] = []
        self.short_limit = short_limit

    def read(self, length: int = -1) -> bytes:
        if length < 0:
            raise AssertionError("unbounded source read")
        self.requests.append(length)
        return super().read(min(length, self.short_limit))


class TextFrameTests(unittest.TestCase):
    def inspect(self, data: bytes, *, image_count: int = 2,
                profile: frame.FrameProfile = SYNTHETIC,
                limits: frame.FrameLimits = frame.FrameLimits(),
                callback=None) -> frame.FrameMetadata:
        return frame.inspect_frame(io.BytesIO(data), offset=0, length=len(data),
                                   image_count=image_count, profile=profile,
                                   limits=limits, decoded_callback=callback)

    def test_complete_frame_reports_only_opaque_locations_and_hashes(self) -> None:
        decoded = decoded_bytes()
        data = text_frame(decoded)
        result = self.inspect(data)
        self.assertEqual(result.decoded_length, len(decoded))
        self.assertEqual(result.record_count, 3)
        self.assertEqual(result.item_records, frame.RecordLocations(8, 16, 3))
        self.assertEqual(result.item_marker_offsets, (0, 4, 8))
        self.assertEqual(result.opaque_separator, frame.RecordLocations(56, 4, 1))
        self.assertEqual(result.trailing_records, frame.RecordLocations(60, 28, 2))
        self.assertEqual(result.prefix_sha256, SYNTHETIC.prefix_sha256)
        self.assertEqual(result.marker_triplet_sha256, SYNTHETIC.marker_sha256)
        self.assertEqual(result.zlib_stream_sha256, hashlib.sha256(data[24:]).hexdigest())
        self.assertEqual(result.decoded_sha256, hashlib.sha256(decoded).hexdigest())
        self.assertEqual(result.decoded_spool_bytes, len(decoded))
        self.assertEqual(result.max_decoder_output_chunk_bytes, len(decoded))
        self.assertFalse(any(isinstance(value, bytes) for value in asdict(result).values()))

    def test_checked_absolute_range_excludes_unrelated_source_bytes(self) -> None:
        decoded = decoded_bytes(2, 1)
        data = text_frame(decoded)
        source = io.BytesIO(b"unrelated" + data + b"outside")
        result = frame.inspect_frame(source, offset=9, length=len(data),
                                     image_count=1, profile=SYNTHETIC)
        self.assertEqual(result.source_offset, 9)
        self.assertEqual(result.source_length, len(data))
        self.assertEqual(result.record_count, 2)

    def test_short_reads_and_high_compression_use_bounded_chunks(self) -> None:
        decoded = decoded_bytes(4_000, 1)
        data = text_frame(decoded)
        source = ShortReads(data)
        result = frame.inspect_frame(source, offset=0, length=len(data),
                                     image_count=1, profile=SYNTHETIC,
                                     limits=frame.FrameLimits(chunk_bytes=127))
        self.assertLessEqual(max(source.requests), 127)
        self.assertEqual(result.max_source_read_request_bytes, max(source.requests))
        self.assertLessEqual(result.max_decoder_output_chunk_bytes, 127)
        self.assertEqual(result.decoded_length, len(decoded))
        self.assertEqual(result.record_count, 4_000)
        one_byte = ShortReads(text_frame(decoded_bytes(1, 0)), short_limit=1)
        result = frame.inspect_frame(one_byte, offset=0, length=len(one_byte.getvalue()),
                                     image_count=0, profile=SYNTHETIC,
                                     limits=frame.FrameLimits(chunk_bytes=1))
        self.assertEqual(max(one_byte.requests), 1)
        self.assertEqual(result.max_decoder_output_chunk_bytes, 1)

    def test_validated_private_spool_callback_has_a_scoped_lifetime(self) -> None:
        decoded = decoded_bytes()
        retained = []
        received = bytearray()

        def receive(spool, metadata) -> None:
            self.assertEqual(spool.tell(), 0)
            self.assertEqual(metadata.decoded_sha256, hashlib.sha256(decoded).hexdigest())
            retained.append(spool)
            while block := spool.read(7):
                received.extend(block)

        self.inspect(text_frame(decoded), callback=receive)
        self.assertEqual(bytes(received), decoded)
        self.assertTrue(retained[0].closed)
        called = []
        with self.assertRaises(frame.FrameError):
            self.inspect(text_frame(decoded) + b"tail", callback=lambda *_: called.append(True))
        self.assertEqual(called, [])

    def test_empty_item_and_trailing_sections_have_no_marker_claim(self) -> None:
        result = self.inspect(text_frame(decoded_bytes(0, 0)), image_count=0)
        self.assertEqual(result.record_count, 0)
        self.assertIsNone(result.marker_triplet_sha256)
        self.assertEqual(result.opaque_separator, frame.RecordLocations(8, 4, 1))
        self.assertEqual(result.trailing_records, frame.RecordLocations(12, 28, 0))

    def test_callback_failure_still_closes_the_private_spool(self) -> None:
        retained = []

        def fail(spool, _metadata) -> None:
            retained.append(spool)
            raise RuntimeError("caller failure")

        with self.assertRaisesRegex(RuntimeError, "caller failure"):
            self.inspect(text_frame(decoded_bytes()), callback=fail)
        self.assertTrue(retained[0].closed)

    def test_prefix_and_marker_checks_reject_changes(self) -> None:
        decoded = decoded_bytes()
        bad_prefix = bytearray(text_frame(decoded))
        bad_prefix[0] ^= 1
        with self.assertRaisesRegex(frame.FrameError, "prefix"):
            self.inspect(bytes(bad_prefix))
        bad_first = bytearray(decoded)
        bad_first[8] ^= 1
        with self.assertRaisesRegex(frame.FrameError, "pinned profile"):
            self.inspect(text_frame(bytes(bad_first)))
        bad_later = bytearray(decoded)
        bad_later[8 + 16 + 4] ^= 1
        with self.assertRaisesRegex(frame.FrameError, "item record 1"):
            self.inspect(text_frame(bytes(bad_later)))

    def test_unpinned_synthetic_profile_still_checks_marker_consistency(self) -> None:
        profile = frame.FrameProfile("unclassified-synthetic")
        result = self.inspect(text_frame(decoded_bytes()), profile=profile)
        self.assertEqual(result.marker_triplet_sha256, SYNTHETIC.marker_sha256)
        bad = bytearray(decoded_bytes())
        bad[8 + 16 + 8] ^= 1
        with self.assertRaisesRegex(frame.FrameError, "marker triplet"):
            self.inspect(text_frame(bytes(bad)), profile=profile)

    def test_checksum_truncation_and_non_zlib_formats_are_rejected(self) -> None:
        decoded = decoded_bytes()
        data = text_frame(decoded)
        corrupt = bytearray(data)
        corrupt[-1] ^= 1
        compressor = frame.zlib.compressobj(wbits=-frame.zlib.MAX_WBITS)
        raw = compressor.compress(decoded) + compressor.flush()
        cases = (
            (bytes(corrupt), "checksum"),
            (data[:-1], "EOF"),
            (text_frame(decoded, compressed=gzip.compress(decoded)), "zlib"),
            (text_frame(decoded, compressed=raw), "zlib"),
        )
        for candidate, error in cases:
            with self.subTest(error=error):
                with self.assertRaisesRegex(frame.FrameError, error):
                    self.inspect(candidate)

    def test_tail_or_second_stream_is_rejected_even_with_valid_first_stream(self) -> None:
        decoded = decoded_bytes()
        for tail in (b"x", frame.zlib.compress(decoded)):
            with self.subTest(tail_length=len(tail)):
                with self.assertRaisesRegex(frame.FrameError, "trailing bytes"):
                    self.inspect(text_frame(decoded) + tail)
        with self.assertRaisesRegex(frame.FrameError, "trailing bytes"):
            self.inspect(text_frame(decoded) + b"many tail bytes",
                         limits=frame.FrameLimits(chunk_bytes=1))

    def test_declared_length_overflow_and_underflow_are_rejected(self) -> None:
        decoded = decoded_bytes()
        with self.assertRaisesRegex(frame.FrameError, "exceeds declared length"):
            self.inspect(text_frame(decoded, declared_length=len(decoded) - 16))
        with self.assertRaisesRegex(frame.FrameError, "differs from actual output"):
            self.inspect(text_frame(decoded, declared_length=len(decoded) + 16))
        with self.assertRaisesRegex(frame.FrameError, "decoded length exceeds limit"):
            self.inspect(text_frame(decoded),
                         limits=frame.FrameLimits(max_decoded_bytes=len(decoded) - 1))

    def test_record_layout_and_count_limits_are_checked_before_inflation(self) -> None:
        decoded = decoded_bytes()
        for declared in (11, len(decoded) + 1):
            with self.subTest(declared=declared):
                with self.assertRaisesRegex(frame.FrameError, "8\\+16"):
                    self.inspect(text_frame(decoded, declared_length=declared))
        with self.assertRaisesRegex(frame.FrameError, "item record count"):
            self.inspect(text_frame(decoded), limits=frame.FrameLimits(max_records=2))
        for count in (-1, frame.FrameLimits().max_records + 1):
            with self.subTest(image_count=count):
                with self.assertRaisesRegex(frame.FrameError, "image count"):
                    self.inspect(text_frame(decoded), image_count=count)
        with self.assertRaisesRegex(frame.FrameError, "8\\+16"):
            self.inspect(text_frame(decoded), image_count=1)

    def test_input_ranges_limits_and_nonseekable_source_are_rejected(self) -> None:
        data = text_frame(decoded_bytes())
        for offset, length in ((-1, len(data)), (0, 23), (1, len(data))):
            with self.subTest(offset=offset, length=length):
                with self.assertRaises(frame.FrameError):
                    frame.inspect_frame(io.BytesIO(data), offset=offset, length=length,
                                        image_count=2, profile=SYNTHETIC)
        with self.assertRaisesRegex(frame.FrameError, "exceeds limit"):
            self.inspect(data, limits=frame.FrameLimits(max_span_bytes=len(data) - 1))
        for limits in (frame.FrameLimits(chunk_bytes=0),
                       frame.FrameLimits(chunk_bytes=frame.IO_CHUNK + 1)):
            with self.subTest(limits=limits):
                with self.assertRaises(frame.FrameError):
                    self.inspect(data, limits=limits)

        class ForwardOnly:
            def seek(self, *_):
                raise io.UnsupportedOperation("forward-only")

        with self.assertRaisesRegex(frame.FrameError, "seekable source"):
            frame.inspect_frame(ForwardOnly(), offset=0, length=len(data),
                                image_count=2, profile=SYNTHETIC)

    def test_source_zero_progress_and_overreport_are_rejected(self) -> None:
        data = text_frame(decoded_bytes())

        class ZeroProgress(io.BytesIO):
            def read(self, length=-1):
                return b""

        class Overreport(io.BytesIO):
            def read(self, length=-1):
                return b"X" * (length + 1)

        for source, error in ((ZeroProgress(data), "truncated"),
                              (Overreport(data), "more bytes")):
            with self.subTest(error=error):
                with self.assertRaisesRegex(frame.FrameError, error):
                    frame.inspect_frame(source, offset=0, length=len(data),
                                        image_count=2, profile=SYNTHETIC)

    def test_clean_cli_is_not_a_private_compatibility_pass(self) -> None:
        result = subprocess.run([sys.executable, str(ROOT / "scripts/hnc8_text_frame.py"),
                                 "--json"], check=True, capture_output=True, text=True)
        self.assertEqual(json.loads(result.stdout),
                         {"status": "NOT_RUN", "private_comparisons": 0})
        partial = subprocess.run([sys.executable, str(ROOT / "scripts/hnc8_text_frame.py"),
                                  "--variant", "c8"], capture_output=True, text=True)
        self.assertEqual(partial.returncode, 2)
        self.assertIn("required together", partial.stderr)


if __name__ == "__main__":
    unittest.main()
