# SPDX-License-Identifier: MIT
"""Original synthetic checks for the bounded HN/C8 PDF layout extractor."""

from __future__ import annotations

from dataclasses import replace
from pathlib import Path
import re
import shutil
import sys
import tempfile
import unittest
from unittest import mock
import zlib


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_layout_pdf as layout  # noqa: E402


TOOLS = {name: name for name in ("qpdf", "mutool", "pdfimages")}
CONTENT = (
    b"q\n2 0 0 3 4 5 cm\nq\n10 0 0 4 2 3 cm\n/Im0 Do\nQ\nQ\n"
    b"q\n0 40 -10 0 150 100 cm\n/ImAlias Do\nQ\n"
    b"q\n20 0 0 20 10 10 cm\n/Im1 Do\nQ\n"
    b"q\n40 0 0 20 100 100 cm\n/Im0 Do\nQ\n"
)


def stream(dictionary: bytes, payload: bytes) -> bytes:
    return (b"<< " + dictionary + b" /Length " + str(len(payload)).encode() +
            b" >>\nstream\n" + payload + b"\nendstream")


def synthetic_pdf(path: Path, *, content: bytes = CONTENT,
                  media_box: bytes = b"[0 0 200 200]",
                  page_extra: bytes = b"") -> Path:
    red = zlib.compress(b"\xff\x00\x00")
    green_blue = zlib.compress(b"\x00\xff\x00\x00\x00\xff")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (b"<< /Type /Page /Parent 2 0 R /MediaBox " + media_box + b" " +
         page_extra + b" /Resources << /XObject << /Im0 5 0 R " +
         b"/ImAlias 5 0 R /Im1 6 0 R >> >> /Contents 4 0 R >>"),
        stream(b"", content),
        stream(b"/Type /XObject /Subtype /Image /Width 1 /Height 1 "
               b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode", red),
        stream(b"/Type /XObject /Subtype /Image /Width 2 /Height 1 "
               b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
               green_blue),
    ]
    result = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        result.extend(f"{offset:010} 00000 n \n".encode())
    result.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\n"
                  f"startxref\n{xref}\n%%EOF\n".encode())
    path.write_bytes(result)
    return path


class LayoutPdfParserTests(unittest.TestCase):
    def test_nested_matrix_composition_and_alias_order(self) -> None:
        resources = {
            "/Im0": {"xobject_name": "/Im0", "object_id": 5},
            "/ImAlias": {"xobject_name": "/ImAlias", "object_id": 5},
            "/Im1": {"xobject_name": "/Im1", "object_id": 6},
        }
        draws = layout._draws(CONTENT, resources, [0, 0, 200, 200],
                              layout.PdfMetadataLimits())
        self.assertEqual([row["xobject_name"] for row in draws],
                         ["/Im0", "/ImAlias", "/Im1", "/Im0"])
        self.assertEqual([row["object_id"] for row in draws], [5, 5, 6, 5])
        self.assertEqual(draws[0]["pdf_ctm"], [20, 0, 0, 12, 8, 14])
        self.assertEqual(draws[1]["top_left_ctm"], [0, -40, 10, 0, 140, 100])

    def test_rejects_malformed_content_and_unsupported_forms(self) -> None:
        images = {"/Im0": {"object_id": 5}}
        limits = layout.PdfMetadataLimits()
        for content, error in (
            (b"Q", layout.PdfMetadataError),
            (b"q 1 0 0 1 0 0 cm /Im0 Do", layout.PdfMetadataError),
            (b"0 0 0 0 0 0 cm /Im0 Do", layout.PdfMetadataError),
            (b"1 0 0 1 NaN 0 cm /Im0 Do", layout.PdfMetadataUnsupported),
            (b"q /Fm0 Do Q", layout.PdfMetadataUnsupported),
            (b"q 1 0 0 1 0 0 cm /Im0 Do BT Q", layout.PdfMetadataUnsupported),
            (b"q /Im0 Do /Im0", layout.PdfMetadataError),
        ):
            with self.subTest(content=content), self.assertRaises(error):
                layout._draws(content, images, [0, 0, 200, 200], limits)

    def test_rejects_invalid_boxes_and_trace_xml(self) -> None:
        for values in ([0, 0, 0, 3], [0, 3, 2, 0], [0, 0, float("nan"), 3]):
            with self.subTest(values=values), self.assertRaises(layout.PdfMetadataError):
                layout._box(values, "test")
        for xml in (
            b'<document><page mediabox="0 0 2 2"><fill_image transform="1 0 0" '
            b'width="1" height="1"/></page></document>',
            b'<document><page mediabox="0 0 2 2"><fill_image transform="1 0 0 1 NaN 0" '
            b'width="1" height="1"/></page></document>',
            b'<document><page mediabox="0 0 2 2"><fill_image_mask/></page></document>',
            b'<document><page mediabox="0 0 2 2">',
        ):
            trace = layout._Trace(layout.PdfMetadataLimits())
            with self.subTest(xml=xml), self.assertRaises(layout.PdfMetadataError):
                trace.feed(xml)
                trace.finish()

    def test_binary_version_and_page_limits_are_positive(self) -> None:
        with self.assertRaises(ValueError):
            layout.PdfMetadataLimits(max_pdf_bytes=0)
        with self.assertRaises(layout.PdfMetadataError):
            layout._mutool_pages(b'<page pagenum="1"><MediaBox l="0" b="0" '
                                 b'r="0" t="2" /></page>', layout.PdfMetadataLimits())
        with self.assertRaisesRegex(layout.PdfMetadataError, "column count"):
            layout._pdfimages(b"page num type width height\n-----\n1 0 image 1\n")

    def test_child_output_failure_timeout_and_cancellation(self) -> None:
        usage = layout._Usage()
        limits = replace(layout.PdfMetadataLimits(), timeout_seconds=0.3)
        for code, expected in (
            ("print('x' * 100)", "output exceeds limit"),
            ("import sys; sys.exit(7)", "exited 7"),
            ("import time; time.sleep(10)", "timed out"),
        ):
            with self.subTest(code=code), self.assertRaisesRegex(
                    layout.PdfMetadataError, expected):
                layout._run([sys.executable, "-c", code], "synthetic child",
                            limits, usage, 16)
        with self.assertRaises(layout.PdfMetadataCancelled):
            layout._run([sys.executable, "-c", "print('should not run')"],
                        "cancelled child", limits, layout._Usage(cancelled=lambda: True), 64)


class LayoutPdfExternalToolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        missing = [name for name in TOOLS if not shutil.which(name)]
        if missing:
            raise AssertionError(f"required PDF validators are absent: {', '.join(missing)}")

    def setUp(self) -> None:
        self.folder = tempfile.TemporaryDirectory(prefix="caj2pdf-layout-pdf-")
        self.addCleanup(self.folder.cleanup)
        self.pdf = synthetic_pdf(Path(self.folder.name) / "synthetic.pdf")

    def extract(self, **kwargs: object) -> dict:
        return layout.extract_pdf_metadata(self.pdf, TOOLS, **kwargs)

    def test_all_tools_agree_on_repeated_alias_draws(self) -> None:
        report = self.extract()
        self.assertEqual((report["status"], report["page_count"], report["draw_count"]),
                         ("PASS", 1, 4))
        draws = report["pages"][0]["draws"]
        self.assertEqual([d["object_id"] for d in draws], [5, 5, 6, 5])
        self.assertEqual([d["xobject_name"] for d in draws],
                         ["/Im0", "/ImAlias", "/Im1", "/Im0"])
        self.assertEqual(draws[0]["pdf_ctm"], [20, 0, 0, 12, 8, 14])
        self.assertEqual(draws[0]["raw_stream_sha256"],
                         draws[1]["raw_stream_sha256"])
        self.assertEqual(draws[0]["raw_stream_sha256"],
                         draws[3]["raw_stream_sha256"])
        self.assertNotEqual(draws[0]["raw_stream_sha256"],
                            draws[2]["raw_stream_sha256"])
        self.assertEqual(report["resources"]["max_temporary_bytes"], 0)
        self.assertGreater(report["resources"]["max_child_rss_kib"], 0)
        self.assertTrue(all(report["tools"][name]["version"] for name in TOOLS))

    def test_nonzero_mediabox_origin_is_normalized_only_in_trace(self) -> None:
        synthetic_pdf(self.pdf, media_box=b"[10 20 210 220]")
        report = self.extract()
        self.assertEqual(report["pages"][0]["media_box"], [10, 20, 210, 220])
        self.assertEqual(report["pages"][0]["draws"][0]["top_left_ctm"],
                         [20, 0, 0, 12, -2, 194])

    def test_invalid_box_singular_matrix_and_unsupported_form_fail(self) -> None:
        cases = (
            (dict(media_box=b"[0 0 0 200]"), layout.PdfMetadataError),
            (dict(content=b"0 0 0 0 0 0 cm /Im0 Do"), layout.PdfMetadataError),
            (dict(content=b"/Fm0 Do"), layout.PdfMetadataUnsupported),
        )
        for options, error in cases:
            with self.subTest(options=options):
                synthetic_pdf(self.pdf, **options)
                with self.assertRaises(error):
                    self.extract()

    def test_truncated_pdf_and_bounded_outputs_fail(self) -> None:
        with self.assertRaisesRegex(layout.PdfMetadataError, "required qpdf binary"):
            layout.extract_pdf_metadata(
                self.pdf, {**TOOLS, "qpdf": "missing-caj2pdf-qpdf-validator"})
        with self.assertRaisesRegex(layout.PdfMetadataError, "exceeds"):
            self.extract(limits=replace(layout.PdfMetadataLimits(), max_pdf_bytes=32))
        with self.assertRaisesRegex(layout.PdfMetadataError, "exceed"):
            self.extract(limits=replace(layout.PdfMetadataLimits(), max_draws_per_page=3))
        with self.assertRaisesRegex(layout.PdfMetadataError, "exceeds"):
            self.extract(limits=replace(layout.PdfMetadataLimits(), max_stream_bytes=2))
        with self.assertRaisesRegex(layout.PdfMetadataError, "exceeds"):
            self.extract(limits=replace(layout.PdfMetadataLimits(), max_json_bytes=64))
        self.pdf.write_bytes(self.pdf.read_bytes()[:100])
        with self.assertRaises(layout.PdfMetadataError):
            self.extract()

    def test_cross_tool_disagreement_and_cancelled_call_fail(self) -> None:
        original = layout._run

        def wrong_hash(arguments: list[str], label: str, *args: object,
                       **kwargs: object):
            result, length = original(arguments, label, *args, **kwargs)
            if label == "mutool raw image":
                return "0" * 64, length
            return result, length

        with mock.patch.object(layout, "_run", side_effect=wrong_hash):
            with self.assertRaisesRegex(layout.PdfMetadataError, "stream hash differs"):
                self.extract()

        def wrong_trace(arguments: list[str], label: str, *args: object,
                        **kwargs: object):
            if label == "mutool page trace":
                consumer = kwargs.pop("consume")
                trace, _ = original(arguments, label, *args, **kwargs)
                assert isinstance(trace, bytes)
                changed = re.sub(rb'transform="[^"]+"',
                                 b'transform="1 0 0 1 0 0"', trace, count=1)
                consumer(changed)
                return b"", len(changed)
            return original(arguments, label, *args, **kwargs)

        with mock.patch.object(layout, "_run", side_effect=wrong_trace):
            with self.assertRaisesRegex(layout.PdfMetadataError, "CTM differs"):
                self.extract()

        original_listing = layout._pdfimages

        def wrong_identity(data: bytes) -> list[dict]:
            rows = original_listing(data)
            rows[0]["object_id"] = 1234
            return rows

        with mock.patch.object(layout, "_pdfimages", side_effect=wrong_identity):
            with self.assertRaisesRegex(layout.PdfMetadataError, "Poppler image"):
                self.extract()

        original_file_hash = layout._file_hash
        pdf_calls = 0

        def changed_pdf(path: Path, *args: object, **kwargs: object):
            nonlocal pdf_calls
            result = original_file_hash(path, *args, **kwargs)
            if path == self.pdf:
                pdf_calls += 1
                if pdf_calls == 2:
                    return "0" * 64, result[1]
            return result

        with mock.patch.object(layout, "_file_hash", side_effect=changed_pdf):
            with self.assertRaisesRegex(layout.PdfMetadataError, "PDF changed"):
                self.extract()

        qpdf_path = Path(shutil.which("qpdf") or "").resolve()
        qpdf_calls = 0

        def changed_tool(path: Path, *args: object, **kwargs: object):
            nonlocal qpdf_calls
            result = original_file_hash(path, *args, **kwargs)
            if path == qpdf_path:
                qpdf_calls += 1
                if qpdf_calls == 2:
                    return "0" * 64, result[1]
            return result

        with mock.patch.object(layout, "_file_hash", side_effect=changed_tool):
            with self.assertRaisesRegex(layout.PdfMetadataError, "binary changed"):
                self.extract()
        with self.assertRaises(layout.PdfMetadataCancelled):
            self.extract(cancelled=lambda: True)


if __name__ == "__main__":
    unittest.main()
