# SPDX-License-Identifier: MIT
"""Original content and failure controls; no vendor runtime/corpus required."""

from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_vendor_fixtures import OriginalBundle

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import vendor_fixtures as fixtures
import vendor_fixture_diff as subject


class FixtureDiff(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.left = OriginalBundle(root / "left")
        self.right = OriginalBundle(root / "right")

    def compare(self, **kwargs):
        return subject.compare(self.left.catalog, self.right.catalog,
                               self.left.roots, self.right.roots, **kwargs)

    def update_pixels(self, bundle, page, data, raster=None):
        image = bundle.pages[page]["image"]
        if raster:
            image["raster"].update(raster)
        image["format"] = "raw"
        bundle.asset(image["artifact"], data)
        bundle.asset(image["payload"], data)
        bundle.emit()

    def update_text(self, bundle, value):
        text = bundle.pages[0]["text"]
        text["normalization"] = None
        text["code_points"] = len(value)
        bundle.asset(text["raw"], value.encode())
        bundle.asset(text["unicode"], value.encode())
        # emit removes references from membership, but the old file must go too.
        old = bundle.roots["bundle"] / "text/1.nfc.utf8"
        old.unlink(missing_ok=True)
        bundle.emit()

    def test_equal_nonblank_pixels_and_unicode_without_empty_text_pass(self):
        result = self.compare()
        self.assertEqual(result["status"], "EQUAL", result)
        self.assertEqual(result["counts"], {"EQUAL": 3, "DIFFERENT": 0, "UNAVAILABLE": 1, "INCOMPARABLE": 0})
        self.assertEqual(result["pages"][1]["text"]["reference"], "NO_TEXT")
        self.assertEqual(result["pages"][0]["image"]["pixels"], 6)
        self.assertEqual(result["basis"], ["original-synthetic"] * 2)

    def test_file_encoding_is_not_pixel_equality(self):
        image = self.right.pages[0]["image"]
        self.right.asset(image["artifact"], b"P6\n# original alternate header\n3 2\n255\n" +
                         self.right.files[("bundle", image["payload"]["path"])])
        self.right.emit()
        self.assertEqual(self.compare()["status"], "EQUAL")

    def test_changed_last_pixel_counts_once_across_channel_chunks(self):
        data = bytearray(self.right.files[("bundle", "pixels/1.rgb")])
        data[-3:] = b"\x03\x04\x05"
        self.update_pixels(self.right, 0, bytes(data))
        for chunk in (1, 2, 4, 7, 65536):
            with self.subTest(chunk=chunk):
                result = self.compare(limits=replace(fixtures.Limits(), read_chunk=chunk))
                self.assertEqual(result["status"], "DIFFERENT", result)
                self.assertEqual(result["pages"][0]["image"]["changed_pixels"], 1)
                self.assertEqual(result["pages"][0]["image"]["first_pixel"], [2, 1])

    def test_bilevel_last_valid_bit_and_row_padding(self):
        raster = {"width": 9, "height": 2, "depth": 1, "channels": 1}
        self.update_pixels(self.left, 0, b"\x80\0\0\0", raster)
        self.update_pixels(self.right, 0, b"\x80\0\0\x80", raster)
        result = self.compare()
        self.assertEqual(result["pages"][0]["image"]["first_pixel"], [8, 1])
        self.assertEqual(result["pages"][0]["image"]["changed_pixels"], 1)
        self.update_pixels(self.right, 0, b"\x80\0\0\x81", raster)
        self.assertEqual(self.compare()["status"], "ERROR")

    def test_16bit_and_bottom_to_top_pixel_coordinates(self):
        raster = {"width": 2, "height": 2, "depth": 16, "channels": 3, "row_order": "bottom-to-top"}
        self.update_pixels(self.left, 0, bytes(range(24)), raster)
        changed = bytes([128] * 6) + bytes(range(6, 24))
        self.update_pixels(self.right, 0, changed, raster)
        result = self.compare(limits=replace(fixtures.Limits(), read_chunk=5))
        self.assertEqual(result["pages"][0]["image"]["changed_pixels"], 1)
        self.assertEqual(result["pages"][0]["image"]["first_pixel"], [0, 1])

    def test_dimensions_differ_even_when_byte_lengths_match(self):
        self.right.pages[0]["image"]["raster"].update(width=2, height=3)
        self.right.emit()
        result = self.compare()
        self.assertEqual(result["status"], "DIFFERENT", result)
        self.assertEqual(result["pages"][0]["image"]["reason"], "image dimensions differ")

    def test_geometry_and_pixel_interpretation_are_not_silently_changed(self):
        for field, value in (("row_order", "bottom-to-top"), ("background", "#000000")):
            with self.subTest(field=field):
                old = self.right.pages[0]["image"]["raster"][field]
                self.right.pages[0]["image"]["raster"][field] = value
                self.right.emit()
                self.assertEqual(self.compare()["status"], "INCOMPARABLE")
                self.right.pages[0]["image"]["raster"][field] = old
        self.right.pages[0]["capture"]["zoom"]["value"] = "2"
        self.right.emit()
        self.assertEqual(self.compare()["status"], "INCOMPARABLE")

    def test_missing_reordered_and_duplicate_pages_are_rejected(self):
        original = deepcopy(self.right.pages)
        for pages in (original[:1], original[::-1], [original[0], original[0]]):
            with self.subTest(pages=len(pages)):
                self.right.manifest["sources"][0]["pages"] = pages
                self.right.emit()
                self.assertEqual(self.compare()["status"], "ERROR")
        self.right.manifest["sources"][0]["pages"] = original

    def test_unexpected_source_and_runtime_identity(self):
        self.right.manifest["sources"][0]["id"] = "different-source"
        self.right.emit()
        self.assertEqual(self.compare()["status"], "DIFFERENT")
        self.right.manifest["runtime"]["settings"]["render"] = "another renderer"
        self.right.emit()
        self.assertEqual(self.compare()["status"], "INCOMPARABLE")

    def test_changed_unicode_character_and_bounded_opt_in_context(self):
        self.update_text(self.left, "a" * 100 + "中" + "z" * 100)
        self.update_text(self.right, "a" * 100 + "文" + "z" * 100)
        result = self.compare()
        text = result["pages"][0]["text"]
        self.assertEqual(result["status"], "DIFFERENT")
        self.assertEqual(text["first_codepoint"], 100)
        self.assertNotIn("context", text)
        context = self.compare(include_text_context=True)["pages"][0]["text"]["context"]
        self.assertEqual(context["start_codepoint"], 84)
        self.assertEqual(len(context["reference"]), 33)
        self.assertEqual(len(context["candidate"]), 33)

    def test_line_order_whitespace_and_length_changes(self):
        self.update_text(self.left, "alpha\nbeta\n")
        for value in ("beta\nalpha\n", "alpha beta\n", "alpha\nbeta", "alpha\nbeta\nextra"):
            with self.subTest(value=value):
                self.update_text(self.right, value)
                self.assertEqual(self.compare()["status"], "DIFFERENT")

    def test_text_mode_changes_are_not_compared_as_raw_equality(self):
        self.right.pages[0]["text"]["mode"] = "enhanced-copy"
        self.right.emit()
        self.assertEqual(self.compare()["status"], "INCOMPARABLE")

    def test_missing_image_is_not_hidden_by_equal_text(self):
        image = self.right.pages[0]["image"]
        for key in ("artifact", "payload"):
            (self.right.roots["bundle"] / image[key]["path"]).unlink()
        self.right.pages[0]["image"] = {k: ("UNAVAILABLE" if k == "status" else None) for k in image}
        self.right.emit()
        result = self.compare()
        self.assertEqual(result["status"], "UNAVAILABLE", result)
        self.assertEqual(result["pages"][0]["text"]["status"], "EQUAL")
        self.assertEqual(result["pages"][0]["image"]["status"], "UNAVAILABLE")

    def test_valid_but_different_source_content_is_rejected_before_comparison(self):
        ref = self.right.manifest["sources"][0]["file"]
        self.right.asset(ref, b"another original source")
        self.right.emit()
        result = self.compare()
        self.assertEqual(result["status"], "DIFFERENT")
        self.assertEqual(result["pages"], [])

    def test_corrupt_truncated_missing_and_oversized_payloads(self):
        path = self.right.roots["bundle"] / "pixels/1.rgb"
        original = path.read_bytes()
        for value in (bytes(len(original)), original[:-1], original + b"x"):
            with self.subTest(size=len(value)):
                path.write_bytes(value)
                self.assertEqual(self.compare()["status"], "ERROR")
        path.unlink()
        self.assertEqual(self.compare()["status"], "ERROR")
        path.write_bytes(original)
        self.assertEqual(self.compare(limits=replace(fixtures.Limits(), artifact_bytes=10))["status"], "ERROR")

    def test_manifest_pin_mismatch(self):
        data = json.loads(self.right.catalog.read_text())
        data["manifest"]["sha256"] = "0" * 64
        self.right.catalog.write_text(json.dumps(data))
        self.assertEqual(self.compare()["status"], "ERROR")

    def test_multipage_reads_collect_only_one_reference_page_at_a_time(self):
        # Two large, distinct pages; instrument actual collection/candidate reads.
        size = 256 * 256 * 3
        for bundle in (self.left, self.right):
            for page in range(2):
                self.update_pixels(bundle, page, bytes([page + 1]) * size,
                                   {"width": 256, "height": 256})
        calls = []
        original = fixtures.Assets.read
        def read(store, ref, collect=False, cap=None, consume=None):
            if ref["path"].startswith("pixels/"):
                calls.append((store, ref["path"], collect, consume is not None))
            return original(store, ref, collect, cap, consume)
        with patch.object(fixtures.Assets, "read", read):
            result = self.compare()
        self.assertEqual(result["status"], "EQUAL", result)
        collections = [(index, row) for index, row in enumerate(calls) if row[2]]
        self.assertEqual([row[1] for _, row in collections], ["pixels/1.rgb", "pixels/2.rgb"])
        for index, row in collections:
            self.assertEqual(calls[index + 1][1:], (row[1], False, True))
        self.assertEqual(result["integrity"][0]["largest_collected_record"], size)
        self.assertLess(result["integrity"][1]["largest_collected_record"], size)
        self.assertLessEqual(max(c["maximum_request"] for c in result["integrity"]), 65536)

    def test_changed_asset_after_comparison_cannot_finish_equal(self):
        original = subject.pixels
        def mutate(*args):
            result = original(*args)
            (self.right.roots["bundle"] / "pixels/1.rgb").write_bytes(b"bad")
            return result
        with patch.object(subject, "pixels", mutate):
            result = self.compare()
        self.assertEqual(result["status"], "ERROR")
        self.assertIn("integrity", result["error"])

    def test_valid_changed_output_mapping_is_detected_before_pixels(self):
        for bundle in (self.left, self.right):
            source = bundle.manifest["sources"][0]
            source["coverage"]["kind"] = "subset"
            source["output_pages"]["value"] = 3
        self.right.pages[1]["output_page"] = 3
        self.left.emit()
        self.right.emit()
        result = self.compare()
        self.assertEqual(result["status"], "DIFFERENT", result)
        self.assertEqual(result["pages"], [])
        self.assertIn("mapping", result["reason"])

    def test_interrupted_comparison_reports_error_instead_of_equal(self):
        with patch.object(subject, "pixels", side_effect=KeyboardInterrupt):
            result = self.compare()
        self.assertEqual(result["status"], "ERROR")
        self.assertIn("interrupted", result["error"])
        self.assertEqual(result["counts"]["EQUAL"], 0)

    def test_deadline_and_read_budget_failure(self):
        result = self.compare(limits=replace(fixtures.Limits(), total_read_bytes=1))
        self.assertEqual(result["status"], "ERROR")
        with patch.object(fixtures.Assets, "check", side_effect=fixtures.FixtureError("reader wall-time limit exceeded")):
            self.assertEqual(self.compare()["status"], "ERROR")

    def test_no_input_and_partial_request(self):
        self.assertEqual(subject.compare()["status"], "NOT_RUN")
        self.assertEqual(subject.compare(self.left.catalog)["status"], "ERROR")

    def command(self):
        args = [sys.executable, str(ROOT / "scripts/vendor_fixture_diff.py")]
        for side, bundle in (("reference", self.left), ("candidate", self.right)):
            args.extend(["--" + side, str(bundle.catalog)])
            for scope, path in bundle.roots.items():
                args.extend([f"--{side}-{scope}", str(path)])
        return args

    def test_cli_exit_status_and_machine_readable_summary(self):
        command = self.command()
        completed = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
        self.assertEqual(json.loads(completed.stdout)["status"], "EQUAL")
        self.update_text(self.right, "changed")
        completed = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(completed.returncode, 1)
        self.assertEqual(json.loads(completed.stdout)["status"], "DIFFERENT")
        self.right.manifest["runtime"]["settings"]["render"] = "different"
        self.right.emit()
        completed = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(completed.returncode, 3)
        self.assertEqual(json.loads(completed.stdout)["status"], "INCOMPARABLE")
        completed = subprocess.run(command[:2] + ["--reference", str(self.left.catalog)],
                                   capture_output=True, text=True, timeout=10)
        self.assertEqual(completed.returncode, 2)
        self.assertEqual(json.loads(completed.stdout)["status"], "ERROR")


if __name__ == "__main__":
    unittest.main()
