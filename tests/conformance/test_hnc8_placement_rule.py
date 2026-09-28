# SPDX-License-Identifier: MIT
"""Original invented source/TSV checks; no external corpus or converter."""

from contextlib import ExitStack, redirect_stdout
import copy
from dataclasses import replace
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_placement_rule as rule  # noqa: E402


PREFIX = b"MIT invented prefix!"
ITEM = struct.pack("<8H", 0x8070, 4, 0x8071, 7, 0x8001, 2, 3, 9)
MARKERS = ITEM[:2] + ITEM[4:6] + ITEM[8:10]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def invented_fixture(variant: str = "C8", source_id: str = "invented.caj") -> tuple[bytes, dict, bytes]:
    """Two pages, padded type-0 width, an off-page draw and a repeated hash."""
    source = bytearray(4096)
    specifications = (
        (100, [(0, 33, 64, 40, 0, 0, 500, b"MIT first payload"),
               (2, 7, 7, 9, 12, 100, 600, b"MIT repeat payload")]),
        (1000, [(2, 20, 20, 30, 1, 2, 1500, b"MIT repeat payload")]),
    )
    source_pages, pdf_pages, rows = [], [], [f"H\t{variant}\t2"]
    for number, (offset, images) in enumerate(specifications, 1):
        tails = b"".join(struct.pack("<HH", image[4], image[5]) + b"opaque MIT tail bytes!!!" for image in images)
        assert len(tails) == 28 * len(images)
        decoded = b"MIT head" + ITEM * number + b"MIT!" + tails
        encoded = zlib.compress(decoded)
        framed = PREFIX + struct.pack("<I", len(decoded)) + encoded
        source[offset:offset + len(framed)] = framed
        source_page = {"page_number": number, "text_offset": offset, "text_length": len(framed),
                       "text_sha256": sha(framed), "images": []}
        width, height = images[0][2] * 0.24, images[0][3] * 0.24
        box = [0.0, 0.0, width, height]
        pdf_page = {"page_number": number, "media_box": box, "draws": []}
        rows.append("\t".join(map(str, ("P", number, len(images), offset, len(framed), len(decoded),
                                       number, sha(encoded), sha(decoded), 128, 64, 1024, 136192))))
        rows.append("\t".join(map(str, ("B", number, *box))))
        for image_number, (kind, visible, display, image_height, x, y, payload, data) in enumerate(images, 1):
            source[payload:payload + len(data)] = data
            source_image = {"image_number": image_number, "record_type": kind,
                            "descriptor_offset": payload - 12, "payload_offset": payload,
                            "payload_length": len(data), "payload_sha256": sha(data),
                            "width": visible, "height": image_height}
            if kind == 0:
                source_image["stride_width"] = display
            matrix = [display * 0.24, 0.0, 0.0, -image_height * 0.24,
                      x * 240 / 2473, height - y * 240 / 2473]
            pdf_page["draws"].append({"draw_number": image_number, "width": display,
                                       "height": image_height, "pdf_ctm": matrix,
                                       "raw_stream_sha256": sha(data) if kind == 2 else "f" * 64})
            source_page["images"].append(source_image)
            rows.append("\t".join(map(str, (
                "I", number, image_number, kind, payload - 12, payload, len(data), visible,
                display, image_height, x, y, sha(data), *matrix))))
        source_pages.append(source_page)
        pdf_pages.append(pdf_page)
    rows.append("R\t4096\t128\t1024\t136192\t0")
    case = {"source_variant": variant, "source_id": source_id, "source_pages": source_pages,
            "output_page_to_source_page": [1, 2], "pdf_pages": pdf_pages}
    return bytes(source), case, ("\n".join(rows) + "\n").encode("ascii")


def change_field(data: bytes, row_number: int, column: int, value: str) -> bytes:
    rows = data.decode("ascii").splitlines()
    fields = rows[row_number].split("\t")
    fields[column] = value
    rows[row_number] = "\t".join(fields)
    return ("\n".join(rows) + "\n").encode("ascii")


class PlacementRuleTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.data, self.case, self.tsv = invented_fixture()
        self.source = self.root / "invented.caj"
        self.source.write_bytes(self.data)
        profiles = patch.dict(rule.frame.PROFILES, {
            name: rule.frame.FrameProfile(name, sha(PREFIX), sha(MARKERS))
            for name in rule.SUPPORTED_PROFILES})
        profiles.start()
        self.addCleanup(profiles.stop)
        splits = patch.object(rule.analysis, "PAGE_SPLITS", {
            "discovery": {"hn_a": (1,), "c8": (1,)},
            "validation": {"hn_a": (), "c8": ()}})
        splits.start()
        self.addCleanup(splits.stop)

    def compare(self, *, tsv: bytes | None = None, case: dict | None = None) -> tuple[dict, dict]:
        report = rule._report()
        native = rule.parse_native_tsv(tsv or self.tsv, len(self.data))
        result = rule.compare_native(native, case or self.case, "c8", self.source,
                                     report["counts"], report["resources"])
        return result, report

    def test_clean_clone_is_not_run_and_all_counters_are_zero(self) -> None:
        report = rule.run()
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(set(report["counts"].values()), {0})
        self.assertEqual(report["profiles"], [])
        self.assertEqual(report["resources"]["harness_vmhwm_kib"], 0)
        self.assertEqual(report["native_audit"]["status"], "NOT_RUN")
        self.assertNotEqual(report["placement_rule_status"], "EMPIRICAL_PROFILE_VALIDATED_SAME_DOCUMENTS")

    def test_valid_native_order_dimensions_raw_words_and_resources(self) -> None:
        result = rule.parse_native_tsv(self.tsv, len(self.data))
        self.assertEqual((result["variant"], result["page_count"], result["draw_count"]), ("C8", 2, 3))
        first, supplemental = result["pages"][0]["images"]
        self.assertEqual((first["width"], first["display_width"]), (33, 64))
        self.assertEqual((supplemental["x_word"], supplemental["y_word"]), (12, 100))
        self.assertLess(supplemental["pdf_ctm"][5], 0)
        self.assertEqual(supplemental["payload_sha256"], result["pages"][1]["images"][0]["payload_sha256"])
        self.assertEqual(result["resources"]["temporary_bytes"], 0)

    def test_truncated_duplicate_extra_and_reordered_rows_fail(self) -> None:
        rows = self.tsv.splitlines(keepends=True)
        bad = (self.tsv[:-1], b"".join(rows[:-1]), b"".join(rows[:3]),
               self.tsv + rows[-1], b"".join([rows[0], rows[2], rows[1], *rows[3:]]),
               change_field(self.tsv, 4, 2, "1"), change_field(self.tsv, 5, 1, "1"),
               change_field(self.tsv, 0, 2, "3"), change_field(self.tsv, 1, 2, "1"),
               self.tsv + b"\n", self.tsv.replace(b"\n", b"\r\n"), self.tsv + b"\x00\n")
        for data in bad:
            with self.subTest(data=data[:40]), self.assertRaises(rule.RuleError):
                rule.parse_native_tsv(data, len(self.data))

    def test_numeric_text_boolean_nonfinite_and_huge_fields_fail(self) -> None:
        bad = ((0, 2, "True"), (1, 3, "-1"), (1, 5, "1.0"), (1, 6, "+1"),
               (3, 10, "65536"), (3, 10, "-32768"), (3, 7, "0"), (3, 3, "1"),
               (3, 13, "NaN"), (3, 13, "inf"), (3, 13, "True"), (3, 13, "1e999"),
               (3, 13, "1e10"), (1, 3, "9" * 500), (1, 7, "A" * 64))
        for row, column, value in bad:
            with self.subTest(value=value[:40]), self.assertRaises(rule.RuleError):
                rule.parse_native_tsv(change_field(self.tsv, row, column, value), len(self.data))
        for value in (True, False, -1, 0, 1.0, 2**64):
            with self.subTest(source_size=value), self.assertRaises(rule.RuleError):
                rule.parse_native_tsv(self.tsv, value)
        for value in (True, False, float("nan"), float("inf"), 10**500, "NaN"):
            with self.subTest(number=str(value)[:40]), self.assertRaises(rule.RuleError):
                rule._number(value, "test")

    def test_output_line_frame_span_and_memory_caps_fail(self) -> None:
        bad = (b"x" * (rule.MAX_OUTPUT_BYTES + 1), b"H\tC8\t2\n" + b"x" * 1025 + b"\n",
               self.tsv.replace(b"C8", b"\xff8", 1),
               change_field(self.tsv, 1, 4, "1048577"), change_field(self.tsv, 1, 3, "4090"),
               change_field(self.tsv, 1, 5, "1048577"), change_field(self.tsv, 1, 6, "65537"),
               change_field(self.tsv, 1, 9, "4097"), change_field(self.tsv, 1, 10, "4097"),
               change_field(self.tsv, 1, 12, "1024"), change_field(self.tsv, 1, 12, "132096"),
               change_field(self.tsv, 1, 12, "1048577"),
               change_field(self.tsv, 3, 5, "4096"), change_field(self.tsv, 3, 4, "4090"),
               change_field(self.tsv, 3, 8, "33"), change_field(self.tsv, 4, 8, "8"),
               change_field(self.tsv, 8, 3, "1025"), change_field(self.tsv, 8, 4, "136193"),
               change_field(self.tsv, 8, 5, "1"), change_field(self.tsv, 8, 1, "1"))
        for data in bad:
            with self.subTest(prefix=data[:40]), self.assertRaises(rule.RuleError):
                rule.parse_native_tsv(data, len(self.data))

    def test_independent_frame_and_all_geometry_pass_with_spool_accounting(self) -> None:
        result, report = self.compare()
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(report["counts"]["source_frames_compared"], 2)
        self.assertEqual(report["counts"]["source_images_compared"], 3)
        self.assertEqual(report["counts"]["page_boxes_passing"], 2)
        self.assertEqual(report["counts"]["first_draws_passing"], 2)
        self.assertEqual(report["counts"]["discovery_draws_passing"], 1)
        self.assertGreater(report["resources"]["max_frame_decoded_spool_bytes"], 0)
        self.assertEqual(report["resources"]["retained_frame_spool_bytes"], 0)
        self.assertEqual(self.source.read_bytes(), self.data)
        self.assertNotEqual(result["pages"][0]["frame"]["encoded_sha256"], self.case["source_pages"][0]["text_sha256"])

    def test_every_ctm_component_and_box_failure_is_detected_at_explicit_tolerance(self) -> None:
        parsed = rule.parse_native_tsv(self.tsv, len(self.data))
        for component in range(6):
            value = parsed["pages"][0]["images"][1]["pdf_ctm"][component]
            bad = change_field(self.tsv, 4, 13 + component, str(value + rule.TOLERANCE_PT * 2))
            result, report = self.compare(tsv=bad)
            with self.subTest(component=component):
                self.assertEqual(result["status"], "FAIL")
                self.assertEqual(report["counts"]["discovery_draws_failing"], 1)
                self.assertEqual(report["counts"]["first_draws_passing"], 2)
        result, report = self.compare(tsv=change_field(self.tsv, 2, 4, "15.3601"))
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(report["counts"]["page_boxes_failing"], 1)
        close = change_field(self.tsv, 4, 17, str(parsed["pages"][0]["images"][1]["pdf_ctm"][4] + 0.00004))
        self.assertEqual(self.compare(tsv=close)[0]["status"], "PASS")

    def test_identity_frame_and_raw_word_mismatches_are_protocol_failures(self) -> None:
        for row, column, value in ((1, 7, "0" * 64), (1, 8, "0" * 64),
                                   (3, 12, "0" * 64), (3, 4, "487"),
                                   (4, 10, "13"), (4, 9, "10")):
            with self.subTest(column=column), self.assertRaises(rule.RuleError):
                self.compare(tsv=change_field(self.tsv, row, column, value))
        for mutate in (
            lambda case: case.update(source_variant="HN-A"),
            lambda case: case.update(output_page_to_source_page=[2, 1]),
            lambda case: case["source_pages"][0]["images"][0].update(width=True),
            lambda case: case["pdf_pages"][0]["draws"][0].update(width=True),
            lambda case: case["pdf_pages"][0].update(media_box=[False, 0, 15.36, 9.6]),
            lambda case: case["pdf_pages"][0]["draws"][1].update(raw_stream_sha256="0" * 64),
        ):
            case = copy.deepcopy(self.case)
            mutate(case)
            with self.assertRaises(rule.RuleError):
                self.compare(case=case)

    def test_reference_values_are_comparison_inputs_only(self) -> None:
        case = copy.deepcopy(self.case)
        case["pdf_pages"][0]["draws"][1]["pdf_ctm"][4] = 123.0
        result, report = self.compare(case=case)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(report["counts"]["discovery_draws_failing"], 1)
        self.assertNotEqual(result["pages"][0]["draws"][1]["pdf_ctm"][4], 123.0)

    def test_original_frame_hash_checksum_and_spool_failures_propagate(self) -> None:
        changed = bytearray(self.data)
        page = self.case["source_pages"][0]
        changed[page["text_offset"] + page["text_length"] - 1] ^= 1
        self.source.write_bytes(changed)
        with self.assertRaises(rule.RuleError):
            self.compare()
        forged_case = copy.deepcopy(self.case)
        forged_case["source_pages"][0]["text_sha256"] = sha(bytes(changed[page["text_offset"]:page["text_offset"] + page["text_length"]]))
        with self.assertRaises(rule.frame.FrameError):
            self.compare(case=forged_case)

    def test_source_fingerprint_is_sorted_pinned_and_checks_bytes_and_symlinks(self) -> None:
        root = self.root / "rust"
        (root / "crates/caj2pdf-core").mkdir(parents=True)
        for name in ("Cargo.toml", "Cargo.lock", "crates/caj2pdf-core/Cargo.toml", "rust-toolchain.toml"):
            (root / name).write_text("MIT original manifest\n")
        (root / "crates/caj2pdf-core/z.rs").write_text("// SPDX-License-Identifier: MIT\n")
        (root / "crates/caj2pdf-core/a.rs").write_text("// original invented source\n")
        first = rule.source_fingerprint(root)
        self.assertEqual(first["file_count"], 6)
        self.assertEqual([entry["path"] for entry in first["files"]], sorted(entry["path"] for entry in first["files"]))
        self.assertEqual(first, rule.source_fingerprint(root))
        (root / "crates/caj2pdf-core/a.rs").write_text("// changed MIT source\n")
        self.assertNotEqual(first["sha256"], rule.source_fingerprint(root)["sha256"])
        (root / "crates/caj2pdf-core/escape.rs").symlink_to(self.source)
        with self.assertRaises(rule.RuleError):
            rule.source_fingerprint(root)

    def _batch(self, stack: ExitStack) -> tuple[dict, Path, dict, str]:
        corpus = self.root / "corpus"
        corpus.mkdir(exist_ok=True)
        reference_root = self.root / "reference"
        reference_root.mkdir(exist_ok=True)
        tool = self.root / "invented-native"
        tool.write_text("#!/bin/sh\nexit 0\n")
        tool.chmod(0o700)
        report_path = self.root / "reference-report.json"
        report_path.write_text('{"status":"PASS"}\n')
        binaries = {name: {"sha256": value} for name, value in rule.reference.PINNED_HASHES.items()}
        environment = {"reference_revision": rule.reference.REFERENCE_REVISION, "reference_clean": True,
                       "pypdf2": {"version": "synthetic"}, "binaries": binaries,
                       "native_compiler_record": {"note": "invented"}, "environment": {},
                       "invocation": ["synthetic audit only"], "timeout_seconds": 180}
        cases, outputs, rows, baseline_pdfs, pdf_metadata, profiles = {}, [], [], {}, {}, []
        for original, variant in zip(rule.reference.PROFILES[:2], ("HN-A", "C8")):
            data, case, output = invented_fixture(variant, original.source_id)
            source = corpus / f"{original.name}.caj"
            source.write_bytes(data)
            rows.append({"id": original.source_id, "path": source.name, "size_bytes": len(data), "sha256": sha(data)})
            cases[original.name] = case
            outputs.append((output, len(output)))
            pdf_data = f"MIT invented PDF {original.name}".encode()
            profile = replace(original, expected_pages=2, expected_draws=3, expected_pdf_sha256=sha(pdf_data))
            profiles.append(profile)
            for run in (1, 2):
                path = self.root / f"{original.name}-{run}.pdf"
                path.write_bytes(pdf_data)
                baseline_pdfs[f"{original.name}-run{run}"] = path
                pdf_metadata[path] = {"pdf_sha256": sha(pdf_data), "page_count": 2, "draw_count": 3,
                                      "pages": case["pdf_pages"], "tools": binaries,
                                      "resources": {"max_tool_output_bytes": 12, "max_child_rss_kib": 30}}
        profiles.append(rule.reference.PROFILES[2])
        cases["hn_b"] = {"source_id": profiles[2].source_id, "source_pages": [{"page_number": index} for index in range(1, 7)]}
        for run in (1, 2):
            path = self.root / f"hn_b-{run}.pdf"
            path.write_bytes(b"MIT invented HN-B PDF")
            baseline_pdfs[f"hn_b-run{run}"] = path
        rows.extend({"id": f"invented-{index}", "path": "ignored", "size_bytes": 1, "sha256": "1" * 64}
                    for index in range(25))
        audit = [{"id": row["id"], "size_bytes": row["size_bytes"], "sha256": row["sha256"]} for row in rows]
        provenance = {"sha256": "a" * 64, "file_count": 1, "total_bytes": 20,
                      "algorithm": "invented stub", "files": []}
        stack.enter_context(patch.object(rule, "source_fingerprint", return_value=provenance))
        stack.enter_context(patch.object(rule.reference, "PROFILES", tuple(profiles)))
        stack.enter_context(patch.object(rule.reference, "load_source_rows", return_value=(rule.reference.MATRIX_SHA256, rows)))
        source_audit = stack.enter_context(patch.object(rule.reference, "audit_sources", return_value=audit))
        environment_audit = stack.enter_context(patch.object(rule.reference, "audit_environment", return_value=environment))
        stack.enter_context(patch.object(rule.shared, "_load_oracle", return_value=cases))
        stack.enter_context(patch.object(rule.shared, "_read_reference_report", return_value=({"status": "PASS"}, baseline_pdfs)))
        stack.enter_context(patch.object(rule.reference, "pdf_metadata", side_effect=lambda path, _: pdf_metadata[path]))
        native_run = stack.enter_context(patch.object(rule.pdf, "_run", side_effect=outputs))
        stack.enter_context(patch.object(rule, "EXPECTED_COMPARISONS", {
            "page_boxes": 4, "first_draws": 4, "discovery_draws": 2, "validation_draws": 0}))
        paths = {name: self.root / name for name in rule.REQUIRED_PATHS}
        paths.update(corpus=corpus, reference_repo=reference_root, native_tool=tool)
        mocks = {"run": native_run, "sources": source_audit, "environment": environment_audit,
                 "outputs": outputs, "provenance": provenance}
        return paths, report_path, mocks, sha(tool.read_bytes())

    def test_opt_in_batch_compares_all_counts_and_audits_without_converter_calls(self) -> None:
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            converter = stack.enter_context(patch.object(rule.reference, "run_converter", side_effect=AssertionError("converter forbidden")))
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "PASS", report["errors"])
            self.assertEqual(report["counts"]["native_launches"], 2)
            self.assertEqual(report["counts"]["native_processes_completed"], 2)
            self.assertEqual(report["counts"]["converter_launches"], 0)
            self.assertEqual(report["counts"]["private_source_checks_before"], 27)
            self.assertEqual(report["counts"]["private_source_checks_after"], 27)
            self.assertEqual(report["counts"]["baseline_pdf_checks_before"], 6)
            self.assertEqual(report["counts"]["baseline_pdf_checks_after"], 6)
            self.assertEqual(report["counts"]["unsupported_source_rows"], 6)
            self.assertEqual(report["counts"]["first_draws_passing"], 4)
            self.assertEqual(report["counts"]["discovery_draws_passing"], 2)
            for key in ("source_audit", "environment_audit", "input_audit", "native_audit"):
                self.assertEqual(report[key]["status"], "PASS")
            converter.assert_not_called()
            for call in mocks["run"].call_args_list:
                command, _, limits, _, cap = call.args
                self.assertEqual(len(command), 2)
                self.assertEqual(command[0], str(paths["native_tool"]))
                self.assertTrue(Path(command[1]).is_relative_to(paths["corpus"]))
                self.assertEqual(cap, rule.MAX_OUTPUT_BYTES)
                self.assertEqual(limits.timeout_seconds, 30)
                self.assertEqual(limits.max_child_virtual_bytes, 1024**3)

    def test_native_protocol_failure_counts_actual_launches_and_completes_post_audits(self) -> None:
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            mocks["run"].side_effect = [(b"H\tHN-A\t2\n", 9), mocks["outputs"][1]]
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["counts"]["native_launches"], 2)
            self.assertEqual(report["counts"]["profiles_completed"], 1)
            self.assertEqual(report["counts"]["profiles_passing"], 1)
            self.assertEqual(report["counts"]["profiles_failing"], 1)
            self.assertEqual(report["counts"]["first_draws_skipped"], 2)
            self.assertEqual(mocks["sources"].call_count, 2)
            self.assertEqual(mocks["environment"].call_count, 2)
            self.assertEqual(report["native_audit"]["status"], "PASS")
            self.assertEqual(report["resources"]["max_source_hash_read_request_bytes"], rule.reference.READ_CHUNK)

    def test_geometry_failure_remains_failure_after_successful_audits(self) -> None:
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            output = change_field(mocks["outputs"][0][0], 4, 17, "99.0")
            mocks["run"].side_effect = [(output, len(output)), mocks["outputs"][1]]
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["counts"]["profiles_completed"], 2)
            self.assertEqual(report["counts"]["discovery_draws_attempted"], 2)
            self.assertEqual(report["counts"]["discovery_draws_failing"], 1)
            self.assertEqual(report["native_audit"]["status"], "PASS")
            self.assertEqual(report["placement_rule_status"], "UNKNOWN_FAILED_RULE_GATE")

    def test_changed_native_sources_environment_and_metadata_invalidate_results(self) -> None:
        for changed in ("native", "provenance", "sources", "environment", "pdf"):
            with self.subTest(changed=changed), ExitStack() as stack:
                paths, report_path, mocks, binary_sha = self._batch(stack)
                original_outputs = list(mocks["outputs"])
                launches = 0

                def launch(*_args):
                    nonlocal launches
                    output = original_outputs[launches]
                    launches += 1
                    if launches == 2:
                        if changed == "native":
                            paths["native_tool"].write_text("#!/bin/sh\nexit 1\n")
                        elif changed == "provenance":
                            mocks["provenance"]["sha256"] = "b" * 64
                        elif changed == "sources":
                            mocks["sources"].side_effect = rule.reference.ReferenceError("original source changed")
                        elif changed == "environment":
                            mocks["environment"].side_effect = rule.reference.ReferenceError("reference tool changed")
                        else:
                            (self.root / "c8-2.pdf").write_bytes(b"changed PDF")
                    return output

                mocks["run"].side_effect = launch
                report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
                self.assertEqual(report["status"], "FAIL")
                self.assertEqual(report["counts"]["native_launches"], 2)
                self.assertEqual(report["counts"]["profiles_passing"], 2)
                self.assertEqual(report["placement_rule_status"], "UNKNOWN_FAILED_RULE_GATE")
                self.assertTrue(any(error.startswith("post-run") for error in report["errors"]))
                if changed != "pdf":
                    self.assertEqual(report["input_audit"]["status"], "PASS")

    def test_child_errors_are_counted_and_parsed_resources_survive_identity_failure(self) -> None:
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            mocks["run"].side_effect = rule.pdf.PdfMetadataError("invented child failed")
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["counts"]["native_launches"], 2)
            self.assertEqual(report["counts"]["native_processes_completed"], 0)
            self.assertEqual(report["counts"]["profiles_failing"], 2)
            self.assertEqual(report["native_audit"]["status"], "PASS")
            self.assertEqual(report["status"], "FAIL")
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            output = change_field(mocks["outputs"][0][0], 3, 12, "0" * 64)
            mocks["run"].side_effect = [(output, len(output)), mocks["outputs"][1]]
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["resources"]["native_source_read_bytes"], 8192)
            self.assertEqual(report["resources"]["max_native_working_memory_bytes"], 136192)
            self.assertEqual(report["profiles"][0]["native_metadata"]["draw_count"], 3)

    def test_missing_explicit_paths_or_bad_pins_fail_without_native_work(self) -> None:
        for arguments in (({}, None, None, None), (None, self.source, None, None),
                          (None, None, "a" * 64, None)):
            paths, reference_report, binary, provenance = arguments
            report = rule.run(paths, reference_report, native_sha256=binary, native_source_sha256=provenance)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["counts"]["native_launches"], 0)
            self.assertEqual(report["counts"]["profiles_skipped"], 2)
        with ExitStack() as stack:
            paths, report_path, mocks, _ = self._batch(stack)
            report = rule.run(paths, report_path, native_sha256="0" * 64, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["native_audit"]["status"], "FAIL")
            mocks["run"].assert_not_called()

    def test_external_baseline_setup_failure_still_reaudits_available_inputs(self) -> None:
        with ExitStack() as stack:
            paths, report_path, mocks, binary_sha = self._batch(stack)
            stack.enter_context(patch.object(rule.shared, "_read_reference_report",
                                             side_effect=rule.shared.ProbeError("baseline missing")))
            report = rule.run(paths, report_path, native_sha256=binary_sha, native_source_sha256="a" * 64)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["counts"]["native_launches"], 0)
            self.assertEqual(mocks["sources"].call_count, 2)
            self.assertEqual(mocks["environment"].call_count, 2)
            self.assertEqual(report["source_audit"]["status"], "PASS")
            self.assertEqual(report["native_audit"]["status"], "PASS")

    def test_reused_subprocess_cap_error_and_timeout_are_bounded(self) -> None:
        scripts = {
            "output": "import sys\nsys.stdout.buffer.write(b'x' * (1024*1024 + 1))\n",
            "failure": "import sys\nprint('invented diagnostic', file=sys.stderr)\nsys.exit(4)\n",
            "timeout": "import time\ntime.sleep(2)\n",
        }
        for name, code in scripts.items():
            tool = self.root / f"{name}.py"
            tool.write_text(code)
            limits = rule.pdf.PdfMetadataLimits(timeout_seconds=0.1 if name == "timeout" else 5)
            usage = rule.pdf._Usage()
            with self.subTest(name=name), self.assertRaises(rule.pdf.PdfMetadataError):
                rule.pdf._run([sys.executable, str(tool)], "invented native", limits,
                              usage, rule.MAX_OUTPUT_BYTES)
            self.assertEqual(usage.total_tool_output_bytes, 0)

    def test_cli_clean_clone_and_partial_arguments_return_structured_status(self) -> None:
        for arguments, expected_code, expected_status in ((["--json"], 0, "NOT_RUN"),
                                                         (["--json", "--native-tool", str(self.source)], 1, "FAIL")):
            output = io.StringIO()
            with redirect_stdout(output):
                code = rule.main(arguments)
            self.assertEqual(code, expected_code)
            self.assertEqual(json.loads(output.getvalue())["status"], expected_status)


if __name__ == "__main__":
    unittest.main()
