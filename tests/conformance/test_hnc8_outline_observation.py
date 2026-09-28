# SPDX-License-Identifier: MIT
"""Original runtime metadata/source controls; never a private outline oracle."""

from contextlib import ExitStack
from copy import deepcopy
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import generate_fixtures
import hnc8_outline_observation as subject

PUBLIC_CHILDREN = 0


def original_pdf(*, reverse_pages=False, stream_title=False, indirect_title=False):
    """Invented links/titles/targets, including an empty title and a GoTo action."""
    titles = ["  Original 中 😀\n ", "Nested", "", "Whitespace\t", "Sibling", "Action"]
    def title(number):
        return b"<" + (b"\xfe\xff"+titles[number].encode("utf-16-be")).hex().encode()+b">"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >>",
        2: b"<< /Type /Pages /Kids ["+(b"4 0 R 3 0 R" if reverse_pages else b"3 0 R 4 0 R")+b"] /Count 2 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
        4: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 80 120] /Resources << >> >>",
        5: b"<< /Type /Outlines /First 6 0 R /Last 11 0 R /Count 6 >>",
        6: b"<< /Title "+title(0)+b" /Parent 5 0 R /Next 10 0 R /First 7 0 R /Last 9 0 R /Count 3 /Dest [3 0 R /Fit] >>",
        7: b"<< /Title "+title(1)+b" /Parent 6 0 R /Next 8 0 R /Dest [4 0 R /XYZ 1.25 2.5 1.5] >>",
        8: b"<< /Title "+title(2)+b" /Parent 6 0 R /Prev 7 0 R /Next 9 0 R /Dest [3 0 R /FitH null] >>",
        9: b"<< /Title "+title(3)+b" /Parent 6 0 R /Prev 8 0 R /Dest [4 0 R /FitR 1 2 3 4] >>",
        10: b"<< /Title "+title(4)+b" /Parent 5 0 R /Prev 6 0 R /Next 11 0 R /Dest [4 0 R /FitV 12] >>",
        11: b"<< /Title "+title(5)+b" /Parent 5 0 R /Prev 10 0 R /A << /S /GoTo /D [3 0 R /XYZ null null 0] >> >>",
    }
    if stream_title:
        objects[7] = generate_fixtures.stream_object(b"original stream must not be queried as outline bytes")
    if indirect_title:
        objects[7] = objects[7].replace(title(1), b"12 0 R")
        objects[12] = title(1)
    return generate_fixtures.render_pdf(objects, tuple(objects)), titles


def original_source(variant="HN-A", records=2, pages=2):
    """Known public container framing with invented record field positions."""
    prefix = 348 if variant == "HN-A" else 216 if variant == "HN-B" else 80
    index = prefix + (records*308 if variant == "HN-A" else 0)
    data = bytearray(index+pages*20+16)
    data[:8] = b"HN\0\0\x90\x01\0\0" if variant == "HN-A" else b"HN\0\0\xc8\0\0\0" if variant == "HN-B" else b"\xc8\0\0\0ORIG"
    struct.pack_into("<i", data, 144 if variant != "C8" else 8, pages)
    if variant == "HN-A":
        struct.pack_into("<i", data, 344, records)
        for ordinal in range(records):
            start = 348+ordinal*308
            # This deliberately does not use the existing CAJ 256-byte field.
            text = (f"Original title {ordinal}" if ordinal else "  中 😀\t ").encode("utf-8")
            data[start+23:start+23+len(text)] = text
            struct.pack_into(">H", data, start+244, ordinal % pages + 1)
            struct.pack_into("<H", data, start+271, ordinal % 3)
    for number in range(pages):
        struct.pack_into("<iih", data, index+number*20, index+pages*20, 8, 0)
    return bytes(data)


