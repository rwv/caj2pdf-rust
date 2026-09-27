# SPDX-License-Identifier: MIT
"""Synthetic protocol and evidence-state tests for optional full-page parity."""

from __future__ import annotations

import contextlib
from contextlib import ExitStack
import hashlib
from io import StringIO
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "tests" / "conformance"))
import jbig2_page_parity as parity  # noqa: E402
import jbig2_oracle as full  # noqa: E402
from test_jbig2_generic_oracle import synthetic_case  # noqa: E402


EMPTY_PRIVATE_ENV = {
    "CAJ2PDF_CORPUS_DIR": "",
    "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
}
COUNT_KEYS = ("attempted", "completed", "matching", "failing", "skipped", "unsupported")


def invoke(*arguments: str) -> tuple[int, dict]:
    output = StringIO()
    with mock.patch.dict(os.environ, EMPTY_PRIVATE_ENV), contextlib.redirect_stdout(output):
        exit_code = parity.main([*arguments, "--json"])
    return exit_code, json.loads(output.getvalue())


def selected() -> list[dict]:
    cases = [{
        "coordinate": ("synthetic/standard.caj", number, 1),
        "classification": parity.instances.text_oracle.STANDARD,
        "instances": 1, "flags_offset": 1234,
        "width": 8, "height": 2,
        "pixel_sha256": "a" * 64, "black_pixels": 1,
    } for number in range(1, parity.EXPECTED)]
    cases.append({
        "coordinate": ("synthetic/anomaly.caj", 11, 1),
        "classification": parity.instances.text_oracle.ANOMALY,
        "instances": 1, "flags_offset": 1234,
        "width": 8, "height": 2,
        "pixel_sha256": "b" * 64, "black_pixels": 1,
    })
    return cases


def complete_line(case: dict, *, pixel_sha: str | None = None,
                  marker: str = "-") -> str:
    sample_id, page, image = case["coordinate"]
    fields = (
        "CASE", sample_id, page, image, "COMPLETE", case["width"], case["height"],
        case["height"], 2, pixel_sha or case["pixel_sha256"],
        case["black_pixels"], 16, 8, 4096, 2, 1024, "-", 0, "Complete", marker,
    )
    return "\t".join(str(field) for field in fields)


def strict_lines(cases: list[dict]) -> list[str]:
    lines = [complete_line(case) for case in cases[:-1]]
    sample_id, page, image = cases[-1]["coordinate"]
    lines.append("\t".join(str(field) for field in (
        "CASE", sample_id, page, image, "HEADER_REFUSED", 8, 2, 0, 0,
        parity.EMPTY_SHA256, 0, 16, 8, 4096, 0, 1024,
        "malformed_text_header", cases[-1]["flags_offset"], "Header", "-",
    )))
    return lines


def parse_lines(lines: list[str], cases: list[dict], policy: str = "strict") -> tuple:
    return parity.parse_output("\n".join([*lines, f"TOTAL\t{len(cases)}"]), cases, policy)


