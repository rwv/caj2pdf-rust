# SPDX-License-Identifier: MIT
"""Original synthetic checks of the bounded unsigned-coordinate intervention."""

from contextlib import ExitStack, contextmanager, redirect_stdout
from copy import deepcopy
from dataclasses import replace
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zlib


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import hnc8_text_content as content  # noqa: E402
from hnc8_layout_source import FileInput, SourceExtractor  # noqa: E402
import test_hnc8_text_content as donor_tests  # noqa: E402
import test_hnc8_text_field as field_tests  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class TextHighbitTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.original, self.base = field_tests.synthetic_source()
        self.source = self.root / "synthetic.caj"
        self.source.write_bytes(self.original)
        profiles = patch.dict(content.text_frame.PROFILES, {
            "c8": content.text_frame.FrameProfile(
                "synthetic", digest(b"MIT synthetic header"), digest(b"A0B1C2"))})
        profiles.start()
        self.addCleanup(profiles.stop)
        runtime = patch.object(content, "ZLIB_RUNTIME_VERSION", zlib.ZLIB_RUNTIME_VERSION)
        runtime.start()
        self.addCleanup(runtime.stop)

    def probes(self) -> tuple:
        return (
            replace(self.base, name="synthetic-x-bit15", new_value=393 ^ 0x8000,
                    field_operation="toggle-bit15"),
            replace(self.base, name="synthetic-y-bit15", field_offset=74, axis="y",
                    original_value=435, new_value=435 ^ 0x8000,
                    field_operation="toggle-bit15"),
            replace(self.base, name="synthetic-y-boundary", field_offset=74, axis="y",
                    original_value=435, new_value=32768, field_operation="boundary32768"),
        )

    def case(self) -> dict:
        with FileInput(self.source) as source:
            page = SourceExtractor(source, "synthetic.caj").read_page(1)
        return {"source_pages": [{key: page[key] for key in (
            "images", "page_number", "text_length", "text_offset", "text_sha256")}]}

    def copy_control(self, probe, name: str) -> dict:
        return content.copy_content(
            self.source, self.root / name, probe, digest(self.original),
            self.case()["source_pages"][0]["text_sha256"], None, 2)

    def pdf(self, probe, *, moved: bool, signed: bool = False) -> dict:
        result = donor_tests.pdf_pages()
        value = probe.new_value if moved else probe.original_value
        if moved and signed:
            value -= 65536
        coordinate = value * content.COORDINATE_SCALE
        component = 4 if probe.axis == "x" else 5
        if component == 5:
            coordinate = 200 - coordinate
        result["pages"][0]["draws"][1]["pdf_ctm"][component] = round(coordinate, 4)
        return result

    def test_clean_highbit_cli_never_audits_runtime_or_private_files(self) -> None:
        with patch.object(content, "audit_zlib_runtime", side_effect=AssertionError("runtime access")), \
                patch.object(Path, "open", side_effect=AssertionError("file access")), \
                patch.object(Path, "resolve", side_effect=AssertionError("path access")):
            report = content.run(batch="highbit")
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(content.main(["--batch", "highbit", "--json"]), 0)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["counts"]["probes_planned"], 4)
        self.assertTrue(all(value == 0 for key, value in report["counts"].items()
                            if key != "probes_planned"))
        self.assertEqual(report["zlib_audit"], {
            "status": "NOT_RUN", "before_checked": 0, "after_checked": 0})
        self.assertEqual(json.loads(output.getvalue()), report)

    def test_four_frozen_cases_match_fields_operations_recipes_and_source_hashes(self) -> None:
        self.assertEqual([
            (p.name, p.field_offset, p.original_value, p.new_value, p.field_operation,
             p.level, p.mem_level, p.strategy, p.expected_mutated_source_sha256)
            for p in content.HIGHBIT_PROBES
        ], [
            ("c8-x-highbit", 33576, 5978, 38746, "toggle-bit15", 9, 8, 0,
             "0f15353f8ef1d7d7e5badc332f65be860ff05cc23115bfab484073c6a50b672d"),
            ("c8-y-boundary32768", 33578, 1479, 32768, "boundary32768", 9, 8, 0,
             "73c3c70fcd8edcde47bf5833289f3faf01b7355b1653561c261be38a6fc25a75"),
            ("hn-a-x-highbit", 17048, 482, 33250, "toggle-bit15", 8, 7, 0,
             "b691aee68b4c5e26a0ace026ab86d7762f096322f2d69252d5d5958725ae6130"),
            ("hn-a-y-highbit", 17050, 5446, 38214, "toggle-bit15", 8, 7, 0,
             "8ed08a34876ea5b40959b429f329ebbc787fd8490f1c4f23fbc519b19adcef31"),
        ])
        for probe in content.HIGHBIT_PROBES:
            content._check_plan(probe)
            self.assertEqual(probe.target_offset + probe.target_length, probe.first_descriptor)
            self.assertEqual(probe.chunk_size, content.MAX_INFLATED_BYTES)
            self.assertEqual(probe.image_number, 2)
            self.assertEqual(probe.target_length - 24, 14522 if probe.profile == "c8" else 7477)
        self.assertEqual({p.field_operation for p in content.FIELD_PROBES}, {"add100"})

    def test_named_operations_reject_invalid_values_without_loosening_add100(self) -> None:
        x, _, boundary = self.probes()
        for invalid in (
            replace(self.base, new_value=494),
            replace(x, new_value=x.new_value + 1),
            replace(x, original_value=32768, new_value=32868),
            replace(x, new_value=65536),
            replace(x, original_value=True),
            replace(x, field_operation="toggle-any-bit"),
            replace(boundary, new_value=32769),
            replace(boundary, original_value=0),
            replace(content.PROBES[0], field_operation="toggle-bit15"),
        ):
            with self.subTest(invalid=invalid), self.assertRaises(content.ContentError):
                content._check_plan(invalid)
        content._check_plan(self.base)

    def test_exact_frames_change_one_logical_slot_and_preserve_row_images_and_source(self) -> None:
        for probe in self.probes():
            with self.subTest(probe=probe.name):
                record = self.copy_control(probe, probe.name)
                mutant = Path(record["path"])
                changed = mutant.read_bytes()
                self.assertEqual(self.source.read_bytes(), self.original)
                self.assertEqual(changed[:probe.target_offset + 24],
                                 self.original[:probe.target_offset + 24])
                self.assertEqual(changed[probe.first_descriptor:],
                                 self.original[probe.first_descriptor:])
                self.assertEqual(len(changed), len(self.original))
                plain, metadata = content._frame(
                    changed[probe.target_offset:probe.first_descriptor], image_count=2)
                expected = bytearray(field_tests.PLAIN)
                expected[probe.field_offset:probe.field_offset + 2] = probe.new_value.to_bytes(2, "little")
                self.assertEqual(plain, expected)
                self.assertEqual(metadata.decoded_length, len(field_tests.PLAIN))
                logical = record["logical_field"]
                self.assertEqual(logical["field_operation"], probe.field_operation)
                self.assertEqual(logical["delta"], probe.new_value - probe.original_value)
                self.assertTrue(logical["all_other_decoded_bytes_unchanged"])
                if probe.field_operation == "toggle-bit15":
                    self.assertEqual(logical["decoded_changed_positions"], [probe.field_offset + 1])
                    self.assertTrue(logical["single_bit15_change"])
                else:
                    self.assertEqual(logical["decoded_changed_positions"],
                                     [probe.field_offset, probe.field_offset + 1])
                    self.assertFalse(logical["single_bit15_change"])
                content._check_source(mutant, "synthetic.caj", self.case(), probe,
                                      mutated_text_sha256=record["mutated_text_sha256"])

    def test_highbit_compression_is_one_shot_and_requires_exact_frame_and_frozen_hash(self) -> None:
        probe = self.probes()[0]
        original_compressobj = zlib.compressobj
        flushes = []

        class Compressor:
            def __init__(self, *args):
                self.inner = original_compressobj(*args)

            def compress(self, data):
                return self.inner.compress(data)

            def flush(self, mode):
                flushes.append(mode)
                return self.inner.flush(mode)

        with patch.object(content.zlib, "compressobj", side_effect=Compressor) as factory:
            self.copy_control(probe, "one-shot")
        factory.assert_called_once_with(9, zlib.DEFLATED, 15, 8, zlib.Z_DEFAULT_STRATEGY)
        self.assertEqual(flushes, [zlib.Z_FINISH])
        modified = bytearray(field_tests.PLAIN)
        modified[72:74] = probe.new_value.to_bytes(2, "little")
        with self.assertRaises(content.ContentError):
            content._compressed(modified, replace(probe, target_length=probe.target_length + 1))
        with self.assertRaisesRegex(content.ContentError, "predeclared SHA"):
            self.copy_control(replace(probe, expected_mutated_source_sha256="0" * 64), "wrong-pin")
        with self.assertRaises(content.ContentError):
            self.copy_control(replace(probe, field_offset=76), "wrong-slot")

    def test_unsigned_negative_offpage_geometry_excludes_signed_alternative(self) -> None:
        for probe in self.probes():
            with self.subTest(probe=probe.name):
                baseline = self.pdf(probe, moved=False)
                unsigned = self.pdf(probe, moved=True)
                result = content.compare_pdf(baseline, unsigned, probe, 2, 4)
                self.assertEqual(result["outcome"], "UNSIGNED_COORDINATE_FIELD_EFFECT")
                prediction = result["field_prediction"]
                self.assertTrue(prediction["unsigned_prediction_matched"])
                self.assertFalse(prediction["signed_i16_prediction_matched"])
                self.assertGreater(abs(prediction["signed_i16_residual_after"]), 6000)
                if probe.axis == "y":
                    self.assertLess(prediction["unsigned_component_after"], 0)
                    self.assertGreater(prediction["signed_i16_component_after"], 200)
                else:
                    self.assertGreater(prediction["unsigned_component_after"], 100)
                    self.assertLess(prediction["signed_i16_component_after"], 0)
                signed = content.compare_pdf(baseline, self.pdf(probe, moved=True, signed=True), probe, 2, 4)
                self.assertEqual(signed["outcome"], "UNSUPPORTED")
                self.assertTrue(signed["field_prediction"]["signed_i16_prediction_matched"])
                self.assertFalse(signed["text_content_effect"])

    def test_unsigned_classification_rejects_any_other_component_draw_or_identity_change(self) -> None:
        probe = self.probes()[1]
        baseline, moved = self.pdf(probe, moved=False), self.pdf(probe, moved=True)
        for kind in ("opposite", "scale", "first", "non-target", "identity", "box", "nan", "prediction"):
            changed = deepcopy(moved)
            draw = changed["pages"][0]["draws"][1]
            if kind == "opposite": draw["pdf_ctm"][4] += 1
            elif kind == "scale": draw["pdf_ctm"][0] += 1
            elif kind == "first": changed["pages"][0]["draws"][0]["pdf_ctm"][4] += 1
            elif kind == "non-target": changed["pages"][1]["draws"][1]["pdf_ctm"][5] += 1
            elif kind == "identity": draw["raw_stream_sha256"] = "0" * 64
            elif kind == "box": changed["pages"][0]["media_box"][3] += 1
            elif kind == "nan": draw["pdf_ctm"][5] = float("nan")
            else: draw["pdf_ctm"][5] += 0.0001
            with self.subTest(kind=kind):
                result = content.compare_pdf(baseline, changed, probe, 2, 4)
                self.assertEqual(result["outcome"], "UNSUPPORTED")
                self.assertFalse(result["text_content_effect"])

    def test_repeated_probe_counts_calls_before_invocation_and_keeps_unsigned_classification(self) -> None:
        probe = self.probes()[0]
        profile = content.reference.Profile("c8", "synthetic.caj", "0" * 64, 2, 4, 1)
        for fail_at in (None, 1, 2):
            counts = content._report("highbit")["counts"]
            calls = 0

            def converter(_source, session, _paths, *, label):
                nonlocal calls
                calls += 1
                self.assertEqual(counts["converter_launches"], calls)
                if calls == fail_at:
                    raise content.ContentError("synthetic failed converter call")
                pdf = session / f"{label}.pdf"
                pdf.write_bytes(b"MIT synthetic PDF stand-in for a mocked converter")
                return {
                    "status": "PASS", "exit_code": 0, "timed_out": False,
                    "elapsed_milliseconds": 1, "output_path": str(pdf),
                    "pdf_sha256": digest(pdf.read_bytes()), "pdf_size_bytes": pdf.stat().st_size,
                    "max_observed_temporary_bytes": 1, "max_observed_child_vmhwm_kib": 2,
                    "max_observed_session_bytes": 3,
                }

            def metadata(pdf, _paths):
                return {
                    **self.pdf(probe, moved=True), "pdf_sha256": digest(pdf.read_bytes()),
                    "tools": {name: {"sha256": content.reference.PINNED_HASHES[name]}
                              for name in ("qpdf", "mutool", "pdfimages")},
                    "resources": {"max_tool_output_bytes": 4, "max_child_rss_kib": 5},
                }

            with self.subTest(fail_at=fail_at), \
                    patch.object(content.reference, "run_converter", side_effect=converter), \
                    patch.object(content.reference, "pdf_metadata", side_effect=metadata):
                args = (probe, profile, self.source, self.case(), self.pdf(probe, moved=False),
                        self.root, {}, digest(self.original), content._report("highbit")["resources"])
                if fail_at is None:
                    result = content._run_probe(*args, counts=counts)
                    self.assertEqual(result["outcome"], "UNSIGNED_COORDINATE_FIELD_EFFECT")
                    self.assertTrue(result["repeatable"])
                    self.assertEqual(result["converter_launches"], 2)
                    self.assertEqual(len(result["runs"]), 2)
                else:
                    with self.assertRaisesRegex(content.ContentError, "failed converter call"):
                        content._run_probe(*args, counts=counts)
                self.assertEqual(counts["converter_launches"], fail_at or 2)

    @contextmanager
    def runtime_files(self):
        """Portable, invented binary bytes and Linux mapping metadata."""
        library = self.root / "libz.so.1.3.1"
        python = self.root / "python3.13"
        library.write_bytes(b"MIT synthetic zlib binary stand-in" * 2500)
        python.write_bytes(b"MIT synthetic Python binary stand-in" * 2500)
        mappings = b"1000-2000 r-xp 0000 00:00 1 " + str(library).encode() + b"\n"
        real_open = Path.open
        self.runtime_read_requests = []
        self.runtime_open_paths = []

        class Reads:
            def __init__(reader, stream):
                reader.stream = stream

            def read(reader, size=-1):
                self.runtime_read_requests.append(size)
                return reader.stream.read(size)

            def __enter__(reader):
                return reader

            def __exit__(reader, *_args):
                reader.stream.close()

        def open_file(path, *args, **kwargs):
            self.runtime_open_paths.append(path)
            if path == Path("/proc/self/maps"):
                return Reads(io.BytesIO(mappings))
            return Reads(real_open(path, *args, **kwargs))

        with ExitStack() as stack:
            stack.enter_context(patch.object(content, "LIBZ_PATH", library))
            stack.enter_context(patch.object(content, "LIBZ_SHA256", digest(library.read_bytes())))
            stack.enter_context(patch.dict(content.reference.PINNED_HASHES, {
                "python": digest(python.read_bytes())}))
            stack.enter_context(patch.object(content.sys, "platform", "linux"))
            stack.enter_context(patch.object(content.sys, "executable", str(python)))
            stack.enter_context(patch.object(content.zlib, "ZLIB_VERSION", content.ZLIB_RUNTIME_VERSION))
            stack.enter_context(patch.object(Path, "open", open_file))
            yield library, python

    def test_runtime_audit_pins_actual_loaded_library_python_and_versions_with_bounded_reads(self) -> None:
        with self.runtime_files() as (library, python):
            result = content.audit_zlib_runtime()
            self.assertTrue(self.runtime_read_requests)
            self.assertEqual(max(self.runtime_read_requests), 65536)
            self.assertGreaterEqual(min(self.runtime_read_requests), 1)
            self.assertEqual(result["libz_sha256"], digest(library.read_bytes()))
            self.assertEqual(result["python_sha256"], digest(python.read_bytes()))
            self.assertEqual(result["mapped_libz_paths"], [str(library)])
            self.assertEqual(result["max_hash_request_bytes"], 65536)
            for kind in ("libz", "python", "runtime", "platform", "mapping"):
                with self.subTest(kind=kind), ExitStack() as stack:
                    if kind == "libz": stack.enter_context(patch.object(content, "LIBZ_SHA256", "0" * 64))
                    elif kind == "python": stack.enter_context(patch.dict(content.reference.PINNED_HASHES, {"python": "0" * 64}))
                    elif kind == "runtime": stack.enter_context(patch.object(content.zlib, "ZLIB_RUNTIME_VERSION", "changed"))
                    elif kind == "platform": stack.enter_context(patch.object(content.sys, "platform", "darwin"))
                    else: stack.enter_context(patch.object(Path, "open", return_value=io.BytesIO(b"unrelated mapping")))
                    with self.assertRaises(content.ContentError):
                        content.audit_zlib_runtime()

    def test_runtime_audit_rejects_oversized_binaries_before_opening_them(self) -> None:
        with self.runtime_files() as (library, python):
            real_stat = Path.stat
            for target, limit in ((library, 1024 * 1024), (python, 16 * 1024 * 1024)):
                def oversized(path, *args, **kwargs):
                    if path == target:
                        return SimpleNamespace(st_size=limit + 1)
                    return real_stat(path, *args, **kwargs)

                self.runtime_open_paths.clear()
                with self.subTest(target=target), patch.object(Path, "stat", oversized):
                    with self.assertRaisesRegex(ValueError, "size limit"):
                        content.audit_zlib_runtime()
                self.assertNotIn(target, self.runtime_open_paths)

    def _mock_highbit_batch(self, stack: ExitStack) -> tuple:
        paths, reference_report, probe_call, source_audit, environment_audit, _ = (
            donor_tests.TextContentTests._mock_batch(self, stack))
        runtime = {"libz_sha256": content.LIBZ_SHA256,
                   "python_sha256": content.reference.PINNED_HASHES["python"],
                   "zlib_runtime_version": content.ZLIB_RUNTIME_VERSION}
        runtime_audit = stack.enter_context(patch.object(content, "audit_zlib_runtime", return_value=runtime))
        return paths, reference_report, probe_call, source_audit, environment_audit, runtime_audit

    @staticmethod
    def successful_probe(probe, *_args, counts: dict) -> dict:
        counts["converter_launches"] += 2
        return {"name": probe.name, "outcome": "UNSIGNED_COORDINATE_FIELD_EFFECT",
                "repeatable": True, "converter_launches": 2,
                "runs": [{"status": "PASS"}, {"status": "PASS"}]}

    def test_shared_batch_bounds_four_copies_eight_calls_and_audits_runtime_afterward(self) -> None:
        with ExitStack() as stack:
            paths, reference_report, probe_call, source_audit, environment_audit, runtime_audit = self._mock_highbit_batch(stack)
            probe_call.side_effect = self.successful_probe
            report = content.run(paths, reference_report, batch="highbit")
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["counts"]["probes_completed"], 4)
        self.assertEqual(report["counts"]["converter_launches"], 8)
        self.assertEqual(report["counts"]["probe_runs"], 8)
        self.assertEqual(report["counts"]["coordinate_field_effect_probes"], 4)
        self.assertEqual(report["counts"]["unsigned_coordinate_field_effect_probes"], 4)
        self.assertEqual(report["zlib_audit"]["status"], "PASS")
        self.assertEqual((report["zlib_audit"]["before_checked"], report["zlib_audit"]["after_checked"]), (2, 2))
        self.assertEqual(runtime_audit.call_count, 2)
        self.assertEqual(source_audit.call_count, 2)
        self.assertEqual(environment_audit.call_count, 2)

    def test_post_runtime_drift_invalidates_completed_outcomes_and_before_failure_launches_none(self) -> None:
        with ExitStack() as stack:
            paths, reference_report, probe_call, _, _, runtime_audit = self._mock_highbit_batch(stack)
            probe_call.side_effect = self.successful_probe
            original = runtime_audit.return_value
            runtime_audit.side_effect = [original, {**original, "libz_sha256": "0" * 64}]
            changed = content.run(paths, reference_report, batch="highbit")
            runtime_audit.side_effect = content.ContentError("bad pinned libz binary")
            before_failed = content.run(paths, reference_report, batch="highbit")
        self.assertEqual(changed["status"], "FAIL")
        self.assertEqual(changed["counts"]["converter_launches"], 8)
        self.assertEqual(changed["zlib_audit"]["status"], "FAIL")
        self.assertEqual(changed["source_audit"]["status"], "PASS")
        self.assertEqual(changed["zlib_audit"]["after"]["libz_sha256"], "0" * 64)
        self.assertEqual(before_failed["status"], "FAIL")
        self.assertEqual(before_failed["counts"]["converter_launches"], 0)
        self.assertEqual(before_failed["counts"]["skipped_probes"], 4)
        self.assertEqual(before_failed["zlib_audit"]["status"], "FAIL")

    def test_mid_probe_failure_retains_launch_counts_and_every_available_post_audit(self) -> None:
        with ExitStack() as stack:
            paths, reference_report, probe_call, _, _, _ = self._mock_highbit_batch(stack)
            calls = 0

            def fail_second(probe, *args, counts: dict):
                nonlocal calls
                calls += 1
                if calls == 2:
                    counts["converter_launches"] += 1
                    raise content.ContentError("synthetic converter-call failure")
                return self.successful_probe(probe, *args, counts=counts)

            probe_call.side_effect = fail_second
            report = content.run(paths, reference_report, batch="highbit")
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["counts"]["probes_attempted"], 2)
        self.assertEqual(report["counts"]["probes_completed"], 1)
        self.assertEqual(report["counts"]["converter_launches"], 3)
        self.assertEqual(report["counts"]["probe_runs"], 2)
        self.assertEqual(report["counts"]["skipped_probes"], 2)
        self.assertEqual(report["probes"][-1]["converter_launches"], 1)
        self.assertTrue(all(report[name]["status"] == "PASS" for name in (
            "source_audit", "input_audit", "environment_audit", "zlib_audit")))


if __name__ == "__main__":
    unittest.main()