class OriginalOutlineObservation(unittest.TestCase):
    def setUp(self):
        cache = Path.home()/".cache"/"caj2pdf-issue119-original"
        cache.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=cache)
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tools = {key: Path(sys.executable if key == "python" else shutil.which(key) or "missing").resolve()
                      for key in (*subject.TOOL_KEYS, "ldd")}
        for key, value in self.tools.items():
            self.assertTrue(value.is_file(), f"mandatory original test tool missing: {key}")

    def file(self, name, data):
        path = self.root/name
        path.write_bytes(data)
        return path

    def commands(self):
        session = Path(tempfile.mkdtemp(dir=self.root))
        (session/"scratch").mkdir()
        report = subject.new_report("public-original-controls")
        commands = subject.Commands(session, report, time.monotonic()+60)
        self.addCleanup(self.count_children, report)
        return commands, report

    @staticmethod
    def count_children(report):
        global PUBLIC_CHILDREN
        PUBLIC_CHILDREN += report["counts"]["validator_launches"]

    def pdf_outputs(self, *, reverse_pages=False, stream_title=False):
        data, titles = original_pdf(reverse_pages=reverse_pages, stream_title=stream_title)
        path = self.file(f"original-{reverse_pages}-{stream_title}.pdf", data)
        commands, report = self.commands()
        qpdf = commands.query([str(self.tools["qpdf"]), "--json", "--json-key=outlines",
                               "--json-stream-data=none", str(path)], "original-qpdf")
        _, refs = subject.parse_qpdf_outlines(qpdf)
        mupdf = commands.query([str(self.tools["mutool"]), "show", "-g", str(path), "pages", "trailer/Root/Outlines",
                                *[str(item["reference"].number) for item in refs]], "original-mupdf")
        return qpdf, mupdf, path, titles, commands, report

    def test_no_input_and_missing_inputs_have_no_reads_or_launches(self):
        with patch.object(subject, "FileReader", side_effect=AssertionError("read")), \
             patch.object(subject.process.subprocess, "Popen", side_effect=AssertionError("launch")):
            report = subject.run()
            failed = subject.run({})
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(failed["status"], "FAIL")
        for value in (report, failed):
            self.assertFalse(any(value["counts"].values()))
            self.assertFalse(value["attempts"])
            self.assertFalse(value["source_inventory"])

    def test_cli_no_input_and_incomplete_inputs_are_bounded(self):
        global PUBLIC_CHILDREN
        for args, status, code in (([], "NOT_RUN", 0), (["--corpus-dir", str(self.root)], "FAIL", 2)):
            PUBLIC_CHILDREN += 1
            result = subprocess.run([sys.executable, str(ROOT/"scripts/hnc8_outline_observation.py"), *args],
                                    check=False, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, code)
            self.assertLess(len(result.stdout), 65536)
            report = json.loads(result.stdout)
            self.assertEqual(report["status"], status)
            self.assertFalse(any(report["counts"].values()))

    def test_duplicate_nesting_literal_and_unicode_json_refusals(self):
        malformed = [b'{"a":1,"a":2}', b"["*145+b"0"+b"]"*145,
                     b'"'+b"a"*16385+b'"', b"9223372036854775808", b"1e100", b"NaN", b"\xff"]
        for value in malformed:
            with self.subTest(value=value[:24]), self.assertRaises(subject.ObservationError):
                subject.decode_json(value)
        with self.assertRaises(subject.ObservationError): subject.decode_json(b"{}", 1)
        with self.assertRaises(subject.ObservationError): subject.title_identity("\ud800")

    def test_exact_real_null_and_destination_parameters(self):
        left = subject.decode_json(b'["3 0 R","/XYZ",0.24000000000000001,null,0]')
        right = subject.decode_json(b'["3 0 R","/XYZ",0.24,null,0]')
        self.assertNotEqual(subject.destination(left), subject.destination(right))
        self.assertEqual(subject.destination(left)[2], ["0.24000000000000001", None, "0"])
        for value in ("name", ["3 1 R", "/Fit"], ["3 0 R", "/Fit", 0],
                      ["3 0 R", "/XYZ", True, None, 0], ["3 0 R", "/Unknown"]):
            with self.assertRaises(subject.ObservationError): subject.destination(value)

    def test_actual_tools_preserve_all_titles_nesting_and_views(self):
        qpdf, mupdf, _, titles, _, report = self.pdf_outputs()
        facts = subject.compare_outline_parsers(qpdf, mupdf)
        self.assertEqual(facts["entry_count"], 6)
        self.assertEqual(facts["page_count"], 2)
        self.assertEqual([row["depth"] for row in facts["entries"]], [0, 1, 1, 1, 0, 0])
        self.assertEqual([row["title_utf8_sha256"] for row in facts["entries"]],
                         [hashlib.sha256(title.encode()).hexdigest() for title in titles])
        self.assertEqual(facts["entries"][1]["destination_parameters"], ["1.25", "2.5", "1.5"])
        self.assertEqual(facts["entries"][-1]["destination_parameters"], [None, None, "0"])
        self.assertEqual(facts["entries"][2]["title_utf8_bytes"], 0)
        self.assertTrue(all(attempt["status"] == "PASS" and attempt["peak_rss_kib"] is not None for attempt in report["attempts"]))

    def test_display_outline_is_lossy_and_not_an_exact_target_oracle(self):
        qpdf, _, path, _, commands, _ = self.pdf_outputs()
        display = commands.query([str(self.tools["mutool"]), "show", str(path), "outline"], "original-lossy-display")
        entries, _ = subject.parse_qpdf_outlines(qpdf)
        self.assertEqual(entries[-1]["destination_parameters"], [None, None, "0"])
        self.assertIn(b"zoom=100,nan,nan", display)
        self.assertNotIn(b"/XYZ", display)

    def test_actual_page_order_disagreement_is_detected(self):
        qpdf, _, _, _, _, _ = self.pdf_outputs()
        _, reversed_mupdf, _, _, _, _ = self.pdf_outputs(reverse_pages=True)
        with self.assertRaisesRegex(subject.ObservationError, "observations differ"):
            subject.compare_outline_parsers(qpdf, reversed_mupdf)

    def test_actual_raw_object_query_marks_stream_without_exposing_payload(self):
        data, _ = original_pdf(stream_title=True)
        path = self.file("stream-object.pdf", data)
        commands, _ = self.commands()
        value = commands.query([str(self.tools["mutool"]), "show", "-g", str(path), "7"], "original-stream-marker")
        self.assertIn(b"stream", value)
        self.assertNotIn(b"original stream must not", value)
        with self.assertRaises(subject.ObservationError):
            subject.PdfValueParser(value.split(b" obj ", 1)[1].strip()).complete()

    def test_linked_child_sibling_cycles_and_stream_refusals(self):
        qpdf, mupdf, _, _, _, _ = self.pdf_outputs()
        variants = [b"\n".join(line for line in mupdf.splitlines() if not line.startswith(b"8 0 obj")),
                    b"\n".join(line for line in mupdf.splitlines() if not line.startswith(b"10 0 obj")),
                    mupdf.replace(b"/Prev 7 0 R", b"/Prev 9 0 R"),
                    mupdf.replace(b"/Next 8 0 R", b"/Next 7 0 R"),
                    mupdf.replace(b"/Last 9 0 R", b"/Last 8 0 R"),
                    mupdf.replace(b"/Parent 6 0 R", b"/Parent 5 0 R", 1),
                    mupdf.replace(b"7 0 obj ", b"7 0 obj << /Length 1 >> stream % ", 1)]
        for value in variants:
            with self.subTest(digest=subject.byte_identity(value)), self.assertRaises(subject.ObservationError):
                subject.compare_outline_parsers(qpdf, value)

    def test_qpdf_schema_and_named_remote_targets_are_refused(self):
        qpdf, mupdf, _, _, _, _ = self.pdf_outputs()
        source = json.loads(qpdf)
        for mutate in (lambda d: d.update(version=True),
                       lambda d: d["outlines"][0].update(dest="named"),
                       lambda d: d["outlines"][0].update(destpageposfrom1=0),
                       lambda d: d["outlines"][0].update(object=d["outlines"][0]["kids"][0]["object"]),
                       lambda d: d["outlines"][0].update(title="\ud800")):
            changed = deepcopy(source); mutate(changed)
            with self.assertRaises(subject.ObservationError): subject.parse_qpdf_outlines(subject.json_bytes(changed))
        for value in (mupdf.replace(b"/GoTo", b"/GoToR"), mupdf.replace(b"/Dest[3 0 R/Fit]", b"/Dest(named)")):
            with self.assertRaises(subject.ObservationError): subject.compare_outline_parsers(qpdf, value)

    def test_actual_indirect_title_is_counted_as_unsupported(self):
        with self.assertRaises(subject.UnsupportedObservation): subject.pdf_title(subject.Ref(12))
        data, _ = original_pdf(indirect_title=True)
        path = self.file("indirect-title.pdf", data)
        commands, report = self.commands()
        report["progress"]["pdfs"]["planned"] = 1
        with subject.FileReader(path, report) as reader, self.assertRaises(subject.UnsupportedObservation):
            subject.observe_pdf(commands, reader, self.tools, "original-indirect-title")
        self.assertEqual(report["progress"]["pdfs"]["attempted"], 1)
        self.assertEqual(report["progress"]["pdfs"]["failed"], 1)
        self.assertEqual(report["progress"]["pdfs"]["unsupported"], 1)
        self.assertEqual(report["progress"]["queries"]["completed"], 2)
        self.assertEqual(report["counts"]["validator_launches"], 2)

    def test_small_pdf_display_grammar_preserves_bytes_and_refuses_extra(self):
        value = subject.PdfValueParser(b'<< /T(a\\n\\t\\050b\\051\\\r\nc) /N <414> /D [3 0 R /XYZ -.25 null +0] >>').complete()
        self.assertEqual(value["/T"], b"a\n\t(b)c")
        self.assertEqual(value["/N"], b"A@")
        self.assertEqual(value["/D"][0], subject.Ref(3))
        self.assertEqual(value["/D"][2], Decimal("-.25"))
        for data in (b"<< /T(a) /T(b) >>", b"(truncated", b"<GG>", b"/Name#x0", b"[3 1 R]",
                     b"[0 0 R]", b"<<>> stream", b"true false", b"["*66+b"null"+b"]"*66):
            with self.subTest(data=data[:32]), self.assertRaises(subject.ObservationError):
                subject.PdfValueParser(data).complete()
        with self.assertRaises(subject.UnsupportedObservation): subject.pdf_title(b"\x80")
        with self.assertRaises(subject.ObservationError): subject.pdf_title(b"\xfe\xff\xd8\x00")

    def test_no_outline_is_not_a_positive_title_oracle(self):
        objects = generate_fixtures.pdf_objects()
        objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
        path = self.file("no-outlines.pdf", generate_fixtures.render_pdf(objects, tuple(objects)))
        commands, _ = self.commands()
        qpdf = commands.query([str(self.tools["qpdf"]), "--json", "--json-key=outlines", "--json-stream-data=none", str(path)], "original-empty-qpdf")
        mupdf = commands.query([str(self.tools["mutool"]), "show", "-g", str(path), "pages", "trailer/Root/Outlines"], "original-empty-mupdf")
        result = subject.compare_outline_parsers(qpdf, mupdf)
        self.assertEqual(result["entry_count"], 0)
        self.assertEqual(result["page_count"], 2)

    def test_reported_depth64_leaf_and_depth65_refusal_in_both_parsers(self):
        def chain(depth):
            child = []
            nodes = []
            raw = [b"page 1 = 3 0 R", b"5 0 obj <</Type/Outlines/First 6 0 R/Last 6 0 R>>"]
            expected = []
            for level in range(depth, -1, -1):
                number = 6+level
                node = {"object":f"{number} 0 R", "title":f"Original level {level}", "open":True,
                        "dest":["3 0 R","/Fit"], "destpageposfrom1":1, "kids":child}
                child = [node]
            for level in range(depth+1):
                number = 6+level
                links = f"/First {number+1} 0 R/Last {number+1} 0 R" if level < depth else ""
                parent = 5 if level == 0 else number-1
                raw.append(f"{number} 0 obj <</Title(Original level {level})/Parent {parent} 0 R{links}/Dest[3 0 R/Fit]>>".encode())
                expected.append({"reference":subject.Ref(number),"page":subject.Ref(3),"depth":level})
            qpdf = subject.json_bytes({"version":2,"parameters":{"decodelevel":"generalized"},"outlines":child})
            return qpdf,b"\n".join(raw)+b"\n",expected
        qpdf,raw,_ = chain(64)
        common = subject.compare_outline_parsers(qpdf,raw)
        self.assertEqual(common["entry_count"],65)
        self.assertEqual(common["entries"][-1]["depth"],64)
        qpdf,raw,expected = chain(65)
        with self.assertRaises(subject.ObservationError): subject.parse_qpdf_outlines(qpdf)
        with self.assertRaises(subject.ObservationError): subject.parse_mutool_objects(raw,expected)

    def test_original_header_variants_and_range_failures(self):
        for variant in ("HN-A", "C8", "HN-B"):
            path = self.file(variant, original_source(variant))
            report = subject.new_report()
            with subject.FileReader(path, report) as reader:
                facts = subject.inventory_header(reader)
            self.assertEqual(facts["variant"], variant)
            self.assertEqual(facts["source_pages"], 2)
            self.assertEqual(facts["outline_like_records"], 2 if variant == "HN-A" else None)
            self.assertLessEqual(report["resources"]["max_field_read_request_bytes"], 4096)
        changed = bytearray(original_source()); struct.pack_into("<i", changed, 344, -1)
        outside = bytearray(original_source()); struct.pack_into("<i", outside, 348+2*308, len(outside)+1)
        for number, data in enumerate((b"other bytes", bytes(changed), bytes(outside), original_source()[:347])):
            path = self.file(f"invalid-header-{number}", data)
            with subject.FileReader(path, subject.new_report()) as reader, self.assertRaises(subject.ObservationError):
                subject.inventory_header(reader)

    def test_streamed_candidate_enumeration_does_not_choose_fields(self):
        data = original_source(records=2)
        records = [data[348+number*308:348+(number+1)*308] for number in range(2)]
        titles = ["  中 😀\t ", "Original title 1"]
        refs = [subject.common_entry(i+1, i, title, i, "/Fit", []) for i, title in enumerate(titles)]
        result = subject.enumerate_records(iter(records), refs, 2, record_count=2)
        self.assertEqual(result["status"], "DISCOVERY_ONLY")
        self.assertEqual(result["title_candidates_attempted"], 2*308*4)
        self.assertGreaterEqual(result["numeric_candidates_attempted"], 2*2448)
        self.assertTrue(any(row["offset"] == 23 and row["codec"] == "utf-8" and row["ordered_matches"] == 2
                            for row in result["title_candidates"]))
        self.assertNotIn("chosen_field", result)
        self.assertNotIn(titles[0], subject.json_bytes(result).decode())
        for items, count in ((records[:1], 2), (records, 1), ([b"x"], 1)):
            with self.assertRaises(subject.ObservationError): subject.enumerate_records(iter(items), refs, 2, record_count=count)
        with self.assertRaises(subject.ObservationError): subject.enumerate_records(iter(records), refs, 2)

    def test_title_windows_do_not_trim_repair_or_truncate_utf16(self):
        record = b" a\t \0"+b"x"*303
        windows = list(subject.title_windows(record))
        first = next(row for row in windows if row[:2] == (0, "utf-8"))
        self.assertEqual(first[4], subject.byte_identity(b" a\t "))
        odd = next(row for row in windows if row[:2] == (307, "utf-16-le"))
        self.assertIsNone(odd[4])
        invalid = b"\xff\0"+b"\0"*306
        self.assertIsNone(next(row for row in subject.title_windows(invalid) if row[:2] == (0, "utf-8"))[4])

    def test_reader_same_hashed_bytes_short_read_and_attempt_accounting(self):
        path = self.file("reader", b"0123456789")
        report = subject.new_report()
        real_pread = os.pread
        with subject.FileReader(path, report, subject.byte_identity(path.read_bytes())) as reader:
            with patch.object(subject.os, "pread", side_effect=lambda fd, n, offset: real_pread(fd, min(n, 2), offset)):
                self.assertEqual(reader.window(0, 10), b"0123456789")
                identity, data = reader.identity(retain=True)
            self.assertEqual(data, b"0123456789")
            self.assertEqual(identity, subject.byte_identity(data))
            calls = report["resources"]["field_read_calls"]
            with patch.object(subject.os, "pread", side_effect=OSError("original raised I/O")), self.assertRaises(OSError):
                reader.window(0, 1)
            self.assertEqual(report["resources"]["field_read_calls"], calls+1)

    def test_reader_zero_overreport_budget_and_cancel_refusals(self):
        path = self.file("reader", b"original")
        report = subject.new_report()
        with subject.FileReader(path, report) as reader:
            for output in (b"", b"too many"):
                with patch.object(subject.os, "pread", return_value=output), self.assertRaises(subject.ObservationError): reader.window(0, 1)
            report["resources"]["field_bytes_requested"] = subject.MAX_FIELD_REQUEST_BYTES
            with self.assertRaises(subject.ObservationError): reader.window(0, 1)
            report["resources"]["field_bytes_requested"] = 0
            report["cancelled"] = lambda: True
            with self.assertRaises(KeyboardInterrupt): reader.window(0, 1)
            report.pop("cancelled")
            with self.assertRaises(subject.ObservationError): reader.window(0, 1, 0)
            with self.assertRaises(subject.ObservationError): reader.request(0, 1, "unknown")

    def test_reader_rejects_symlinks_mutation_replacement_and_wrong_pin(self):
        path = self.file("reader", b"original")
        symlink = self.root/"link"; symlink.symlink_to(path)
        directory_link = self.root/"directory-link"; directory_link.symlink_to(self.root, target_is_directory=True)
        for target in (symlink, directory_link/"reader", self.root):
            with self.assertRaises((OSError, subject.ObservationError)): subject.FileReader(target, subject.new_report())
        with subject.FileReader(path, subject.new_report()) as reader:
            path.write_bytes(b"changed!")
            os.utime(path, ns=(reader.original.st_atime_ns, reader.original.st_mtime_ns+1_000_000_000))
            with self.assertRaises(subject.ObservationError): reader.identity()
        path.write_bytes(b"original")
        with subject.FileReader(path, subject.new_report()) as reader:
            moved = self.root/"held"; path.rename(moved); path.write_bytes(b"original")
            with self.assertRaises(subject.ObservationError): reader.identity()
        with subject.FileReader(path, subject.new_report(), {"size_bytes":8, "sha256":"0"*64}) as reader:
            with self.assertRaises(subject.ObservationError): reader.identity()

    def test_real_nonregular_paths_are_refused_without_blocking(self):
        fifo = self.root/"original-fifo"; os.mkfifo(fifo)
        endpoint = self.root/"original-socket"
        channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(channel.close)
        channel.bind(str(endpoint))
        commands, report = self.commands()
        script = ("import sys; sys.path.insert(0,sys.argv[1]); import hnc8_outline_observation as a\n"
                  "for path in sys.argv[2:]:\n"
                  " try: descriptor=a.open_regular_nofollow(path)\n"
                  " except (a.ObservationError,OSError): continue\n"
                  " else: raise AssertionError('nonregular input was accepted')\n"
                  "print('three nonregular inputs refused')\n")
        limits = subject.process.pdf.PdfMetadataLimits(timeout_seconds=.5)
        with patch.object(subject.process.pdf, "PdfMetadataLimits", return_value=limits):
            output = commands.query([sys.executable,"-c",script,str(ROOT/"scripts"),str(fifo),str(endpoint),str(self.root)],
                                    "original-nonregular-paths")
        self.assertEqual(output,b"three nonregular inputs refused\n")
        self.assertEqual(report["counts"]["validator_launches"],1)
        self.assertEqual(report["attempts"][0]["status"],"PASS")

    def test_audit_continues_after_missing_and_changed_file(self):
        right = self.file("right", b"original")
        entries = [("missing", self.root/"missing", subject.byte_identity(b"missing"), 100),
                   ("wrong", right, {"size_bytes":8, "sha256":"0"*64}, 100),
                   ("right", right, subject.byte_identity(b"original"), 100)]
        result = subject.audit(entries, subject.new_report(), time.monotonic()+10)
        self.assertEqual((result["attempted"], result["verified"], result["failed"]), (3, 1, 2))
        self.assertEqual(result["files"]["right"], subject.byte_identity(b"original"))

    def test_child_output_failure_timeout_and_launch_refusal_are_recorded(self):
        commands, report = self.commands()
        with patch.object(subject, "MAX_QUERY_BYTES", 128), self.assertRaises(subject.process.CompositionError):
            commands.query([sys.executable, "-c", "print('x'*8192)"], "original-output-refusal")
        self.assertEqual(report["progress"]["queries"]["failed"], 1)
        self.assertIsNotNone(report["attempts"][0]["peak_rss_kib"])
        limits = subject.process.pdf.PdfMetadataLimits(timeout_seconds=.1)
        with patch.object(subject.process.pdf, "PdfMetadataLimits", return_value=limits), self.assertRaises(subject.process.CompositionError):
            commands.query([sys.executable, "-c", "import time; time.sleep(10)"], "original-timeout")
        self.assertTrue(report["attempts"][-1]["timed_out"])
        with self.assertRaises(OSError): commands.query([str(self.root/"absent-tool")], "original-failed-launch")
        self.assertIsNone(report["attempts"][-1]["peak_rss_kib"])
        self.assertEqual(report["attempts"][-1]["status"], "FAIL")

    def test_closed_pipe_child_still_has_disk_cap_and_is_reaped(self):
        commands, report = self.commands()
        commands.directory_caps[commands.session] = (subject.MIB, 128)
        script = "import os,time; os.close(1);os.close(2);open('oversize','wb').write(b'x'*4096);time.sleep(10)"
        # CWD is inherited, so use the owned absolute session path explicitly.
        script = script.replace("'oversize'", repr(str(commands.session/"oversize")))
        with self.assertRaises(subject.process.CompositionError): commands.query([sys.executable,"-c",script], "original-closed-pipes")
        attempt = report["attempts"][-1]
        self.assertEqual(attempt["status"], "FAIL")
        self.assertIsNotNone(attempt["exit_code"])
        self.assertIsNotNone(attempt["peak_rss_kib"])
        self.assertEqual(attempt["stdout_bytes"], 0)
        self.assertEqual(attempt["stdout_sha256"], hashlib.sha256(b"").hexdigest())

    def test_closing_slots_refuse_without_phantom_children(self):
        commands, report = self.commands()
        report["counts"]["validator_launches"] = subject.MAX_CHILDREN-subject.CLOSING_CHILDREN
        with self.assertRaises(subject.ObservationError): commands.query(["never-launched"], "original-cap")
        self.assertEqual(report["counts"]["validator_launches"], 18)
        self.assertFalse(report["attempts"])
        commands.closing = True
        commands.query([sys.executable,"-c","pass"], "original-closing", provenance=True)
        # Manually installed counts are logical cap fixtures, not actual calls.
        report["counts"]["validator_launches"] = len(report["attempts"])

    def test_capture_requires_plain_name_and_reserves_before_write(self):
        commands, _ = self.commands()
        for name in ("../escape", "/absolute", ".", ".."):
            with self.assertRaises(subject.ObservationError): subject.capture(commands,b"a",name)
        with patch.object(commands,"disk",side_effect=subject.ObservationError("reserve")), self.assertRaises(subject.ObservationError):
            subject.capture(commands,b"a","not-written")
        self.assertFalse((commands.session/"not-written").exists())
        identity = subject.capture(commands,b"original","owned.txt")
        self.assertEqual(identity["sha256"],hashlib.sha256(b"original").hexdigest())
        self.assertEqual((commands.session/"owned.txt").stat().st_mode & 0o777,0o400)

    def test_protocol_freeze_source_coverage_and_schema_gates(self):
        path = self.file("plan.md", b"Status: **DRAFT.**\n")
        with self.assertRaises(subject.ObservationError): subject._load_plan(path, subject.byte_identity(path.read_bytes())["sha256"],subject.new_report(),time.monotonic()+10)
        contract = {"schema_version":1,"stage":"A","enumeration":subject.ENUMERATION,
                    "queries":"qpdf-outlines+mutool-g-objects","code_sha256":{}}
        def plan(value):
            return b"Status: **FROZEN BEFORE PRIVATE OBSERVATION.**\n<!-- execution-contract -->\n```json\n"+subject.json_bytes(value)+b"```\n"
        data=plan(contract); path.write_bytes(data)
        with patch.object(subject,"_source_files",return_value=[]):
            identity,pins=subject._load_plan(path,subject.byte_identity(data)["sha256"],subject.new_report(),time.monotonic()+10)
            self.assertEqual(pins,{})
            changed=deepcopy(contract);changed["schema_version"]=True
            data=plan(changed);path.write_bytes(data)
            with self.assertRaises(subject.ObservationError): subject._load_plan(path,subject.byte_identity(data)["sha256"],subject.new_report(),time.monotonic()+10)
        with self.assertRaises(subject.ObservationError): subject._matrix_source_path(self.root,{"path":"../other"})

    def test_startup_attempts_all_six_even_after_failed_first_version(self):
        commands, _ = self.commands()
        calls=[]
        library=self.file("original-library",b"original public library stand-in")
        def query(argv,label,**kwargs):
            calls.append(label)
            if label=="python-version": raise subject.ObservationError("original version failure")
            if label.endswith("startup-libraries"): return b"library => "+str(library).encode()+b" (0x123)\n"
            return (subject.process.VERSIONS[label.split("-")[0]]+"\n").encode()
        with patch.object(commands,"query",side_effect=query): result=subject._startup(commands,self.tools,commands.report)
        self.assertEqual(len(calls),6)
        self.assertEqual(result["status"],"FAIL")
        self.assertIn(str(library),result["libraries"])
        self.assertEqual(set(result["versions"]),{"qpdf","mutool"})
        self.assertTrue(all(path.is_file() for path in subject._codec_files()))

    def test_actual_public_versions_libraries_and_environment_repeat(self):
        commands, report = self.commands()
        # Original tools use the active host profile; private defaults remain
        # frozen, and no runtime profile fallback is exposed by the CLI.
        versions = {"python": "Python "+platform.python_version()}
        for key in ("qpdf", "mutool"):
            data = commands.query([str(self.tools[key]), "-v" if key == "mutool" else "--version"],
                                  "original-active-"+key, provenance=True)
            versions[key] = data.decode().strip().splitlines()[0]
        with patch.object(subject, "TOOL_VERSIONS", versions):
            before = subject._startup(commands, self.tools, report)
            commands.closing = True
            after = subject._startup(commands, self.tools, report)
        self.assertEqual(before["status"], "PASS", before["failures"])
        self.assertEqual(before, after)
        self.assertEqual(report["counts"]["validator_launches"], 14)
        self.assertEqual(report["progress"]["queries"]["attempted"], 0)
        self.assertTrue(before["libraries"])
        self.assertEqual(set(before["versions"]), set(subject.TOOL_KEYS))
        # A stat-only conservative bound includes every actual hash invocation:
        # setup+pre+post for public/code/tools/codecs/plan, four for libraries,
        # twice for all sources/repeated PDFs, thrice for the reference report,
        # and once for the at-most-1MiB generated receipt. No private file read.
        known_private = (2*(264_682_650+27)
                         +4*sum(size+1 for size,_ in subject.process.BASELINE_PINS.values())
                         +3*(subject.REFERENCE_BYTES+1))
        public = sum((ROOT/relative).stat().st_size+1 for relative,_ in subject.process.PUBLIC_PINS.values())
        code = sum(path.stat().st_size+1 for path in subject._source_files())
        tools = sum(path.stat().st_size+1 for path in self.tools.values())
        codecs = sum(path.stat().st_size+1 for path in subject._codec_files())
        libraries = sum(identity["size_bytes"]+1 for identity in before["libraries"].values())
        modeled = known_private+3*(public+code+tools+codecs+256*1024+1)+4*libraries+subject.MAX_QUERY_BYTES+1
        self.assertLessEqual(modeled,subject.MAX_HASH_REQUEST_BYTES)
        print(f"Conservative Stage A opaque request bound: {modeled} bytes",file=sys.stderr)

    def test_public_inventory_and_reference_counts_need_all_fixed_profiles(self):
        _, rows, basis = subject._public_inputs(subject.new_report(), time.monotonic()+10)
        self.assertEqual(len(rows), 27)
        self.assertEqual({key: value["output_page_count"] for key, value in basis.items()},
                         {"hn_a": 68, "c8": 7, "hn_b": 2})
        generations = []
        for profile, fact in basis.items():
            size, digest = subject.process.BASELINE_PINS[profile]
            generations.append({"profile": profile, "source_sha256": fact["source_sha256"], "deterministic": True,
                                "runs": [{"status": "PASS", "exit_code": 0, "pdf_size_bytes": size,
                                          "pdf_sha256": digest} for _ in (1, 2)]})
        document = {"status": "PASS", "generations": generations}
        self.assertEqual(subject._reference_basis(subject.json_bytes(document), rows, basis),
                         {"hn_a": 68, "c8": 7, "hn_b": 2})
        document["generations"][0]["source_sha256"] = basis["c8"]["source_sha256"]
        with self.assertRaises(subject.ObservationError):
            subject._reference_basis(subject.json_bytes(document), rows, basis)

    def test_unfrozen_contract_stops_before_any_private_input(self):
        plan = self.file("draft.md", b"Status: **DRAFT.**\n")
        paths = {"corpus": self.root/"never-open-corpus", "reference_report": self.root/"never-open-report",
                 "artifact_root": self.root/"never-create-artifacts", "plan": plan}
        for profile in subject.process.BASELINE_PINS:
            for repeat in (1, 2): paths[f"{profile}_{repeat}"] = self.root/f"never-open-{profile}-{repeat}.pdf"
        opened = []
        original = subject.open_regular_nofollow
        def confined(path):
            opened.append(Path(path))
            self.assertEqual(Path(path), plan)
            return original(path)
        with patch.object(subject, "open_regular_nofollow", side_effect=confined):
            report = subject.run(paths, plan_sha256=subject.byte_identity(plan.read_bytes())["sha256"])
        self.assertEqual(report["status"], "FAIL")
        self.assertFalse(any(report["counts"].values()))
        self.assertFalse(paths["artifact_root"].exists())
        self.assertTrue(opened)

    def test_final_persistence_expiration_never_leaves_pass(self):
        commands,report=self.commands()
        report["status"]="PASS";report["_started"]=0
        commands.deadline=10
        with patch.object(subject.time,"monotonic",side_effect=[1,2,11]):
            identity=subject.persist_report(commands,report)
        self.assertEqual(report["status"],"FAIL")
        stored=json.loads((commands.session/"observation-report.json").read_bytes())
        self.assertEqual(stored["status"],"FAIL")
        self.assertEqual(identity["sha256"],subject.byte_identity((commands.session/"observation-report.json").read_bytes())["sha256"])

    def orchestration(self, *, mutate=None, first_header_failure=False, first_pdf_failure=False):
        """Full original file/audit/receipt flow, with actual two-tool PDF calls."""
        corpus=self.root/"corpus";corpus.mkdir()
        rows=[]
        for profile,variant in (("hn_a","HN-A"),("c8","C8"),("hn_b","HN-B")):
            data=original_source(variant,records=52 if variant=="HN-A" else 0)
            path=corpus/(profile+".bin");path.write_bytes(data)
            rows.append({"path":path.name,"sha256":hashlib.sha256(data).hexdigest(),"size_bytes":len(data),"page_count":2})
        pdf,_=original_pdf()
        pins={profile:(len(pdf),hashlib.sha256(pdf).hexdigest()) for profile in ("hn_a","c8","hn_b")}
        basis={profile:{"source_sha256":row["sha256"],"output_page_count":2} for profile,row in zip(pins,rows)}
        reference={"status":"PASS","generations":[{"profile":profile,"source_sha256":row["sha256"],"deterministic":True,
                    "runs":[{"status":"PASS","exit_code":0,"pdf_size_bytes":len(pdf),"pdf_sha256":pins[profile][1]} for _ in (1,2)]}
                    for profile,row in zip(pins,rows)]}
        reference_data=subject.json_bytes(reference)
        reference_path=self.file("reference.json",reference_data)
        plan=self.file("original-plan.md",b"original synthetic coordination contract")
        paths={"corpus":corpus,"reference_report":reference_path,"artifact_root":self.root/"artifacts","plan":plan}
        for profile in pins:
            for repeat in (1,2): paths[f"{profile}_{repeat}"]=self.file(f"{profile}_{repeat}.pdf",pdf)
        original_run_report=subject.new_report
        original_request=subject.FileReader.request
        original_inventory=subject.inventory_header
        source_paths={corpus/row["path"] for row in rows}
        def guarded_request(reader,offset,length,scope):
            if reader.path in source_paths:
                receipts=list(paths["artifact_root"].glob("*/execution-receipt.json"))
                self.assertEqual(len(receipts),1,"source was read before immutable receipt")
                self.assertEqual(receipts[0].stat().st_mode & 0o777,0o400)
            return original_request(reader,offset,length,scope)
        def inventory(reader):
            if first_header_failure: raise KeyboardInterrupt("original interruption")
            return original_inventory(reader)
        original_observe=subject.observe_pdf
        def observe(*args,**kwargs):
            if first_pdf_failure: raise subject.UnsupportedObservation("original tool protocol failure")
            value=original_observe(*args,**kwargs)
            if mutate: mutate(paths,value)
            return value
        startup={"status":"PASS","failures":{},"versions":{},"libraries":{},"environment":{"sha256":"0"*64}}
        with ExitStack() as patches:
            patches.enter_context(patch.object(subject,"new_report",side_effect=lambda:original_run_report("public-original-controls")))
            patches.enter_context(patch.object(subject,"_load_plan",return_value=(subject.byte_identity(plan.read_bytes()),{})))
            patches.enter_context(patch.object(subject,"_public_inputs",return_value=([],rows,basis)))
            patches.enter_context(patch.object(subject,"_startup",return_value=startup))
            patches.enter_context(patch.object(subject.process,"BASELINE_PINS",pins))
            patches.enter_context(patch.object(subject.process,"TOOL_PINS",
                {key:subject.process.file_identity(self.tools[key],128*subject.MIB)["sha256"] for key in subject.TOOL_KEYS}))
            patches.enter_context(patch.object(subject,"REFERENCE_SHA",hashlib.sha256(reference_data).hexdigest()))
            patches.enter_context(patch.object(subject,"REFERENCE_BYTES",len(reference_data)))
            patches.enter_context(patch.object(subject,"DISCOVERY_SHA",rows[0]["sha256"]))
            patches.enter_context(patch.object(subject.FileReader,"request",guarded_request))
            patches.enter_context(patch.object(subject,"inventory_header",side_effect=inventory))
            if first_pdf_failure:
                # Fail through the real observer's own counter boundary.
                patches.enter_context(patch.object(subject,"parse_qpdf_outlines",side_effect=subject.UnsupportedObservation("original schema failure")))
            elif mutate:
                patches.enter_context(patch.object(subject,"observe_pdf",side_effect=observe))
            result=subject.run(paths,plan_sha256="0"*64)
        self.addCleanup(self.count_children,result)
        return result,paths

    def test_full_original_orchestration_receipt_counts_and_final_audit(self):
        report,_=self.orchestration()
        self.assertEqual(report["status"],"PASS",report["errors"])
        self.assertEqual(report["origin"],"public-original-controls")
        self.assertEqual(report["counts"]["validator_launches"],12)
        self.assertEqual(report["counts"]["aggregate_launches"],13)
        self.assertEqual(report["progress"]["sources"]["completed"],3)
        self.assertEqual(report["progress"]["pdfs"]["completed"],6)
        self.assertEqual(report["progress"]["queries"]["completed"],12)
        self.assertEqual(report["discovery"]["record_count"],52)
        self.assertFalse(report["discovery"]["ordered_alignment_hypothesis"])
        self.assertEqual(report["compatibility_status"],"UNVERIFIED")
        self.assertEqual(report["audits"]["before"]["files"],report["audits"]["after"]["files"])
        self.assertEqual(report["audits"]["runtime"]["status"],"PASS")

    def test_original_interruption_has_failed_attempt_and_complete_final_audit(self):
        report,_=self.orchestration(first_header_failure=True)
        self.assertEqual(report["status"],"FAIL")
        self.assertEqual(report["progress"]["sources"],{"planned":3,"attempted":1,"completed":0,"failed":1,"remaining":2,"unsupported":0})
        self.assertEqual(report["counts"]["validator_launches"],0)
        self.assertEqual(report["audits"]["after"]["attempted"],report["audits"]["before"]["attempted"])

    def test_original_pdf_protocol_failure_keeps_attempts_and_unstarted_work(self):
        report,_=self.orchestration(first_pdf_failure=True)
        self.assertEqual(report["status"],"FAIL")
        self.assertEqual(report["progress"]["pdfs"],{"planned":6,"attempted":1,"completed":0,"failed":1,"remaining":5,"unsupported":1})
        self.assertEqual(report["progress"]["queries"]["attempted"],1)
        self.assertEqual(report["progress"]["queries"]["completed"],1)
        self.assertEqual(report["audits"]["after"]["status"],"PASS")

    def test_original_post_audit_mutation_cannot_retain_integrity_pass(self):
        changed=[False]
        def mutate(paths,_):
            if not changed[0]:
                paths["hn_b_2"].write_bytes(b"changed original PDF")
                changed[0]=True
        report,_=self.orchestration(mutate=mutate)
        self.assertEqual(report["status"],"FAIL")
        self.assertEqual(report["audits"]["after"]["status"],"FAIL")
        self.assertGreaterEqual(report["audits"]["after"]["failed"],1)


if __name__=="__main__":
    try:
        unittest.main()
    finally:
        print(f"Original public child launches: {PUBLIC_CHILDREN}; private/native/converter/vendor launches: 0",file=sys.stderr)
