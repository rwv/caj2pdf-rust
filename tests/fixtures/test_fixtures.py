# SPDX-License-Identifier: MIT
"""Structural and reproducibility checks for independently authored fixtures."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = ROOT / "tests" / "fixtures"
VALID_PDFS = ("valid_nested_outline.pdf", "valid_out_of_order_objects.pdf")


def xref_entries(pdf: bytes) -> list[bytes]:
    tail = re.search(rb"\nstartxref\n(\d+)\n%%EOF\n\Z", pdf)
    if tail is None:
        raise AssertionError("PDF has no complete final startxref/EOF")
    xref_offset = int(tail.group(1))
    lines = pdf[xref_offset:].splitlines()
    if lines[:2] != [b"xref", b"0 12"]:
        raise AssertionError("startxref does not point to the 12-entry xref table")
    return lines[2:14]


class FixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = json.loads((FIXTURE_DIR / "manifest.json").read_text(encoding="utf-8"))

    def test_manifest_digests_and_provenance(self) -> None:
        self.assertEqual(self.manifest["schema_version"], 1)
        self.assertEqual(self.manifest["license"], "MIT")
        self.assertEqual(self.manifest["generator"], "scripts/generate_fixtures.py")
        entries = self.manifest["fixtures"]
        names = [entry["path"] for entry in entries]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(names, sorted(names))
        self.assertEqual(len(entries), 13)
        for entry in entries:
            with self.subTest(path=entry["path"]):
                path = FIXTURE_DIR / entry["path"]
                self.assertEqual(path.parent, FIXTURE_DIR)
                payload = path.read_bytes()
                self.assertEqual(entry["size"], len(payload))
                self.assertEqual(entry["sha256"], hashlib.sha256(payload).hexdigest())
                self.assertIn(entry["expected_outcome"], ("valid", "malformed"))
                self.assertTrue(entry["condition"])

    def test_regeneration_is_byte_identical(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [sys.executable, str(ROOT / "scripts" / "generate_fixtures.py"),
                 "--output-dir", directory],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            names = [entry["path"] for entry in self.manifest["fixtures"]]
            for name in (*names, "manifest.json"):
                with self.subTest(path=name):
                    self.assertEqual(
                        (Path(directory) / name).read_bytes(),
                        (FIXTURE_DIR / name).read_bytes(),
                    )

    def test_valid_pdfs_have_correct_xref_entries(self) -> None:
        for name in VALID_PDFS:
            with self.subTest(path=name):
                pdf = (FIXTURE_DIR / name).read_bytes()
                self.assertTrue(pdf.startswith(b"%PDF-1.7\n"))
                entries = xref_entries(pdf)
                self.assertEqual(len(entries), 12)
                self.assertEqual(entries[0], b"0000000000 65535 f ")
                for number, entry in enumerate(entries[1:], 1):
                    self.assertRegex(entry, rb"^\d{10} 00000 n $")
                    offset = int(entry[:10])
                    self.assertTrue(pdf[offset:].startswith(f"{number} 0 obj\n".encode("ascii")))

    def test_valid_pdf_page_outline_and_embedded_stream(self) -> None:
        for name in VALID_PDFS:
            with self.subTest(path=name):
                pdf = (FIXTURE_DIR / name).read_bytes()
                self.assertIn(b"/Kids [3 0 R 4 0 R] /Count 2", pdf)
                self.assertIn(b"/MediaBox [0 0 200 300]", pdf)
                self.assertIn(b"/MediaBox [0 0 400 250]", pdf)
                self.assertIn(b"/Title (Part One) /Parent 7 0 R /First 9 0 R", pdf)
                self.assertIn(b"/Count 1 /Dest [3 0 R /Fit]", pdf)
                self.assertIn(b"/Title (Chapter Two) /Parent 8 0 R /Dest [4 0 R /XYZ 0 250 null]", pdf)
                self.assertIn(b"/Names [(marker.bin) 11 0 R]", pdf)
                self.assertIn(b"/EF << /F 10 0 R >>", pdf)

                stream = re.search(
                    rb"(?m)^10 0 obj\n<< /Type /EmbeddedFile /Length (\d+) >>\nstream\n",
                    pdf,
                )
                self.assertIsNotNone(stream)
                assert stream is not None
                length = int(stream.group(1))
                payload = pdf[stream.end():stream.end() + length]
                self.assertEqual(pdf[stream.end() + length:][:10], b"\nendstream")
                self.assertIn(b"\x00\xffendstream\nendobj\n5 0 obj\n", payload)
                self.assertIn(b"xref\nstartxref\n%%EOF\n", payload)

    def test_out_of_order_pdf_is_valid_but_not_numerically_ordered(self) -> None:
        pdf = (FIXTURE_DIR / "valid_out_of_order_objects.pdf").read_bytes()
        offsets = [int(entry[:10]) for entry in xref_entries(pdf)[1:]]
        physical_order = [number for _, number in sorted((offset, number) for number, offset in enumerate(offsets, 1))]
        self.assertEqual(physical_order[0], 10)
        self.assertNotEqual(physical_order, list(range(1, 12)))

    def test_malformed_pdf_conditions_are_present(self) -> None:
        oversized = (FIXTURE_DIR / "invalid_stream_length.pdf").read_bytes()
        stream = re.search(rb"(?m)^5 0 obj\n<< /Length (\d+) >>\nstream\n", oversized)
        self.assertIsNotNone(stream)
        assert stream is not None
        declared = int(stream.group(1))
        actual = oversized.index(b"\nendstream", stream.end()) - stream.end()
        self.assertGreater(declared, actual)

        invalid_xref = (FIXTURE_DIR / "invalid_xref_offset.pdf").read_bytes()
        self.assertGreater(int(xref_entries(invalid_xref)[5][:10]), len(invalid_xref))

        count = (FIXTURE_DIR / "invalid_page_count.pdf").read_bytes()
        self.assertIn(b"/Kids [3 0 R 4 0 R] /Count 3", count)

        duplicate = (FIXTURE_DIR / "duplicate_object.pdf").read_bytes()
        self.assertEqual(duplicate.count(b"\n6 0 obj\n"), 2)

        repairable = (FIXTURE_DIR / "repairable_duplicate_mediabox_tail.pdf").read_bytes()
        self.assertEqual(repairable.count(b"/MediaBox [0 0 612 792]"), 2)
        self.assertIn(b"%%EOF\nWebFastLoadP", repairable)

        truncated = (FIXTURE_DIR / "truncated_xref.pdf").read_bytes()
        self.assertIn(b"xref\n0 12\n", truncated)
        self.assertNotIn(b"trailer\n", truncated)
        self.assertFalse(truncated.endswith(b"%%EOF\n"))

    def test_short_caj_family_signatures_are_only_probes(self) -> None:
        signatures = {
            "truncated_caj.caj": b"CAJ",
            "truncated_hn.hn": b"HN",
            "truncated_c8.c8": b"\xc8\x00\x00\x00",
            "truncated_kdh.kdh": b"KDH",
            "truncated_teb.teb": b"TEB",
        }
        for name, signature in signatures.items():
            with self.subTest(path=name):
                self.assertEqual((FIXTURE_DIR / name).read_bytes(), signature)
                entry = next(entry for entry in self.manifest["fixtures"] if entry["path"] == name)
                self.assertEqual(entry["expected_outcome"], "malformed")


if __name__ == "__main__":
    unittest.main()
