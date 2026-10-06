# SPDX-License-Identifier: MIT
"""Original synthetic full-array, pixel, process and audit regression tests."""

from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import random
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_page_composition as subject


def fixture(width=4, height=2):
    """Invented compact six-row metadata; no source or converter fixture."""
    pages, source, outputs, lines = [], [], [], []
    point_width, point_height = width*72/300, height*72/300
    ctm = [point_width, 0.0, 0.0, -point_height, 0.0, point_height]
    digest = hashlib.sha256(b"original synthetic encoded identity").hexdigest()
    for number in range(1, 7):
        output = 1 if number == 1 else 2 if number == 6 else None
        image = {"image_number": 1, "record_type": 2, "descriptor_offset": 1000+number*20,
                 "payload_offset": 1500+number*20, "payload_length": 12, "width": width, "height": height,
                 "payload_sha256": digest}
        images = [image] if output else []
        source.append({"page_number": number, "text_offset": 500+number*16,
                       "text_length": 8, "images": deepcopy(images)})
        pages.append({"source_page": number, "row_offset": 100+(number-1)*20,
                      "text_offset": 500+number*16, "text_length": 8,
                      "image_count": len(images), "output_page": output,
                      "media_box": [0.0, 0.0, point_width, point_height] if output else None,
                      "images": [{**image, "page_number": number, "display_width": width, "pdf_ctm": ctm}] if output else []})
        lines.append("\t".join(map(str, ["P", number, 100+(number-1)*20, 500+number*16,
                                         8, len(images), output or 0, 0, 0,
                                         point_width if output else 0, point_height if output else 0])))
        if output:
            lines.append("\t".join(map(str, ["I", number, 1, 2, image["descriptor_offset"],
                                             image["payload_offset"], 12, width, width, height, *ctm])))
            draw = {"draw_number": 1, "width": width, "height": height, "bits_per_component": 8,
                    "pdf_ctm": ctm, "filter": "/DCTDecode", "color_space": "DeviceGray",
                    "raw_stream_sha256": digest, "raw_stream_length": 12, "object_id": 20+output}
            outputs.append({"page_number": output, "media_box": [0.0, 0.0, point_width, point_height], "draws": [draw]})
    # Twelve numeric resource fields after the variant.
    values = [6, 2, 4, 0, 2, 100, 32, 1, 1, 0, 0, 0]
    lines.append("\t".join(map(str, ["R", "HN-B", *values])))
    case = {"source_variant": "HN-B", "source_id": "invented", "source_pages": source,
            "output_page_to_source_page": [1, 6], "pdf_pages": outputs, "pdf_sha256": digest}
    return {"variant": "HN-B", "pages": pages}, {"pages": outputs}, case, ("\n".join(lines)+"\n").encode()


def original_binary_pdf(path, bits):
    """Original 32x3 top-row-first indexed image, including five padded columns."""
    encoded = zlib.compress(bits)
    content = b"q\n32 0 0 3 0 0 cm\n/Im0 Do\nQ\n"
    image = (b"/Type /XObject /Subtype /Image /Width 32 /Height 3 /BitsPerComponent 1 "
             b"/ColorSpace [/Indexed /DeviceRGB 1 <ffffff000000>] /Filter /FlateDecode "
             b"/DecodeParms << /BitsPerComponent 1 /Colors 1 /Columns 32 /Predictor 1 >>")
    return original_image_pdf(path, image, encoded, content, 32, 3)


