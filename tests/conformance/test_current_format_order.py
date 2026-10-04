# SPDX-License-Identifier: MIT
"""Original controls for source-page identity checks, not corpus substitutes."""

from contextlib import nullcontext
import hashlib
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import current_format_order as order
from current_formats import Commands


class PageIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_image_order_content_and_missing_images_are_detected(self):
        first = ("bits", 8, 1, "first")
        second = ("jpeg", "second")
        self.assertTrue(order.images_match([first, second], [first, second]))
        self.assertFalse(order.images_match([first, second], [second, first]))
        self.assertFalse(order.images_match([first, second], [first]))
        self.assertFalse(order.images_match([first], [("bits", 8, 1, "changed")]))
        self.assertFalse(order.images_match([], []))
        # Repeated byte-identical source groups affect identity counts, not
        # this check's intentionally separate placement/alias semantics.
        self.assertTrue(order.images_match([first, second] * 2, [first, second]))
        self.assertFalse(order.images_match([first, second, first], [first, second]))

    def test_bitmap_hash_masks_only_unused_bits_and_preserves_row_order(self):
        image = self.root / "image.pbm"
        image.write_bytes(b"P4\n9 2\n\x80\xff\x01\x7f")
        expected = hashlib.sha256(b"\x80\x80\x01\x00").hexdigest()
        self.assertEqual(order.extracted_image(image), ("bits", 9, 2, expected))
        image.write_bytes(b"P4\n9 2\n\x01\x7f\x80\xff")
        self.assertNotEqual(order.extracted_image(image)[-1], expected)
        for raster in (b"\x80", b"\x80\xff\x01\x7fextra"):
            image.write_bytes(b"P4\n9 2\n" + raster)
            with self.assertRaises(ValueError):
                order.extracted_image(image)

    def test_jpeg_identity_is_exact_and_unknown_representation_fails(self):
        image = self.root / "image.jpg"
        image.write_bytes(b"original opaque JPEG identity control")
        self.assertEqual(order.extracted_image(image), ("jpeg", hashlib.sha256(image.read_bytes()).hexdigest()))
        with self.assertRaises(ValueError):
            order.extracted_image(self.root / "image.png")

    def test_caj_table_keeps_zero_length_rows_and_declared_order(self):
        source = self.root / "source.caj"
        data = bytearray(0x30)
        data[:4] = b"CAJ\0"
        struct.pack_into("<II", data, 0x10, 3, 0x30)
        data.extend(b"".join(struct.pack("<III", 100, length, page) for length, page in [(0, 91), (3, 7), (0, 45)]))
        source.write_bytes(data)
        self.assertEqual(order.caj_page_ids(source), ["91 0 R", "7 0 R", "45 0 R"])
        source.write_bytes(data[:-1])
        with self.assertRaises(ValueError):
            order.caj_page_ids(source)

    def test_oracle_must_bind_source_and_encoded_image_not_just_page_number(self):
        entry = {"page": 1, "image": 1, "encoded_sha256": "encoded", "offset": 200,
                 "length": 20, "decoder_result": "PASS", "width": 9, "height": 2,
                 "visible_bits_sha256": "pixels"}
        fixture = {"sample": {"source_sha256": "source", "images": [entry]}}
        image = {"record_type": 0, "image_number": 1, "payload_sha256": "encoded",
                 "payload_offset": 200, "payload_length": 20}
        with patch.object(order, "oracle", return_value=fixture):
            self.assertEqual(order.expected_image({"page_number": 1}, image, "source", "sample"),
                             ("bits", 9, 2, "pixels"))
            with self.assertRaises(ValueError):
                order.expected_image({"page_number": 1}, image, "changed", "sample")
            with self.assertRaises(ValueError):
                order.expected_image({"page_number": 1}, dict(image, payload_sha256="changed"), "source", "sample")
            entry["decoder_result"] = "NOT_RUN"
            with self.assertRaises(ValueError):
                order.expected_image({"page_number": 1}, image, "source", "sample")

    def test_source_without_pinned_oracle_is_not_run_not_failed(self):
        image = {"record_type": 0, "image_number": 1, "payload_sha256": "encoded",
                 "payload_offset": 200, "payload_length": 20}
        page = {"page_number": 1, "images": [image]}

        class Extractor:
            def __init__(self, stream, source_id):
                pass

            def iter_pages(self):
                yield page

        with patch.object(order, "oracle", return_value={}):
            with self.assertRaises(order.MissingOracle):
                order.expected_image(page, image, "source", "new-sample")
            with patch.object(order, "pdf_pages", return_value=[{}]), patch.object(
                order, "FileInput", return_value=nullcontext(None),
            ), patch.object(order, "SourceExtractor", Extractor):
                result = order.check(None, self.root, self.root, {"detected_type": "HN", "id": "new-sample",
                                                                  "sha256": "source"}, {"page_count": 1}, "new")
        self.assertEqual(result["status"], "NOT_RUN")
        self.assertIn("new-sample", result["reason"])

    @unittest.skipUnless(shutil.which("qpdf"), "qpdf is required for the original PDF control")
    def test_real_pdf_page_reordering_is_rejected(self):
        source = ROOT / "tests/fixtures/valid_nested_outline.pdf"
        output = self.root / "swapped.pdf"
        subprocess.run(["qpdf", str(source), "--pages", ".", "2,1", "--", str(output)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        commands = Commands(self.root)
        self.assertEqual(order.check(commands, source, source, {"detected_type": "PDF"}, {}, "same")["status"], "PASS")
        self.assertEqual(order.check(commands, source, output, {"detected_type": "PDF"}, {}, "swapped")["status"], "FAIL")

    def test_source_outline_keeps_unicode_depth_and_destination(self):
        source = self.root / "outline.caj"
        data = bytearray(0x114 + 308)
        struct.pack_into("<I", data, 0x110, 1)
        title = "Original 目录".encode("gb18030")
        data[0x114:0x114 + len(title)] = title
        data[0x114 + 280] = ord("2")
        struct.pack_into("<I", data, 0x114 + 304, 2)
        source.write_bytes(data)
        expected = hashlib.sha256()
        order.conformance.update_outline_hash(expected, {
            "depth": 1, "title": "Original 目录", "page": 2, "destination": "#page=2",
        })
        self.assertEqual(order.source_outline_hash(source, "CAJ", {"page_count": 2}),
                         (1, expected.hexdigest()))
        with self.assertRaises(ValueError):
            order.source_outline_hash(source, "CAJ", {"page_count": 1})
        self.assertIsNone(order.source_outline_hash(source, "C8", {"variant": "C8"}))


if __name__ == "__main__":
    unittest.main()
