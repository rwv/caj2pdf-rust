# SPDX-License-Identifier: MIT
"""Original synthetic full-array, pixel, process and audit regression tests."""

from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import random
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_page_composition as subject


def fixture():
    """Invented compact six-row metadata; no source or converter fixture."""
    pages, source, outputs, lines = [], [], [], []
    ctm = [0.96, 0.0, 0.0, -0.48, 0.0, 0.48]
    digest = hashlib.sha256(b"original synthetic encoded identity").hexdigest()
    for number in range(1, 7):
        output = 1 if number == 1 else 2 if number == 6 else None
        image = {"image_number": 1, "record_type": 2, "descriptor_offset": 1000+number*20,
                 "payload_offset": 1500+number*20, "payload_length": 12, "width": 4, "height": 2,
                 "payload_sha256": digest}
        images = [image] if output else []
        source.append({"page_number": number, "text_offset": 500+number*16,
                       "text_length": 8, "images": deepcopy(images)})
        pages.append({"source_page": number, "row_offset": 100+(number-1)*20,
                      "text_offset": 500+number*16, "text_length": 8,
                      "image_count": len(images), "output_page": output,
                      "media_box": [0.0, 0.0, 0.96, 0.48] if output else None,
                      "images": [{**image, "page_number": number, "display_width": 4, "pdf_ctm": ctm}] if output else []})
        lines.append("\t".join(map(str, ["P", number, 100+(number-1)*20, 500+number*16,
                                         8, len(images), output or 0, 0, 0,
                                         0.96 if output else 0, 0.48 if output else 0])))
        if output:
            lines.append("\t".join(map(str, ["I", number, 1, 2, image["descriptor_offset"],
                                             image["payload_offset"], 12, 4, 4, 2, *ctm])))
            draw = {"draw_number": 1, "width": 4, "height": 2, "bits_per_component": 8,
                    "pdf_ctm": ctm, "filter": "/DCTDecode", "color_space": "DeviceGray",
                    "raw_stream_sha256": digest, "raw_stream_length": 12, "object_id": 20+output}
            outputs.append({"page_number": output, "media_box": [0.0, 0.0, 0.96, 0.48], "draws": [draw]})
    # Seventeen numeric resource fields after the variant.
    values = [6, 2, 4, 0, 2, 256, 0, 0, 0, 0, 100, 32, 1, 1, 0, 0, 0]
    lines.append("\t".join(map(str, ["R", "HN-B", *values])))
    case = {"source_variant": "HN-B", "source_id": "invented", "source_pages": source,
            "output_page_to_source_page": [1, 6], "pdf_pages": outputs, "pdf_sha256": digest}
    return {"variant": "HN-B", "pages": pages}, {"pages": outputs}, case, ("\n".join(lines)+"\n").encode()


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
        self.assertEqual([counts[f"{kind}_passing"] for kind in ("source_rows", "output_pages", "draws", "jpeg_streams")], [6, 2, 2, 2])
        self.assertTrue(all(row["status"] == "PASS" for row in progress))

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
                    path.write_bytes(b"P6\n1 1\n255\n"+(b"\xff\xff" if malformed else
                                      b"\x00\x00\x00" if "candidate" in path.name else b"\xff\xff\xff"))
                    return subject.file_identity(path, subject.RASTER_LIMIT)
                with patch.object(subject, "_to_file", side_effect=write_pair), self.assertRaises(subject.CompositionError):
                    subject.check_pixels(commands, self.root/"ref", self.root/"candidate",
                                         [{"page_number": 1, "media_box": [0,0,.24,.24]}],
                                         {"mutool": Path("mutool")}, result)
                self.assertEqual((report["counts"]["page_renderer_pairs_attempted"], report["counts"]["page_renderer_pairs_failing"]), (1, 1))
                self.assertEqual(len(list(session.glob("*.ppm"))), 2)
                self.assertEqual(result["progress"][0]["status"], "FAIL")

    def test_end_to_end_immutable_pre_post_audits_and_exact_metadata_signature(self):
        self._end_to_end(False)

    def test_end_to_end_post_audit_mutation_fails(self):
        self._end_to_end(True)

    def test_end_to_end_first_pixel_failure_preserves_failure_and_post_audits(self):
        self._end_to_end(False, pixel_failure=True)

    def test_end_to_end_timeout_including_post_audits_fails(self):
        self._end_to_end(False, expired=True)

    def _end_to_end(self, mutate, *, pixel_failure=False, expired=False):
        """Exercise runner control flow using only invented files/observations.

        The compact oracle has three artificial profiles sharing the six-row
        fixture. No converter is called, and this is not compatibility data.
        """
        _, document, case, data = fixture()
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
        rows = []
        cases = []
        baselines = {}
        for name in ("hn_a", "c8", "hn_b"):
            (corpus/f"{name}.caj").write_bytes(b"synthetic source stub")
            rows.append({"id": name, "path": f"{name}.caj", "detected_type": "HN",
                         "size_bytes": 4096, "sha256": "0"*64})
            cases.append({**deepcopy(case), "case": name, "source_id": name})
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
            snapshots.append(deepcopy(result))
            return result
        def pinned(path, _sha):
            if path.name == "execution-receipt.json":
                return {"harness_files": {}, "synthetic": True}
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
                pixels = b"P6\n4 2\n255\n"+b"\x00\xff\x00"*8
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
            path.write_text(json.dumps({"harness_files": {}, "synthetic": True}))
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
            self.assertEqual(allow_raw_bilevel, "-run1" not in _path.stem)
            calls.append(set(tools))
            return deepcopy(document)
        expected = {"source_rows": 18, "output_pages": 6, "draws": 6,
                    "type0_arrays": 0, "jpeg_streams": 6, "page_renderer_pairs": 12}
        paths = {"corpus": corpus, "reference_report": reference_report, "table": table,
                 "artifact_root": artifacts, "native_tool": native}
        with patch.object(subject, "_pinned_json", side_effect=pinned), \
                patch.object(subject, "_baselines", side_effect=baselines_after_receipt), \
                patch.object(subject, "_execution_receipt", side_effect=receipt), \
                patch.object(subject, "_audit", side_effect=audit), \
                patch.object(subject.Commands, "run", command), \
                patch.object(subject.pdf, "extract_pdf_metadata", side_effect=metadata), \
                patch.object(subject, "EXPECTED", expected), \
                patch.object(subject, "RENDER_LAUNCH_LIMIT", 24), \
                patch.object(subject.time, "monotonic", side_effect=lambda: now[0]):
            result = subject.run(paths, native_sha256="0"*64, native_source_sha256="1"*64)
        self.assertEqual(len(calls), 2 if pixel_failure else 6)
        self.assertEqual(len(snapshots), 2)
        self.assertNotIn("versions", snapshots[0]["environment_audit"])
        self.assertEqual(result["counts"]["native_launches"], 1 if pixel_failure else 3)
        self.assertEqual(result["counts"]["render_launches"], 2 if pixel_failure else 24)
        self.assertEqual(result["counts"]["converter_launches"], 0)
        if pixel_failure:
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