def original_image_pdf(path, image, encoded, content, width, height):
    """Build one small original runtime image wrapper with an explicit xref."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>",
               b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".encode(),
               b"<< /Length " + str(len(content)).encode() + b" >>\nstream\n" + content + b"endstream",
               b"<< " + image + b" /Length " + str(len(encoded)).encode() + b" >>\nstream\n" + encoded + b"\nendstream"]
    data = bytearray(b"%PDF-1.4\n")
    offsets = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(data)
    data.extend(b"xref\n0 6\n0000000000 65535 f \n")
    for offset in offsets:
        data.extend(f"{offset:010} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    path.write_bytes(data)
    return path


class SyntheticComposition(unittest.TestCase):
    def setUp(self):
        cache = Path.home() / ".cache" / "caj2pdf-issue117-synthetic"
        cache.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def ppm(self, name, width, height, pixels):
        path = self.root / name
        path.write_bytes(f"P6\n{width} {height}\n255\n".encode()+pixels)
        return path

    def commands(self):
        session = self.root / "session"
        session.mkdir()
        (session / "scratch").mkdir()
        report = subject._report()
        return subject.Commands(session, report), report

    def test_no_input_reads_or_launches_nothing(self):
        with patch.object(subject, "file_identity", side_effect=AssertionError("unexpected read")), \
                patch.object(subject.subprocess, "Popen", side_effect=AssertionError("unexpected launch")):
            result = subject.run()
        self.assertEqual(result["status"], "NOT_RUN")
        self.assertTrue(all(value == 0 for value in result["counts"].values()))
        self.assertEqual(result["attempts"], [])
        self.assertEqual(result["profiles"], [])
        self.assertEqual(subject.run({})["status"], "FAIL")

    def test_pinned_json_parses_the_single_bounded_hashed_read(self):
        path = self.root/"public-metadata.json"
        data = b'{"original":"MIT synthetic metadata"}'
        path.write_bytes(data)
        with patch.object(Path, "read_bytes", side_effect=AssertionError("second unbounded read")):
            value = subject._pinned_json(path, hashlib.sha256(data).hexdigest())
        self.assertEqual(value["original"], "MIT synthetic metadata")
        with self.assertRaises(subject.CompositionError):
            subject._pinned_json(path, "0"*64)

    def test_stable_wrong_baseline_cannot_be_frozen_as_an_audit_pin(self):
        baselines = {f"{profile}-run{repeat}": self.root/f"{profile}-{repeat}"
                     for profile in subject.BASELINE_PINS for repeat in (1, 2)}
        with patch.object(subject, "file_identity", return_value={"sha256": "0"*64, "size_bytes": 1}), \
                self.assertRaisesRegex(subject.CompositionError, "frozen hash/size"):
            subject._audit({}, baselines, [], {}, {}, "0"*64, "0"*64)

    def test_preserved_metadata_and_original_native_pins_are_required(self):
        original = {"status": "FAIL", "native_audit": {"status": "PASS", "after_status": "PASS",
                    "binary": {"sha256": "0"*64}, "source": {"sha256": "1"*64}}}
        with patch.object(subject, "_pinned_json", return_value=original):
            subject._require_original_native("0"*64, "1"*64)
            with self.assertRaises(subject.CompositionError):
                subject._require_original_native("2"*64, "1"*64)
            with self.assertRaises(subject.CompositionError):
                subject._require_original_native("0"*64, "2"*64)
        with patch.object(subject, "file_identity", return_value={"sha256": "0"*64, "size_bytes": 1}), \
                self.assertRaises(subject.CompositionError):
            subject._preserved_inputs()

    def test_native_rows_mapping_and_summary(self):
        native, _, _, data = fixture()
        parsed = subject.parse_native(data, 4096)
        expected = deepcopy(native["pages"])
        for page in expected:
            for image in page["images"]:
                image.pop("payload_sha256")
        self.assertEqual(parsed["pages"], expected)
        self.assertEqual(parsed["resources"]["no_image_pages"], 4)
        for changed in (data[:-2], data+b"P\t1\n", data.replace(b"R\tHN-B\t6", b"R\tHN-B\t5"),
                        data.replace(b"P\t2\t120", b"P\t2\t121"),
                        data.replace(b"I\t1\t1\t2", b"I\t1\t2\t2"),
                        data.replace(b"I\t1\t1\t2", b"I\t1\t1\t3"),
                        data.replace(b"0.96", b"nan", 1), b"\xff", b"x"*2049):
            with self.subTest(data=changed[:20]), self.assertRaises(subject.CompositionError):
                subject.parse_native(changed, 4096)

    def test_integer_and_numeric_boundaries(self):
        for value in (True, "-1", "+1", "01", "1.0", "", 5):
            with self.subTest(value=value), self.assertRaises(subject.CompositionError):
                subject._integer(value, "synthetic", maximum=4)
        self.assertEqual(subject._integer("0", "synthetic"), 0)
        for text in ("NaN", "inf", "2147483648"):
            with self.assertRaises(subject.CompositionError):
                subject._numbers([text], "synthetic")

    def test_metadata_full_success_and_exact_hnb_mapping(self):
        native, document, case, _ = fixture()
        counts = subject._report()["counts"]
        progress = []
        result = subject.compare_metadata(native, document, deepcopy(document), case, counts, progress)
        self.assertEqual(result["output_to_source"], [1, 6])
        self.assertEqual(result["no_image_source_rows"], [2, 3, 4, 5])
        self.assertEqual([counts[f"{kind}_passing"] for kind in ("source_rows", "output_pages", "draws", "jpeg_streams", "jpeg_color_spaces")], [6, 2, 2, 2, 2])
        self.assertTrue(all(row["status"] == "PASS" for row in progress))

    def test_jpeg_color_space_mismatch_is_a_located_failure_after_stream_identity(self):
        for changed in ("reference", "native", "pinned"):
            with self.subTest(changed=changed):
                native, reference, case, _ = fixture()
                case = deepcopy(case)
                produced = deepcopy(reference)
                document = {"reference": reference, "native": produced,
                            "pinned": {"pages": case["pdf_pages"]}}[changed]
                document["pages"][0]["draws"][0]["color_space"] = "DeviceRGB"
                counts, progress = subject._report()["counts"], []
                with self.assertRaisesRegex(subject.CompositionError, "ColorSpace"):
                    subject.compare_metadata(native, reference, produced, case, counts, progress)
                self.assertEqual(counts["jpeg_streams_passing"], 1)
                self.assertEqual(counts["jpeg_color_spaces_attempted"], 1)
                self.assertEqual(counts["jpeg_color_spaces_failing"], 1)
                self.assertEqual(counts["jpeg_color_spaces_passing"], 0)
                self.assertEqual(counts["jpeg_color_spaces_unsupported"], 0)
                self.assertEqual(progress[-1], {"kind": "jpeg_color_spaces", "status": "FAIL", "unsupported": False,
                                               "page": 1, "image": 1})

    def test_jpeg_unknown_color_space_is_unsupported_and_never_a_pass(self):
        for color in (None, "Indexed", "DeviceCMYK", [], True):
            with self.subTest(color=color):
                native, reference, case, _ = fixture()
                produced = deepcopy(reference)
                produced["pages"][0]["draws"][0]["color_space"] = color
                counts, progress = subject._report()["counts"], []
                with self.assertRaises(subject.CompositionUnsupported):
                    subject.compare_metadata(native, reference, produced, case, counts, progress)
                self.assertEqual(counts["jpeg_color_spaces_attempted"], 1)
                self.assertEqual(counts["jpeg_color_spaces_failing"], 1)
                self.assertEqual(counts["jpeg_color_spaces_unsupported"], 1)
                self.assertEqual(counts["jpeg_color_spaces_passing"], 0)
                self.assertEqual(progress[-1]["status"], "FAIL")
                self.assertTrue(progress[-1]["unsupported"])

    def test_consistent_rgb_metadata_remains_supported(self):
        native, reference, case, _ = fixture()
        for page in reference["pages"]:
            page["draws"][0]["color_space"] = "DeviceRGB"
        case["pdf_pages"] = deepcopy(reference["pages"])
        counts = subject._report()["counts"]
        result = subject.compare_metadata(native, reference, deepcopy(reference), case, counts)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(counts["jpeg_color_spaces_passing"], 2)

    def test_original_grayscale_jpeg_bytes_do_not_establish_rgb_page_parity(self):
        commands, report = self.commands()
        tools = {name: shutil.which(name) for name in ("cjpeg", "qpdf", "mutool", "pdftoppm")}
        self.assertTrue(all(tools.values()), "original JPEG/PDF controls require all CI tools")
        pgm = self.root/"original-gray.pgm"
        pgm.write_bytes(b"P5\n8 4\n255\n"+bytes((index*73+index//8*19)%256 for index in range(32)))
        limits = subject.pdf.PdfMetadataLimits(timeout_seconds=10)
        encoded, _ = commands.run([tools["cjpeg"], "-quality", "90", "-grayscale", str(pgm)],
                                  "original grayscale JPEG", limits, subject.pdf._Usage(), 65536)
        content = b"q\n8 0 0 -4 0 4 cm\n/Im0 Do\nQ\n"
        wrappers = {}
        for color in ("DeviceGray", "DeviceRGB"):
            image = (f"/Type /XObject /Subtype /Image /Width 8 /Height 4 /BitsPerComponent 8 "
                     f"/ColorSpace /{color} /Filter /DCTDecode").encode()
            path = original_image_pdf(self.root/f"{color}.pdf", image, encoded, content, 8, 4)
            raw, _ = commands.run([tools["qpdf"], "--show-object=5", "--raw-stream-data", str(path)],
                                  "original unchanged DCT stream", limits, subject.pdf._Usage(), 65536)
            self.assertEqual(raw, encoded)
            metadata, _ = commands.run([tools["qpdf"], "--json", "--json-key=qpdf", "--json-stream-data=none",
                                       "--json-object=5", str(path)], "original image declaration",
                                      limits, subject.pdf._Usage(), 65536)
            dictionary = json.loads(metadata)["qpdf"][1]["obj:5 0 R"]["stream"]["dict"]
            self.assertEqual(dictionary["/ColorSpace"], "/"+color)
            wrappers[color] = path
        commands.kind = "render"
        for renderer in ("mutool", "pdftoppm"):
            with self.subTest(renderer=renderer):
                rasters = []
                for color, path in wrappers.items():
                    output = commands.session/f"{renderer}-{color}.ppm"
                    if renderer == "mutool":
                        arguments = [tools[renderer], "draw", "-q", "-r", "300", "-A", "0",
                                     "-c", "rgb", "-F", "pnm", "-o", "-", str(path), "1"]
                    else:
                        arguments = [tools[renderer], "-r", "300", "-singlefile", "-aa", "no", "-aaVector", "no",
                                     "-f", "1", "-l", "1", str(path)]
                    subject._to_file(commands, arguments, "original whole-page color control", output, 65536, timeout=10)
                    rasters.append(output)
                comparison = subject.compare_pixels(*rasters, 34, 17)
                self.assertEqual(comparison["status"], "FAIL")
                self.assertGreater(comparison["changed_pixels"], 0)
                self.assertGreater(comparison["baseline_nonwhite_pixels"], 0)
        self.assertEqual(report["counts"]["native_launches"], 0)
        self.assertEqual(report["counts"]["converter_launches"], 0)
        self.assertEqual(report["counts"]["render_launches"], 4)

    def test_metadata_failure_is_granular_and_missing_blank_draw_fails(self):
        for mutation, failing, expected_draw_passes in (("draw", "output_pages", 0),
                                                       ("matrix", "draws", 0),
                                                       ("jpeg", "jpeg_streams", 1)):
            with self.subTest(mutation=mutation):
                native, baseline, case, _ = fixture()
                produced = deepcopy(baseline)
                if mutation == "draw":
                    produced["pages"][0]["draws"] = []
                elif mutation == "matrix":
                    produced["pages"][0]["draws"][0]["pdf_ctm"][4] += .001
                else:
                    produced["pages"][0]["draws"][0]["raw_stream_sha256"] = "0"*64
                counts, progress = subject._report()["counts"], []
                with self.assertRaises(subject.CompositionError):
                    subject.compare_metadata(native, baseline, produced, case, counts, progress)
                self.assertEqual(counts["source_rows_attempted"], 1)
                self.assertEqual(counts["source_rows_passing"], 1)
                self.assertEqual(counts[f"{failing}_failing"], 1)
                self.assertEqual(counts["draws_passing"], expected_draw_passes)
                self.assertEqual(progress[-1]["status"], "FAIL")

    def test_complete_pixel_arrays_include_edges_and_final_partial_chunk(self):
        count = 21847
        pixels = b"\xff"*3*(count-2)+b"\xff\x00\xff\x00\xff\xff"
        baseline = self.ppm("baseline.ppm", count, 1, pixels)
        same = self.ppm("same.ppm", count, 1, pixels)
        result = subject.compare_pixels(baseline, same, count, 1)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["compared_channels"], count*3)
        self.assertEqual(result["baseline_nonwhite_pixels"], 2)
        for offset in (0, len(pixels)-1):
            changed = bytearray(pixels)
            changed[offset] ^= 255
            candidate = self.ppm(f"changed-{offset}.ppm", count, 1, changed)
            diff = subject.compare_pixels(baseline, candidate, count, 1)
            self.assertEqual(diff["status"], "FAIL")
            self.assertEqual(diff["changed_pixels"], 1)
            self.assertEqual(diff["changed_channels"], 1)
            self.assertEqual(diff["absolute_difference_sum"], 255)

    def test_complete_page_canvas_boundaries_never_crop_or_relax_pixels(self):
        box = [0, 0, 1.2, 1.68]  # An original five-by-seven nominal grid.
        for width, height in ((5, 7), (6, 7), (5, 8), (6, 8)):
            with self.subTest(grid=(width, height)):
                body = bytes((i*29) % 256 for i in range(width*height*3))
                first = self.ppm("canvas-first.ppm", width, height, body)
                same = self.ppm("canvas-same.ppm", width, height, body)
                result = subject.compare_page_pixels(first, same, box)
                self.assertEqual(result["status"], "PASS")
                self.assertEqual(result["nominal_grid"], [5, 7])
                self.assertEqual(result["compared_channels"], width*height*3)
                changed = self.ppm("canvas-edge.ppm", width, height, body[:-1]+bytes([body[-1]^255]))
                self.assertEqual(subject.compare_page_pixels(first, changed, box)["changed_channels"], 1)
                wrong = self.ppm("canvas-mismatch.ppm", width+1, height, b"\x00"*((width+1)*height*3))
                with self.assertRaises(subject.CompositionError):
                    subject.compare_page_pixels(first, wrong, box)
        for width, height in ((4, 7), (7, 7), (5, 6), (5, 9)):
            first = self.ppm("canvas-outside.ppm", width, height, b"\x00"*(width*height*3))
            with self.subTest(grid=(width, height)), self.assertRaises(subject.CompositionError):
                subject.compare_page_pixels(first, first, box)
        first = self.ppm("canvas-trailing.ppm", 5, 7, b"\x00"*106)
        with self.assertRaises(subject.CompositionError):
            subject.compare_page_pixels(first, first, box)
        first = self.ppm("canvas-short.ppm", 5, 7, b"\x00"*104)
        with self.assertRaises(subject.CompositionError):
            subject.compare_page_pixels(first, first, box)
        for geometry in (None, [0,0,1], [1,0,1.2,1.68], [0,0,.96,1.68], [0,0,1.3,1.68],
                         [0,0,8000,1.68], [0,0,float("nan"),1.68], [0,0,1<<2000,1.68]):
            with self.subTest(geometry=str(geometry)[:80]), self.assertRaises(subject.CompositionUnsupported):
                subject.compare_page_pixels(first, first, geometry)

    def test_original_decimal_ctm_boundary_still_fails_complete_poppler_pixels(self):
        commands, report = self.commands()
        tools = {name: shutil.which(name) for name in ("mutool", "pdftoppm")}
        self.assertTrue(all(tools.values()), "original numeric boundary controls require both renderers")
        image = b"/Type /XObject /Subtype /Image /Width 3 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray"
        samples = bytes([0, 70, 255, 240, 130, 10])
        wrappers = []
        for name, width in (("ratio", 2071*72/300), ("binary-factor", 2071*.24)):
            content = f"q\n{width!r} 0 0 -36.72 0 36.72 cm\n/Im0 Do\nQ\n".encode()
            wrappers.append(original_image_pdf(self.root/f"{name}.pdf", image, samples, content, 497.04, 36.72))
        self.assertNotEqual(2071*72/300, 2071*.24)
        commands.kind = "render"
        for renderer, expected_grid in (("mutool", (2071,153)), ("pdftoppm", (2071,154))):
            paths = []
            for index, pdf in enumerate(wrappers):
                output = commands.session/f"precision-{renderer}-{index}.ppm"
                if renderer == "mutool":
                    argv = [tools[renderer], "draw", "-q", "-r", "300", "-A", "0", "-c", "rgb", "-F", "pnm", "-o", "-", str(pdf), "1"]
                else:
                    argv = [tools[renderer], "-r", "300", "-singlefile", "-aa", "no", "-aaVector", "no", "-f", "1", "-l", "1", str(pdf)]
                subject._to_file(commands, argv, "original numeric boundary full page", output, subject.RASTER_LIMIT, timeout=10)
                paths.append(output)
            result = subject.compare_page_pixels(*paths, [0,0,497.04,36.72])
            self.assertEqual((result["width"], result["height"]), expected_grid)
            self.assertGreater(result["baseline_nonwhite_pixels"], 0)
            self.assertEqual(result["status"], "PASS" if renderer == "mutool" else "FAIL")
            if renderer == "pdftoppm":
                self.assertGreater(result["changed_channels"], 0)
        self.assertEqual(report["counts"]["render_launches"], 4)
        self.assertEqual(report["counts"]["native_launches"], 0)
        self.assertEqual(report["counts"]["converter_launches"], 0)

    def test_corrected_reference_is_separately_pinned_and_changes_only_two_colors(self):
        _, _, case, _ = fixture()
        for page in case["pdf_pages"]:
            page["draws"][0]["color_space"] = "DeviceRGB"
        legacy = deepcopy(case)
        corrected = subject._corrected_hnb_case(case)
        for page in corrected["pdf_pages"]:
            self.assertEqual(page["draws"][0]["color_space"], "DeviceGray")
            page["draws"][0]["color_space"] = "DeviceRGB"
        self.assertEqual(corrected, legacy)
        self.assertEqual(case, legacy)
        for mutation in ("variant", "mapping", "pages", "draws", "color"):
            bad = deepcopy(case)
            if mutation == "variant": bad["source_variant"] = "C8"
            elif mutation == "mapping": bad["output_page_to_source_page"] = [1, 2]
            elif mutation == "pages": bad["pdf_pages"].pop()
            elif mutation == "draws": bad["pdf_pages"][0]["draws"].append(deepcopy(bad["pdf_pages"][0]["draws"][0]))
            else: bad["pdf_pages"][0]["draws"][0]["color_space"] = "DeviceGray"
            with self.subTest(mutation=mutation), self.assertRaises(subject.CompositionError):
                subject._corrected_hnb_case(bad)
        path = self.root/"original-corrected-reference.pdf"
        path.write_bytes(b"original invented opaque PDF identity; no private bytes")
        identity = subject.file_identity(path, subject.PDF_LIMIT)
        with patch.object(subject, "CORRECTED_HNB_PIN", (identity["size_bytes"], identity["sha256"])):
            self.assertEqual(subject._corrected_hnb_identity(path), identity)
            receipt = self.root/"original-corrected-receipt.json"
            receipt.write_text(json.dumps({"harness_files": {}, "corrected_hn_b_reference": identity}))
            receipt_identity = subject.file_identity(receipt, subject.MIB)
            subject._verify_receipt(receipt_identity)
            path.write_bytes(b"changed")
            with self.assertRaises(subject.CompositionError):
                subject._corrected_hnb_identity(path)
            with self.assertRaises(subject.CompositionError):
                subject._verify_receipt(receipt_identity)

    def test_native_revision_requires_declared_pair_and_preserved_provenance(self):
        original = ("2"*64, "3"*64)
        report = {"status": "FAIL", "native_audit": {"status": "PASS", "after_status": "PASS",
                  "binary": {"sha256": original[0]}, "source": {"sha256": original[1]}}}
        with patch.object(subject, "_pinned_json", return_value=report):
            subject._require_original_native(*original)
            subject._require_original_native(*subject.RATIONAL_NATIVE_PIN)
            for pair in ((original[0],subject.RATIONAL_NATIVE_PIN[1]), (subject.RATIONAL_NATIVE_PIN[0],original[1]), ("0"*64,"0"*64)):
                with self.subTest(pair=pair), self.assertRaises(subject.CompositionError):
                    subject._require_original_native(*pair)
            report["native_audit"]["after_status"] = "FAIL"
            with self.assertRaises(subject.CompositionError):
                subject._require_original_native(*subject.RATIONAL_NATIVE_PIN)

    def test_exact_nonwhite_is_sample_aligned(self):
        self.assertEqual(subject._nonwhite_count(b"\xff"*30), 0)
        self.assertEqual(subject._nonwhite_count(b"\x00"*30), 10)
        generator = random.Random(117)
        for size in (1, 2, 3, 10, 100, 21845):
            data = bytes(generator.choice((0, 1, 255)) for _ in range(size*3))
            expected = sum(data[i:i+3] != b"\xff"*3 for i in range(0, len(data), 3))
            self.assertEqual(subject._nonwhite_count(data), expected)
        with self.assertRaises(subject.CompositionError):
            subject._nonwhite_count(b"\xff")

    def test_pixels_reverse_shift_missing_color_and_white_output_fail(self):
        body = bytes([255,0,0, 0,0,255, 255,255,255, 0,255,0, 0,0,0, 255,255,0])
        baseline = self.ppm("positive.ppm", 3, 2, body)
        alternatives = (body[9:]+body[:9], b"\xff"*3+body[:-3], b"\xff"*len(body), body[:-3]+b"\xff"*3)
        for number, pixels in enumerate(alternatives):
            candidate = self.ppm(f"bad-{number}.ppm", 3, 2, pixels)
            self.assertEqual(subject.compare_pixels(baseline, candidate, 3, 2)["status"], "FAIL")

    def test_strict_pnm_rejects_bad_headers_lengths_and_profiles(self):
        for body in (b"P6\n1 1\n254\nabc", b"P6\n0 1\n255\n", b"P6\n1 1\n255\nab",
                     b"P6\n1 1\n255\nabcd", b"P5\n1 1\n255\na", b"P9\n1 1\n255\na",
                     b"P6\n"+b"x"*4097):
            path = self.root / "malformed"
            path.write_bytes(body)
            with self.subTest(body=body[:20]), self.assertRaises(subject.CompositionError):
                subject._raster(path, 1, 1, rgb=True)
        info = subject.pnm_header(io.BytesIO(b"P6\n# invented comment\n1 1\n255\nabc"), rgb=True)
        self.assertEqual((info.width, info.height), (1, 1))

    def test_binary_padding_and_all_rows_match_independent_wrappers(self):
        bits = bytes([0x80,0,0,1, 0,0x80,0,0, 0,0,0,0x80])
        pbm = self.root / "padded.pbm"
        pbm.write_bytes(b"P4\n32 3\n"+bits)
        rgb = bytearray()
        for byte in bits:
            for bit in range(8):
                rgb.extend(b"\x00"*3 if byte & (0x80 >> bit) else b"\xff"*3)
        ppm = self.ppm("padded.ppm", 32, 3, rgb)
        first, second = self.root/"first.bits", self.root/"second.bits"
        subject.canonical_binary(pbm, first, 32, 3)
        subject.canonical_binary(ppm, second, 32, 3)
        self.assertEqual(subject.compare_samples(first, second, 32, 3)["status"], "PASS")
        changed = bytearray(bits)
        changed[3] ^= 1  # A padded column, never cropped, zero-filled or ignored.
        second.write_bytes(changed)
        result = subject.compare_samples(first, second, 32, 3)
        self.assertEqual((result["changed_bits"], result["changed_bytes"]), (1, 1))
        second.write_bytes(bits[8:]+bits[4:8]+bits[:4])
        self.assertTrue(subject.compare_samples(first, second, 32, 3)["reverse_only_match"])
        ppm.write_bytes(b"P6\n32 3\n255\n"+b"\x7f"+rgb[1:])
        with self.assertRaises(subject.CompositionError):
            subject.canonical_binary(ppm, self.root/"invalid.bits", 32, 3)

    def test_binary_dictionary_polarity_and_rejections(self):
        head = b"<< /Subtype /Image /Width 32 /Height 3 /BitsPerComponent 1 "
        gray = head+b"/ColorSpace /DeviceGray /Decode [1 0] >>"
        indexed = head+b"/Filter /FlateDecode /ColorSpace [ /Indexed /DeviceRGB 1 <ffffff000000> ] >>"
        self.assertFalse(subject._sample_dictionary(gray, 32, 3))
        self.assertFalse(subject._sample_dictionary(indexed, 32, 3))
        self.assertTrue(subject._sample_dictionary(indexed.replace(b"ffffff000000", b"000000ffffff"), 32, 3))
        for bad in (gray.replace(b"[1 0]", b"[0 1]"), gray+b"/Mask 1 0 R", indexed.replace(b"ffffff000000", b"ff0000000000"),
                    gray.replace(b"/Width 32", b"/Width 31"), indexed.replace(b"/FlateDecode", b"/DCTDecode"),
                    gray[:-2]+b" /Filter 4 0 R >>"):
            with self.assertRaises(subject.CompositionError):
                subject._sample_dictionary(bad, 32, 3)

    def test_explicit_no_prediction_parameters_and_strict_refusals(self):
        head = (b"<< /Subtype /Image /Width 32 /Height 3 /BitsPerComponent 1 "
                b"/Filter /FlateDecode /ColorSpace [/Indexed /DeviceRGB 1 <ffffff000000>] ")
        parameters = b"/DecodeParms << /BitsPerComponent 1 /Colors 1 /Columns 32 /Predictor 1 >>"
        self.assertFalse(subject._sample_dictionary(head+parameters+b" >>", 32, 3))
        self.assertFalse(subject._sample_dictionary(head+parameters+b" /Decode [0 1] >>", 32, 3))
        alternatives = (parameters.replace(b"/Predictor 1", b"/Predictor 2"),
                        parameters.replace(b"/Predictor 1", b"/Predictor 12"),
                        parameters.replace(b"/Columns 32", b"/Columns 31"),
                        parameters.replace(b"/Colors 1", b"/Colors 2"),
                        parameters.replace(b"/BitsPerComponent 1", b"/BitsPerComponent 8"),
                        parameters.replace(b"/Predictor 1", b"/Predictor 1 /Predictor 1"),
                        parameters.replace(b"/Predictor 1", b"/Predictor 1 /Unknown 0"),
                        parameters.replace(b"/Predictor 1", b"/Predictor << /Nested 1 >>"),
                        parameters.replace(b"/Columns 32", b"/Columns 4 0 R"),
                        parameters.replace(b"/Columns 32", b"/Columns "+b"9"*10000),
                        parameters+b" "+parameters, b"/DecodeParms 4 0 R", b"/DecodeParms []", b"/DecodeParms << >>",
                        parameters+b" /Mask [0 1]", parameters+b" /ImageMask false", parameters+b" /SMask 4 0 R")
        for altered in alternatives:
            with self.subTest(parameters=altered[:80]), self.assertRaises(subject.CompositionUnsupported):
                subject._sample_dictionary(head+altered+b" >>", 32, 3)
        with self.assertRaises(subject.CompositionUnsupported):
            subject._sample_dictionary((head+parameters+b" >>").replace(b"/FlateDecode", b"/DCTDecode"), 32, 3)

    def test_actual_qpdf_and_poppler_full_original_no_prediction_bits(self):
        tools = {}
        for name in ("qpdf", "pdfimages"):
            executable = shutil.which(name)
            self.assertIsNotNone(executable, f"mandatory original-fixture validator is missing: {name}")
            tools[name] = Path(executable)
        bits = bytes([0x80,0,0,1, 0,0x80,0,0x10, 0,0,0,0x84])
        document = original_binary_pdf(self.root/"original-padded.pdf", bits)
        commands, report = self.commands()
        draw = {"bits_per_component": 1, "width": 32, "height": 3, "draw_number": 1, "object_id": 5}
        qpdf_bits = commands.session/"qpdf.bits"
        subject._qpdf_samples(commands, document, draw, qpdf_bits, tools)
        extracted = subject._poppler_samples(commands, document, 1, [draw], commands.session/"poppler", tools)
        self.assertEqual(qpdf_bits.read_bytes(), bits)
        self.assertEqual(extracted[1].read_bytes(), bits)
        full = subject.compare_samples(qpdf_bits, extracted[1], 32, 3)
        self.assertEqual((full["status"], full["compared_bytes"]), ("PASS", 12))
        self.assertEqual(report["counts"]["validator_launches"], 3)
        self.assertTrue(all(attempt["status"] == "PASS" for attempt in report["attempts"]))
        altered = commands.session/"one-padded-bit.bits"
        changed = bytearray(bits)
        changed[3] ^= 1
        altered.write_bytes(changed)
        self.assertEqual(subject.compare_samples(qpdf_bits, altered, 32, 3)["changed_bits"], 1)
        altered.write_bytes(bits[8:]+bits[4:8]+bits[:4])
        self.assertTrue(subject.compare_samples(qpdf_bits, altered, 32, 3)["reverse_only_match"])

    def test_unsupported_required_comparison_is_failed_and_never_passing(self):
        for error in (subject.CompositionUnsupported("invented dictionary profile"),
                      subject.pdf.PdfMetadataUnsupported("invented metadata profile"),
                      subject.CompositionError("invented ordinary error")):
            counts, progress = subject._report()["counts"], []
            with self.subTest(error=type(error).__name__), self.assertRaises(type(error)):
                with subject._comparison(counts, "type0_arrays", progress):
                    raise error
            self.assertEqual((counts["type0_arrays_attempted"], counts["type0_arrays_failing"], counts["type0_arrays_passing"]), (1, 1, 0))
            self.assertEqual(counts["type0_arrays_unsupported"], int(subject._unsupported(error)))
            self.assertEqual(progress[0]["status"], "FAIL")

    def test_process_success_digest_and_failed_launch_are_counted(self):
        commands, report = self.commands()
        usage = subject.pdf._Usage()
        data, size = commands.run([sys.executable, "-c", "import sys;print('abc');sys.stderr.write('warn')"],
                                  "synthetic", subject.pdf.PdfMetadataLimits(), usage, 100, include_stderr=True)
        self.assertEqual((data, size), (b"abc\nwarn", 4))
        self.assertGreater(report["attempts"][0]["peak_rss_kib"], 0)
        self.assertEqual(report["counts"]["validator_launches"], 1)
        digest, size = commands.run([sys.executable, "-c", "print('abc')"], "digest",
                                    subject.pdf.PdfMetadataLimits(), usage, 100, digest_only=True)
        self.assertEqual(digest, hashlib.sha256(b"abc\n").hexdigest())
        with self.assertRaises(FileNotFoundError):
            commands.run([str(self.root/"absent")], "failed launch", subject.pdf.PdfMetadataLimits(), usage, 100)
        self.assertEqual(report["counts"]["validator_launches"], 3)
        self.assertEqual(report["attempts"][-1]["status"], "FAIL")

    def test_process_limits_and_closed_pipe_lifetime_accounting(self):
        commands, report = self.commands()
        for code, limit, timeout in (("print('too long')", 1, 2),
                                     ("import os,time;os.close(1);os.close(2);time.sleep(1)", 100, .05)):
            with self.assertRaises(subject.CompositionError):
                commands.run([sys.executable, "-c", code], "limit", subject.pdf.PdfMetadataLimits(timeout_seconds=timeout),
                             subject.pdf._Usage(), limit)
            self.assertEqual(report["attempts"][-1]["status"], "FAIL")
            self.assertIsNotNone(report["attempts"][-1]["peak_rss_kib"])
        path = commands.session / "too-big"
        code = f"import os,time;os.close(1);os.close(2);time.sleep(.05);open({str(path)!r},'wb').write(bytes(20000));time.sleep(1)"
        with patch.object(subject, "DISK_LIMIT", 10000), self.assertRaises(subject.CompositionError):
            commands.run([sys.executable, "-c", code], "closed-pipe disk", subject.pdf.PdfMetadataLimits(timeout_seconds=2),
                         subject.pdf._Usage(), 100)
        self.assertEqual(report["attempts"][-1]["stdout_bytes"], 0)
        self.assertEqual(report["attempts"][-1]["stdout_sha256"], hashlib.sha256(b"").hexdigest())
        self.assertIsNotNone(report["attempts"][-1]["exit_code"])

    def test_extraction_file_and_directory_caps_are_live_until_child_exit(self):
        commands, report = self.commands()
        directory = commands.session/"extraction"
        directory.mkdir()
        output = directory/"large.bin"
        code = f"import os,time;os.close(1);os.close(2);time.sleep(.05);open({str(output)!r},'wb').write(bytes(20000));time.sleep(1)"
        with patch.object(subject, "RASTER_LIMIT", 10000), commands.extraction_caps(directory), \
                self.assertRaises(subject.CompositionError):
            commands.run([sys.executable, "-c", code], "live extraction file cap",
                         subject.pdf.PdfMetadataLimits(timeout_seconds=2), subject.pdf._Usage(), 100)
        self.assertEqual(report["attempts"][-1]["status"], "FAIL")
        self.assertLessEqual(output.stat().st_size, 10000)  # Child RLIMIT_FSIZE is a hard file cap.
        output.unlink()
        commands.directory_caps[directory] = (10000, 15000)
        first, second = directory/"first", directory/"second"
        first.write_bytes(b"x"*9000)
        second.write_bytes(b"x"*9000)
        with self.assertRaisesRegex(subject.CompositionError, "directory cap"):
            commands.disk()

    def test_shared_metadata_controller_restores_on_failure(self):
        commands, _ = self.commands()
        original = subject.pdf._run
        with self.assertRaisesRegex(RuntimeError, "invented"):
            with commands.metadata_controller():
                self.assertEqual(subject.pdf._run, commands.run)
                raise RuntimeError("invented")
        self.assertIs(subject.pdf._run, original)

    def test_qpdf_failure_marks_type0_attempt_not_skip(self):
        commands, report = self.commands()
        draw = {"bits_per_component": 1, "width": 32, "height": 3, "draw_number": 1, "object_id": 2}
        document = {"pages": [{"page_number": 1, "draws": [draw]}]}
        result = {"profile": "invented", "type0_arrays": []}
        with patch.object(subject, "_poppler_samples", side_effect=subject.CompositionError("invented extraction failure")):
            with self.assertRaises(subject.CompositionError):
                subject.check_arrays(commands, self.root/"reference", self.root/"native", document, document, {}, result)
        self.assertEqual((report["counts"]["type0_arrays_attempted"], report["counts"]["type0_arrays_failing"]), (1, 1))
        self.assertEqual(result["progress"][0]["status"], "FAIL")

    def test_render_failure_marks_pair_and_retains_first_failed_pair(self):
        for malformed in (True, False):
            with self.subTest(malformed=malformed):
                session = self.root / f"render-{malformed}"
                session.mkdir()
                (session/"scratch").mkdir()
                report = subject._report()
                commands = subject.Commands(session, report)
                result = {"profile": "invented", "page_pixels": []}
                def write_pair(_commands, _args, _label, path, _limit, **_kwargs):
                    path.write_bytes(b"P6\n5 5\n255\n"+(b"\xff"*74 if malformed else
                                      b"\x00"*75 if "candidate" in path.name else b"\xff"*75))
                    return subject.file_identity(path, subject.RASTER_LIMIT)
                with patch.object(subject, "_to_file", side_effect=write_pair), self.assertRaises(subject.CompositionError):
                    subject.check_pixels(commands, self.root/"ref", self.root/"candidate",
                                         [{"page_number": 1, "media_box": [0,0,1.2,1.2]}],
                                         {"mutool": Path("mutool")}, result)
                self.assertEqual((report["counts"]["page_renderer_pairs_attempted"], report["counts"]["page_renderer_pairs_failing"]), (1, 1))
                self.assertEqual(len(list(session.glob("*.ppm"))), 2)
                self.assertEqual(result["progress"][0]["status"], "FAIL")

    def test_end_to_end_immutable_pre_post_audits_and_exact_metadata_signature(self):
        self._end_to_end(False)

    def test_end_to_end_explicit_corrected_hnb_keeps_legacy_audits_and_policy(self):
        self._end_to_end(False, corrected_hn_b=True)

    def test_end_to_end_post_audit_mutation_fails(self):
        self._end_to_end(True)

    def test_end_to_end_first_pixel_failure_preserves_failure_and_post_audits(self):
        self._end_to_end(False, pixel_failure=True)

    def test_end_to_end_timeout_including_post_audits_fails(self):
        self._end_to_end(False, expired=True)

    def test_end_to_end_unsupported_metadata_remains_failure_with_post_audits(self):
        self._end_to_end(False, unsupported=True)

    def _end_to_end(self, mutate, *, pixel_failure=False, expired=False, unsupported=False, corrected_hn_b=False):
        """Exercise runner control flow using only invented files/observations.

        The compact oracle has three artificial profiles sharing the six-row
        fixture. No converter is called, and this is not compatibility data.
        """
        _, document, case, data = fixture(6, 6)
        corpus = self.root / "corpus"
        corpus.mkdir()
        artifacts = self.root / "artifacts"
        artifacts.mkdir()
        table = self.root / "invented-table"
        table.write_text("invented table stub; never parsed by this test")
        reference_report = self.root / "reference-report"
        reference_report.write_text("invented reference stub")
        native = self.root / "native"
        native.write_text("invented executable stub")
        corrected = self.root/"corrected-hnb.pdf"
        corrected.write_bytes(b"invented corrected reference identity")
        corrected_identity = subject.file_identity(corrected, subject.PDF_LIMIT)
        rows = []
        cases = []
        baselines = {}
        for name in ("hn_a", "c8", "hn_b"):
            (corpus/f"{name}.caj").write_bytes(b"synthetic source stub")
            rows.append({"id": name, "path": f"{name}.caj", "detected_type": "HN",
                         "size_bytes": 4096, "sha256": "0"*64})
            cases.append({**deepcopy(case), "case": name, "source_id": name})
            if corrected_hn_b and name == "hn_b":
                for page in cases[-1]["pdf_pages"]:
                    page["draws"][0]["color_space"] = "DeviceRGB"
            for repeat in (1, 2):
                path = self.root / f"{name}-run{repeat}.pdf"
                path.write_bytes(b"synthetic PDF stub")
                baselines[f"{name}-run{repeat}"] = path
        # Twenty-four unused rows only exercise the original audit-set guard.
        rows.extend({"id": f"unused-{i}", "detected_type": "HN"} for i in range(24))
        snapshots = []
        now = [100.0]
        def audit(*_args):
            result = {name: {"status": "PASS", "invented_pin": 1}
                      for name in ("source_audit", "baseline_audit", "table_audit", "environment_audit", "input_audit", "native_audit")}
            if mutate and snapshots:
                result["environment_audit"]["invented_pin"] = 2
            if expired and snapshots:
                now[0] = 2001.0
            if corrected_hn_b:
                result["input_audit"]["corrected_hn_b_reference"] = subject._corrected_hnb_identity(corrected)
            snapshots.append(deepcopy(result))
            return result
        def pinned(path, _sha):
            if path.name == "execution-receipt.json":
                return json.loads(path.read_text())
            if path.name == "matrix.json":
                return {"samples": rows}
            if path.name == "hnc8_layout_oracle.json":
                return {"cases": cases}
            return {"invented": True}
        def command(controller, arguments, label, _limits, _usage, _maximum, **kwargs):
            counts = controller.report["counts"]
            if controller.kind == "native":
                counts["native_launches"] += 1
                Path(arguments[2]).write_bytes(b"X")
                return data, len(data)
            counts["validator_launches"] += 1
            if controller.kind == "render":
                counts["render_launches"] += 1
                pixels = b"P6\n6 6\n255\n"+b"\x00\xff\x00"*36
                if pixel_failure and any(Path(argument).name == "hn_a.pdf" for argument in arguments):
                    pixels = pixels[:-1]+b"\xff"
                kwargs["consume"](pixels)
                return b"", len(pixels)
            text = (subject.VERSIONS[Path(arguments[0]).name]+"\n").encode()
            return text, len(text)
        calls = []
        receipts = []
        def receipt(_paths, _tools, _oracle, _rows, commands, _native_sha, _source_sha):
            path = commands.session/"execution-receipt.json"
            data = {"harness_files": {}, "synthetic": True}
            if corrected_hn_b:
                data["corrected_hn_b_reference"] = corrected_identity
            path.write_text(json.dumps(data))
            identity = subject.file_identity(path, subject.MIB)
            receipts.append(identity)
            return identity
        def baselines_after_receipt(*_args):
            self.assertEqual(len(receipts), 1)
            self.assertTrue(Path(receipts[0]["path"]).is_file())
            return baselines
        def metadata(_path, tools, *, limits, allow_raw_bilevel=False):
            self.assertEqual(set(tools), {"qpdf", "mutool", "pdfimages"})
            self.assertEqual(limits.max_draws_per_page, 256)
            self.assertEqual(allow_raw_bilevel, "-run1" not in _path.stem and _path != corrected)
            calls.append(set(tools))
            if unsupported:
                raise subject.pdf.PdfMetadataUnsupported("invented required metadata profile")
            return deepcopy(document)
        expected = {"source_rows": 18, "output_pages": 6, "draws": 6,
                    "type0_arrays": 0, "jpeg_streams": 6, "jpeg_color_spaces": 6,
                    "page_renderer_pairs": 12}
        paths = {"corpus": corpus, "reference_report": reference_report, "table": table,
                 "artifact_root": artifacts, "native_tool": native}
        if corrected_hn_b:
            paths["corrected_hn_b"] = corrected
        with patch.object(subject, "_pinned_json", side_effect=pinned), \
                patch.object(subject, "_baselines", side_effect=baselines_after_receipt), \
                patch.object(subject, "_execution_receipt", side_effect=receipt), \
                patch.object(subject, "_require_original_native", return_value=None), \
                patch.object(subject, "_audit", side_effect=audit), \
                patch.object(subject.Commands, "run", command), \
                patch.object(subject.pdf, "extract_pdf_metadata", side_effect=metadata), \
                patch.object(subject, "EXPECTED", expected), \
                patch.object(subject, "CORRECTED_HNB_PIN", (corrected_identity["size_bytes"], corrected_identity["sha256"])), \
                patch.object(subject, "RENDER_LAUNCH_LIMIT", 24), \
                patch.object(subject.time, "monotonic", side_effect=lambda: now[0]):
            result = subject.run(paths, native_sha256="0"*64, native_source_sha256="1"*64)
        self.assertEqual(result["counts"]["corrected_reference_checks_before"], int(corrected_hn_b))
        if corrected_hn_b:
            self.assertEqual(result["counts"]["corrected_reference_checks_after"], 1)
            self.assertIn("legacy parity not claimed", result["reference_policy"])
            self.assertEqual(result["profiles"][-1]["comparison_basis"], "separately pinned corrected grayscale reference")
            self.assertEqual(result["input_audit"]["corrected_hn_b_reference"], corrected_identity)
            self.assertTrue(all(page["draws"][0]["color_space"] == "DeviceRGB" for page in cases[-1]["pdf_pages"]))
        self.assertEqual(len(calls), 1 if unsupported else 2 if pixel_failure else 6)
        self.assertEqual(len(snapshots), 2)
        self.assertNotIn("versions", snapshots[0]["environment_audit"])
        self.assertEqual(result["counts"]["native_launches"], 1 if pixel_failure or unsupported else 3)
        self.assertEqual(result["counts"]["render_launches"], 0 if unsupported else 2 if pixel_failure else 24)
        self.assertEqual(result["counts"]["converter_launches"], 0)
        if unsupported:
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual([result["counts"][key] for key in ("profiles_attempted", "profiles_failing", "profiles_unsupported", "profiles_skipped")], [1, 1, 1, 2])
            self.assertEqual([result["counts"][key] for key in ("metadata_groups_attempted", "metadata_groups_failing", "metadata_groups_unsupported", "metadata_groups_passing")], [1, 1, 1, 0])
            self.assertEqual(result["counts"]["source_rows_attempted"], 0)
            self.assertEqual(result["counts"]["source_checks_after"], 27)
            self.assertEqual(result["counts"]["baseline_checks_after"], 6)
        elif pixel_failure:
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual([result["counts"][key] for key in ("profiles_attempted", "profiles_failing", "profiles_skipped", "native_completed")], [1,1,2,1])
            self.assertEqual([result["counts"][key] for key in ("page_renderer_pairs_attempted", "page_renderer_pairs_failing", "page_renderer_pairs_skipped")], [1,1,11])
            self.assertEqual(result["counts"]["source_checks_after"], 27)
            self.assertEqual(result["counts"]["baseline_checks_after"], 6)
            self.assertEqual(len(list(Path(result["artifact_session_path"]).glob("*.ppm"))), 2)
        elif expired:
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["counts"]["source_checks_after"], 27)
            self.assertTrue(any("including required post-audits" in error for error in result["errors"]))
        elif mutate:
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["environment_audit"]["after_status"], "FAIL")
        else:
            self.assertEqual(result["status"], "PASS", result["errors"])
            self.assertEqual(result["counts"]["source_checks_after"], 27)
            self.assertTrue(all(result["counts"][f"{kind}_passing"] == count for kind, count in expected.items()))
            self.assertTrue(all(result["counts"][f"{kind}_failing"] == 0 for kind in expected))

    def test_refused_render_ceiling_has_no_phantom_launch(self):
        commands, report = self.commands()
        commands.kind = "render"
        report["counts"]["render_launches"] = subject.RENDER_LAUNCH_LIMIT
        with self.assertRaises(subject.CompositionError):
            commands.run([sys.executable, "-c", "print('unused')"], "refused",
                         subject.pdf.PdfMetadataLimits(), subject.pdf._Usage(), 100)
        self.assertEqual(report["counts"]["validator_launches"], 0)
        self.assertEqual(report["attempts"], [])

    def test_combined_native_and_validator_ceiling_refuses_without_launch(self):
        commands, report = self.commands()
        report["counts"]["validator_launches"] = subject.TOOL_LIMIT-1
        report["counts"]["native_launches"] = 1
        for kind in ("native", "validator", "render"):
            commands.kind = kind
            with self.assertRaisesRegex(subject.CompositionError, "combined"):
                commands.run([sys.executable, "-c", "print('unused')"], "refused",
                             subject.pdf.PdfMetadataLimits(), subject.pdf._Usage(), 100)
        self.assertEqual(report["counts"]["native_launches"], 1)
        self.assertEqual(report["counts"]["validator_launches"], subject.TOOL_LIMIT-1)
        self.assertEqual(report["attempts"], [])


if __name__ == "__main__":
    unittest.main()