class PageParityTests(unittest.TestCase):
    def test_clean_clone_both_policies_report_not_run_with_zero_cases(self) -> None:
        for policy in ("strict", "hn-c8-unused-refinement-template"):
            with self.subTest(policy=policy):
                code, report = invoke("--text-header-policy", policy)
                self.assertEqual(code, 0)
                self.assertEqual(report["status"], "NOT_RUN")
                self.assertEqual(report["text_header_policy"], policy)
                for key in ("compatibility", "opt_in_anomaly"):
                    group = report[key]
                    self.assertEqual(group["status"], "NOT_RUN")
                    self.assertEqual({count: group[count] for count in COUNT_KEYS},
                                     dict.fromkeys(COUNT_KEYS, 0))
                self.assertEqual(report["strict_header_refusals"], 0)
                self.assertEqual(report["submitted_cases"], 0)
                self.assertEqual(report["source_hashes_before"], {})
                self.assertEqual(report["source_hashes_after"], {})
                self.assertIsNone(report["table_sha256_before"])
                self.assertIsNone(report["table_sha256_after"])
                self.assertIsNone(report["rust_binary_sha256_before"])
                self.assertIsNone(report["rust_binary_sha256_after"])

    def test_requested_missing_or_bad_private_inputs_fail(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            bad_table = root / "bad-table.fixture"
            bad_table.write_bytes(b"invented invalid fixture, without table rows")
            variants = (
                ("--corpus-dir", str(root)),
                ("--table-fixture", str(root / "missing-table.fixture")),
                ("--table-fixture", str(bad_table)),
                ("--corpus-dir", str(root), "--table-fixture", str(bad_table)),
                ("--corpus-dir", str(root / "missing-corpus"),
                 "--table-fixture", str(bad_table)),
            )
            for arguments in variants:
                with self.subTest(arguments=arguments):
                    code, report = invoke(*arguments)
                    self.assertEqual((code, report["status"]), (1, "FAIL"))
                    self.assertEqual(report["compatibility"]["matching"], 0)
                    self.assertEqual(report["opt_in_anomaly"]["matching"], 0)

    def test_changed_full_page_pixel_manifest_fails_even_without_private_inputs(self) -> None:
        manifest_path = full.DEFAULT_MANIFEST
        self.assertEqual(full.sha256_file(manifest_path),
                         "bda920020d111e200352e7a38d8ec54c0be6c2e0a6b1c87c6ac55aac861c9bbe")
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            changed = Path(temporary) / "changed-page-oracle.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            image = manifest["samples"][0]["images"][0]
            original = image["normalized_pixel_sha256"]
            image["normalized_pixel_sha256"] = "f" * 64 if original != "f" * 64 else "e" * 64
            changed.write_text(json.dumps(manifest), encoding="utf-8")
            code, report = invoke("--manifest", str(changed))
        self.assertEqual((code, report["status"]), (1, "FAIL"))
        self.assertEqual(report["compatibility"]["matching"], 0)
        self.assertEqual(report["opt_in_anomaly"]["matching"], 0)
        self.assertEqual(report["source_hashes_before"], {})

    def test_strict_full_page_matches_and_header_refusal_are_separate(self) -> None:
        cases = selected()
        compatibility, strict, anomaly, resources = parse_lines(strict_lines(cases), cases)
        self.assertEqual(compatibility["status"], "PASS")
        self.assertEqual({key: compatibility[key] for key in COUNT_KEYS}, {
            "attempted": 545, "completed": 545, "matching": 545,
            "failing": 0, "skipped": 0, "unsupported": 0,
        })
        self.assertEqual(strict, 1)
        self.assertEqual(anomaly["status"], "NOT_RUN")
        self.assertEqual(anomaly["attempted"], 0)
        self.assertEqual(resources["max_request_bytes"], 8)
        self.assertEqual(resources["peak_scratch_bytes"], 2)
        self.assertEqual(resources["peak_output_bytes"], 2)
        self.assertTrue(resources["large_page"]["pixel_match"])

    def test_opt_in_pixel_match_and_mismatch_are_labelled_separately(self) -> None:
        cases = selected()
        lines = [complete_line(case) for case in cases[:-1]]
        lines.append(complete_line(cases[-1], marker=parity.ANOMALY_MARKER))
        standard, strict, anomaly, _ = parse_lines(lines, cases, parity.HN_C8_POLICY)
        self.assertEqual((standard["status"], standard["matching"]), ("PASS", 545))
        self.assertEqual(strict, 0)
        self.assertEqual((anomaly["status"], anomaly["attempted"], anomaly["matching"]),
                         ("PASS", 1, 1))
        self.assertEqual(anomaly["case"]["anomaly_marker"], parity.ANOMALY_MARKER)

        lines[-1] = complete_line(cases[-1], pixel_sha="c" * 64,
                                  marker=parity.ANOMALY_MARKER)
        standard, _, anomaly, _ = parse_lines(lines, cases, parity.HN_C8_POLICY)
        self.assertEqual((standard["status"], standard["matching"]), ("PASS", 545))
        self.assertEqual((anomaly["status"], anomaly["failing"]), ("FAIL", 1))
        self.assertEqual(anomaly["first_failure"]["kind"], "pixel_mismatch")
        self.assertEqual(anomaly["first_failure"]["expected_sha256"], "b" * 64)

    def test_standard_pixel_mismatch_and_typed_unsupported_refusal_are_preserved(self) -> None:
        cases = selected()
        lines = strict_lines(cases)
        lines[0] = complete_line(cases[0], pixel_sha="c" * 64)
        standard, strict, _, _ = parse_lines(lines, cases)
        self.assertEqual((standard["completed"], standard["matching"], standard["failing"]),
                         (545, 544, 1))
        self.assertEqual(standard["first_failure"]["kind"], "pixel_mismatch")
        self.assertEqual(standard["first_failure"]["coordinate"], cases[0]["coordinate"])
        self.assertEqual(strict, 1)

        fields = complete_line(cases[1]).split("\t")
        fields[4] = "REFUSED"
        fields[7:11] = ["0", "0", parity.EMPTY_SHA256, "0"]
        fields[14] = "0"
        fields[16:19] = ["unsupported_segment_profile", "42", "Header"]
        lines = strict_lines(cases)
        lines[1] = "\t".join(fields)
        standard, strict, _, _ = parse_lines(lines, cases)
        self.assertEqual((standard["completed"], standard["matching"],
                          standard["failing"], standard["unsupported"]),
                         (544, 544, 1, 1))
        self.assertEqual(standard["first_failure"], {
            "coordinate": cases[1]["coordinate"], "status": "REFUSED",
            "kind": "unsupported_segment_profile", "source_byte_offset": 42,
            "stage": "Header",
        })
        self.assertEqual(strict, 1)

    def test_early_source_and_directory_refusals_keep_typed_location(self) -> None:
        cases = selected()
        for stage, kind in (("Source", "source_read_error"),
                            ("Directory", "unsupported_segment_profile"),
                            ("PageInfo", "unsupported_page_profile")):
            with self.subTest(stage=stage):
                lines = strict_lines(cases)
                fields = lines[0].split("\t")
                fields[4] = "REFUSED"
                fields[5:11] = ["0", "0", "0", "0", parity.EMPTY_SHA256, "0"]
                fields[11:16] = ["0", "0", "0", "0", "0"]
                fields[16:19] = [kind, "42", stage]
                lines[0] = "\t".join(fields)
                standard, strict, _, _ = parse_lines(lines, cases)
                self.assertEqual(standard["first_failure"], {
                    "coordinate": cases[0]["coordinate"], "status": "REFUSED",
                    "kind": kind, "source_byte_offset": 42, "stage": stage,
                })
                self.assertEqual((standard["attempted"], standard["completed"],
                                  standard["matching"], standard["failing"]),
                                 (545, 544, 544, 1))
                self.assertEqual(strict, 1)

    def test_malformed_case_and_total_protocol_cannot_claim_parity(self) -> None:
        cases = selected()
        original = strict_lines(cases)
        variants = []
        shortened = original.copy()
        shortened[0] = "\t".join(original[0].split("\t")[:-1])
        variants.append((shortened, "TOTAL\t546"))
        negative = original.copy()
        fields = negative[0].split("\t")
        fields[12] = "-1"
        negative[0] = "\t".join(fields)
        variants.append((negative, "TOTAL\t546"))
        oversized = original.copy()
        fields = oversized[0].split("\t")
        fields[8] = "3"
        oversized[0] = "\t".join(fields)
        variants.append((oversized, "TOTAL\t546"))
        wrong_order = original.copy()
        fields = wrong_order[0].split("\t")
        fields[2] = "546"
        wrong_order[0] = "\t".join(fields)
        variants.append((wrong_order, "TOTAL\t546"))
        variants.append((original[:-1], "TOTAL\t546"))
        variants.append((original, "TOTAL\t545"))
        for lines, total in variants:
            with self.subTest(first=lines[0][:64], total=total), self.assertRaises(parity.ParityError):
                parity.parse_output("\n".join([*lines, total]), cases)

    def test_resource_limits_reject_impossible_rust_metrics(self) -> None:
        cases = selected()
        limits = (
            (11, parity.MAX_SOURCE_BYTES),
            (12, parity.MAX_REQUEST_BYTES),
            (13, parity.MAX_RESIDENT_BYTES),
            (14, parity.MAX_SCRATCH_BYTES),
            (15, parity.MAX_RSS_KIB),
        )
        for field, ceiling in limits:
            with self.subTest(field=field):
                lines = strict_lines(cases)
                fields = lines[0].split("\t")
                fields[field] = str(ceiling + 1)
                lines[0] = "\t".join(fields)
                with self.assertRaisesRegex(parity.ParityError, "impossible page metrics"):
                    parse_lines(lines, cases)
        for field in (11, 12, 13, 15):
            with self.subTest(zero_complete_field=field):
                lines = strict_lines(cases)
                fields = lines[0].split("\t")
                fields[field] = "0"
                lines[0] = "\t".join(fields)
                with self.assertRaisesRegex(parity.ParityError, "falsely reports completion"):
                    parse_lines(lines, cases)

    def test_strict_header_cannot_be_reclassified_as_a_full_page_match(self) -> None:
        cases = selected()
        lines = strict_lines(cases)
        lines[-1] = complete_line(cases[-1])
        standard, strict, anomaly, _ = parse_lines(lines, cases)
        self.assertEqual((standard["matching"], strict), (545, 0))
        self.assertEqual(anomaly["first_failure"]["kind"], "strict_header_not_refused")

    def test_fresh_directory_rejects_extra_segment_bad_association_and_truncation(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            case = synthetic_case(root / "sample.caj")
            rows = [{"id": case["id"], "path": case["path"],
                     "sha256": case["source_sha256"], "variant": case["variant"]}]
            paths = {case["id"]: case["source_path"]}
            image = {key: case[key] for key in
                     ("page", "image", "offset", "length", "width", "height", "segments")}
            inventory = {"status": "PASS", "expected_images": 1, "checked_images": 1,
                         "samples": [{"id": case["id"], "path": case["path"],
                                      "source_sha256": case["source_sha256"],
                                      "variant": case["variant"], "images": [image]}]}
            with mock.patch.object(full, "EXPECTED_IMAGES", 1), mock.patch.object(
                full, "EXPECTED_TYPE3", {case["id"]: 1}
            ):
                self.assertEqual(len(full.checked_cases(inventory, rows, paths)), 1)
                image["segments"] = [*case["segments"], dict(case["segments"][-1])]
                with self.assertRaisesRegex(full.OracleError, "expected five JBIG2 segments"):
                    full.checked_cases(inventory, rows, paths)
                image["segments"] = [dict(segment) for segment in case["segments"]]
                image["segments"][4]["page_association"] = 2
                with self.assertRaisesRegex(full.OracleError, "unexpected segment 4 profile"):
                    full.checked_cases(inventory, rows, paths)
                image["segments"][4]["page_association"] = 1
                case["source_path"].write_bytes(case["source_path"].read_bytes()[:-1])
                with self.assertRaisesRegex(full.OracleError, "image span is invalid"):
                    full.checked_cases(inventory, rows, paths)

    def test_all_27_invented_source_hashes_reject_changed_and_truncated_bytes(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            rows = []
            for index in range(27):
                path = root / f"invented-{index:02d}.caj"
                data = f"synthetic page source {index:02d}".encode()
                path.write_bytes(data)
                rows.append({
                    "id": path.name, "path": path.name, "variant": "HN",
                    "size_bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                })
            with mock.patch.object(full.conformance, "load_matrix", return_value=rows):
                checked, paths = full.validate_sources(full.DEFAULT_MATRIX, root)
                self.assertEqual(len(checked), 27)
                self.assertEqual(len(paths), 27)

                changed = root / rows[0]["path"]
                changed.write_bytes(b"X" * rows[0]["size_bytes"])
                with self.assertRaisesRegex(full.OracleError, "source SHA-256 differs"):
                    full.validate_sources(full.DEFAULT_MATRIX, root)
                changed.write_bytes(f"synthetic page source {0:02d}".encode())

                truncated = root / rows[-1]["path"]
                truncated.write_bytes(truncated.read_bytes()[:-1])
                with self.assertRaisesRegex(full.OracleError, "source size differs"):
                    full.validate_sources(full.DEFAULT_MATRIX, root)

    def test_plan_adds_exact_page_and_generic_spans_without_expected_pixels(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            cases = []
            baseline = {}
            for index in (1, 2):
                case = synthetic_case(root / f"source-{index}.caj")
                case["id"] = f"synthetic/source-{index}.caj"
                case["path"] = case["id"]
                case["coordinate"] = (case["id"], index, 1)
                case["page"] = index
                profile, record_sha = full.profile_and_hash(case)
                baseline[case["coordinate"]] = {
                    "offset": case["offset"], "length": case["length"],
                    "width": case["width"], "height": case["height"],
                    "profile": profile, "encoded_sha256": record_sha,
                    "normalized_pixel_sha256": "a" * 64,
                    "black_pixels": 1,
                }
                cases.append(case)
            control = [(cases[0]["coordinate"], parity.instances.text_oracle.STANDARD, 1, 42),
                       (cases[1]["coordinate"], parity.instances.text_oracle.ANOMALY, 1, 43)]
            base = "\t".join(f"base-{field}" for field in range(19))
            with mock.patch.object(parity, "EXPECTED", 2), mock.patch.object(
                parity, "STANDARD", 1
            ), mock.patch.object(parity.instances, "load_text_manifest", return_value={}), mock.patch.object(
                parity.instances, "plan_for_cases", return_value=([base, base], control)
            ), mock.patch.object(parity, "_oracle_images", return_value=baseline):
                lines, prepared = parity.plan_for_cases(cases, {}, {}, {})
                self.assertEqual(len(lines), 2)
                self.assertEqual(len(prepared), 2)
                for line, case in zip(lines, cases):
                    fields = line.split("\t")
                    self.assertEqual(len(fields), 28)
                    self.assertEqual(fields[:19], base.split("\t"))
                    self.assertEqual(fields[19:22], [str(case["offset"]),
                                                     str(case["length"]),
                                                     baseline[case["coordinate"]]["encoded_sha256"]])
                    self.assertNotIn("a" * 64, fields)
                self.assertEqual(prepared[1]["classification"],
                                 parity.instances.text_oracle.ANOMALY)

                source = cases[0]["source_path"]
                data = bytearray(source.read_bytes())
                data[-1] ^= 1
                source.write_bytes(data)
                with self.assertRaisesRegex(parity.ParityError,
                                            "encoded record or profile differs"):
                    parity.plan_for_cases(cases, {}, {}, {})

    def test_postchecks_record_actual_27_source_and_table_hashes_after_rust_failure(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            fixture = root / "private.fixture"
            fixture.write_bytes(b"invented placeholder")
            rows = [{"id": f"source-{index:02d}.caj", "sha256": f"{index:064x}"}
                    for index in range(27)]
            expected_hashes = {row["id"]: row["sha256"] for row in rows}
            report = parity.initial_report()
            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(parity, "load_manifest", return_value={}))
                stack.enter_context(mock.patch.object(parity.instances.parity, "check_table",
                                                      return_value=fixture))
                sources = stack.enter_context(mock.patch.object(parity.oracle,
                    "validate_sources", return_value=(rows, {})))
                stack.enter_context(mock.patch.object(parity.oracle, "sha256_file",
                    side_effect=lambda path: parity.instances.parity.TABLE_SHA
                    if path == fixture else "d" * 64))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "run_directory_inventory", return_value={}))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "checked_cases", return_value=[]))
                stack.enter_context(mock.patch.object(parity.instances.headers,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity.dictionaries,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity, "plan_for_cases",
                    return_value=(["synthetic TSV"] * parity.EXPECTED, [])))
                stack.enter_context(mock.patch.object(parity, "build_binary",
                                                      return_value=root / "binary"))
                stack.enter_context(mock.patch.object(parity, "run_binary",
                    side_effect=parity.ParityError("invented malformed Rust CASE")))
                with self.assertRaisesRegex(parity.ParityError, "invented malformed Rust CASE"):
                    parity.run(root, fixture, report=report)
            self.assertEqual(sources.call_count, 2)
            self.assertEqual(report["source_hashes_before"], expected_hashes)
            self.assertEqual(report["source_hashes_after"], expected_hashes)
            self.assertEqual(report["table_sha256_before"], parity.instances.parity.TABLE_SHA)
            self.assertEqual(report["table_sha256_after"], parity.instances.parity.TABLE_SHA)
            self.assertEqual(report["rust_binary_sha256_before"], "d" * 64)
            self.assertEqual(report["rust_binary_sha256_after"], "d" * 64)
            self.assertEqual(report["submitted_cases"], 546)
            self.assertEqual(report["compatibility"]["attempted"], 0)
            self.assertEqual(report["phase"], "rust_decode")

    def test_changed_source_and_table_postchecks_run_even_after_build_failure(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            fixture = root / "private.fixture"
            fixture.write_bytes(b"invented placeholder")
            rows = [{"id": f"source-{index:02d}.caj", "sha256": f"{index:064x}"}
                    for index in range(27)]
            report = parity.initial_report()
            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(parity, "load_manifest", return_value={}))
                table = stack.enter_context(mock.patch.object(parity.instances.parity,
                    "check_table", side_effect=[fixture,
                        parity.instances.parity.ParityError("changed table")]))
                sources = stack.enter_context(mock.patch.object(parity.oracle,
                    "validate_sources", side_effect=[(rows, {}),
                        full.OracleError("changed source")]))
                stack.enter_context(mock.patch.object(parity.oracle, "sha256_file",
                    side_effect=lambda path: parity.instances.parity.TABLE_SHA
                    if path == fixture else "d" * 64))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "run_directory_inventory", return_value={}))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "checked_cases", return_value=[]))
                stack.enter_context(mock.patch.object(parity.instances.headers,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity.dictionaries,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity, "plan_for_cases",
                                                      return_value=(["synthetic TSV"], [])))
                stack.enter_context(mock.patch.object(parity, "build_binary",
                    side_effect=parity.ParityError("invented build error")))
                decode = stack.enter_context(mock.patch.object(parity, "run_binary"))
                with self.assertRaisesRegex(parity.ParityError,
                                            "source postcheck: changed source; table postcheck: changed table"):
                    parity.run(root, fixture, report=report)
            self.assertEqual((sources.call_count, table.call_count), (2, 2))
            decode.assert_not_called()
            self.assertEqual(len(report["source_hashes_before"]), 27)
            self.assertEqual(report["source_hashes_after"], {})
            self.assertEqual(report["table_sha256_before"], parity.instances.parity.TABLE_SHA)
            self.assertIsNone(report["table_sha256_after"])
            self.assertIsNone(report["rust_binary_sha256_before"])
            self.assertIsNone(report["rust_binary_sha256_after"])
            self.assertEqual(report["submitted_cases"], 0)
            self.assertEqual((report["status"], report["compatibility"]["status"]),
                             ("FAIL", "FAIL"))

    def test_opt_in_post_audit_failure_invalidates_anomaly_pass(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            fixture = root / "private.fixture"
            fixture.write_bytes(b"invented placeholder")
            rows = [{"id": f"source-{index:02d}.caj", "sha256": f"{index:064x}"}
                    for index in range(27)]
            report = parity.initial_report(parity.HN_C8_POLICY)
            parsed = parity.initial_report(parity.HN_C8_POLICY)
            parsed["compatibility"].update(status="PASS", attempted=545,
                                           completed=545, matching=545)
            parsed["opt_in_anomaly"].update(status="PASS", attempted=1,
                                            completed=1, matching=1)
            table_reads = 0

            def changed_table_hash(path: Path) -> str:
                nonlocal table_reads
                if path == fixture:
                    table_reads += 1
                    return (parity.instances.parity.TABLE_SHA if table_reads == 1
                            else "e" * 64)
                return "d" * 64

            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(parity, "load_manifest", return_value={}))
                stack.enter_context(mock.patch.object(parity.instances.parity,
                    "check_table", return_value=fixture))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "validate_sources", return_value=(rows, {})))
                stack.enter_context(mock.patch.object(parity.oracle, "sha256_file",
                                                      side_effect=changed_table_hash))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "run_directory_inventory", return_value={}))
                stack.enter_context(mock.patch.object(parity.oracle,
                    "checked_cases", return_value=[]))
                stack.enter_context(mock.patch.object(parity.instances.headers,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity.dictionaries,
                    "load_baseline", return_value=([], {})))
                stack.enter_context(mock.patch.object(parity, "plan_for_cases",
                                                      return_value=(["synthetic TSV"], [])))
                stack.enter_context(mock.patch.object(parity, "build_binary",
                                                      return_value=root / "binary"))
                stack.enter_context(mock.patch.object(parity, "run_binary", return_value=(
                    parsed["compatibility"], 0, parsed["opt_in_anomaly"], parsed["resources"])))
                with self.assertRaisesRegex(parity.ParityError,
                                            "private T.88 state table changed during the run"):
                    parity.run(root, fixture, report=report, policy=parity.HN_C8_POLICY)
            self.assertEqual((report["status"], report["compatibility"]["status"],
                              report["opt_in_anomaly"]["status"]),
                             ("FAIL", "FAIL", "FAIL"))
            self.assertEqual(report["source_hashes_before"], report["source_hashes_after"])
            self.assertEqual(len(report["source_hashes_after"]), 27)
            self.assertEqual((report["table_sha256_before"], report["table_sha256_after"]),
                             (parity.instances.parity.TABLE_SHA, "e" * 64))

    def test_private_plan_is_removed_when_rust_decode_fails(self) -> None:
        observed: list[Path] = []

        def fail_decode(command: list[str]) -> str:
            plan = Path(command[2])
            self.assertTrue(plan.is_file())
            self.assertEqual(plan.read_text(encoding="utf-8"), "synthetic plan\n")
            observed.append(plan)
            raise parity.ParityError("invented decoder failure")

        with mock.patch.object(parity.instances, "bounded_decode", side_effect=fail_decode):
            with self.assertRaisesRegex(parity.ParityError, "invented decoder failure"):
                parity.run_binary(Path("/tmp/invented-binary"), Path("/tmp/invented-table"),
                                  ["synthetic plan"], [], parity.STRICT_POLICY)
        self.assertEqual(len(observed), 1)
        self.assertFalse(observed[0].exists())
        self.assertFalse(observed[0].parent.exists())


if __name__ == "__main__":
    unittest.main()
