# SPDX-License-Identifier: MIT
"""Validate original controls with independent installed PDF tools."""

import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "canary_controls", ROOT / "scripts" / "cajviewer_canary_fixtures.py")
CONTROLS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTROLS)


class OriginalControlsTests(unittest.TestCase):
    def test_deterministic_pins_and_refuse_overwrite(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "controls"
            manifest = CONTROLS.generate(directory)
            self.assertEqual(manifest["vendor_passes"], 0)
            self.assertEqual(len(manifest["files"]), 4)
            for record in manifest["files"]:
                payload = (directory / record["path"]).read_bytes()
                self.assertEqual(len(payload), record["size_bytes"])
                self.assertEqual(hashlib.sha256(payload).hexdigest(), record["sha256"])
                self.assertLess(len(payload), 32 * 1024)
            self.assertEqual(json.loads((directory / "controls.json").read_text()), manifest)
            with self.assertRaises(FileExistsError):
                CONTROLS.generate(directory)

    def test_different_unicode_has_identical_visible_operators(self):
        first = CONTROLS.pdf()
        second = CONTROLS.pdf(alternate=True)
        self.assertNotEqual(first, second)
        self.assertEqual(first.count(b"<000052>"), 0)
        self.assertEqual(second.replace(b"<E000>", b"<0052>"), first)
        self.assertIn(b"/Rotate 90", first)
        self.assertIn(b"/MediaBox [0 0 2052.0 1538.0]", first)
        self.assertIn(b"/Count 4", first)
        image = CONTROLS.pdf(image_only=True)
        self.assertNotIn(b"/Font", image)
        self.assertNotIn(b"/ToUnicode", image)
        self.assertNotIn(b" Tj", image)

    def test_asymmetric_color_extents_are_complete(self):
        pixels = CONTROLS.raster()
        width, height = CONTROLS.RASTER_WIDTH, CONTROLS.RASTER_HEIGHT
        self.assertEqual(len(pixels), width * height * 3)
        def pixel(x, y):
            begin = (y * width + x) * 3
            return tuple(pixels[begin:begin + 3])
        self.assertEqual(pixel(width // 2, 0), (255, 0, 0))
        self.assertEqual(pixel(width // 2, height - 1), (0, 0, 255))
        self.assertEqual(pixel(0, height // 2), (0, 255, 0))
        self.assertEqual(pixel(width - 1, height // 2), (255, 0, 255))
        self.assertEqual(pixel(width // 2, height // 2), (255, 255, 255))

    def test_independent_pdf_structure_text_and_pixel_origin_control(self):
        tools = {name: shutil.which(name) for name in ("qpdf", "pdftotext", "mutool", "pdfinfo")}
        if not all(tools.values()):
            # These validators are a required native CI setup step.
            self.fail("original canary control validators are required: qpdf/pdftotext/mutool/pdfinfo")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "controls"
            CONTROLS.generate(directory)
            for name in ("digital.pdf", "alternate-unicode.pdf", "image-only.pdf", "second-text.pdf"):
                result = subprocess.run([tools["qpdf"], "--check", str(directory / name)],
                                        capture_output=True, timeout=15)
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
            def text(name):
                return subprocess.run([tools["pdftotext"], "-layout", str(directory / name), "-"],
                                      capture_output=True, check=True, timeout=15).stdout.decode("utf-8")
            digital = text("digital.pdf")
            alternate = text("alternate-unicode.pdf")
            self.assertIn("RUST 0123456789 中口一", digital)
            self.assertIn("\ue000UST 0123456789 中口一", alternate)
            self.assertEqual(text("image-only.pdf").strip(), "")
            self.assertIn("RUST 9876543210 中口一", text("second-text.pdf"))
            info = subprocess.run([tools["pdfinfo"], str(directory / "digital.pdf")],
                                  capture_output=True, check=True, timeout=15).stdout.decode()
            self.assertIn("Pages:           4", info)
            self.assertIn("256.5 x 192.25", info)
            hashes = []
            for name in ("digital.pdf", "alternate-unicode.pdf"):
                raster = directory / (name + ".ppm")
                subprocess.run([tools["mutool"], "draw", "-q", "-r", "144", "-c", "rgb",
                                "-F", "ppm", "-o", str(raster), str(directory / name), "1"],
                               capture_output=True, check=True, timeout=15)
                hashes.append(hashlib.sha256(raster.read_bytes()).hexdigest())
            self.assertEqual(hashes[0], hashes[1])


if __name__ == "__main__":
    unittest.main()
