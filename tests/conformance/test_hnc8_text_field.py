# SPDX-License-Identifier: MIT
"""Original synthetic controls for bounded decoded fields and zlib wrappers."""

from contextlib import ExitStack, redirect_stdout
from copy import deepcopy
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
sys.path.insert(0, str(Path(__file__).resolve().parent))
import hnc8_text_content as content  # noqa: E402
from hnc8_layout_source import FileInput, SourceExtractor  # noqa: E402
import test_hnc8_text_content as donor_tests  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


PLAIN = (b"header!!" + donor_tests.RECORD * 2 + b"sep!" + bytes(28) +
         struct.pack("<HH", 393, 435) + bytes(range(24)))


def synthetic_source() -> tuple[bytes, content.ContentProbe]:
    """Two original image slots; either u16 +100 keeps the frame size exact."""
    text = donor_tests.text_frame(PLAIN)
    assert len(PLAIN) == 100 and len(text) == 89
    text_offset = 120
    first_descriptor = text_offset + len(text)
    image = donor_tests.dib()
    second_descriptor = first_descriptor + 12 + len(image)
    data = bytearray(second_descriptor + 12 + len(image))
    data[:4] = bytes.fromhex("c8000000")
    data[8:12] = struct.pack("<i", 1)
    data[80:90] = struct.pack("<iih", text_offset, len(text), 2)
    data[text_offset:first_descriptor] = text
    for descriptor in (first_descriptor, second_descriptor):
        data[descriptor:descriptor + 12] = struct.pack(
            "<iii", 0, descriptor + 12, len(image))
        data[descriptor + 12:descriptor + 12 + len(image)] = image
    probe = content.ContentProbe(
        name="synthetic-x", profile="c8", page=1, donor_page=1,
        row_offset=80, target_offset=text_offset, target_length=len(text),
        donor_offset=text_offset, donor_length=len(text), first_descriptor=first_descriptor,
        level=9, mem_level=8, strategy=zlib.Z_DEFAULT_STRATEGY,
        chunk_size=content.MAX_INFLATED_BYTES, kind="field", field_offset=72,
        axis="x", original_value=393, new_value=493)
    return bytes(data), probe


class TextFieldTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.original, self.probe = synthetic_source()
        self.source = self.root / "synthetic.caj"
        self.source.write_bytes(self.original)
        profiles = patch.dict(content.text_frame.PROFILES, {
            "c8": content.text_frame.FrameProfile(
                "synthetic", digest(b"MIT synthetic header"), digest(b"A0B1C2"))})
        profiles.start()
        self.addCleanup(profiles.stop)
        # Synthetic tests must also work on hosts with another zlib release.
        runtime = patch.object(content, "ZLIB_RUNTIME_VERSION", zlib.ZLIB_RUNTIME_VERSION)
        runtime.start()
        self.addCleanup(runtime.stop)

    def y_probe(self) -> content.ContentProbe:
        return replace(self.probe, name="synthetic-y", axis="y", field_offset=74,
                       original_value=435, new_value=535)

    def wrapper_probe(self) -> content.ContentProbe:
        return replace(self.probe, name="synthetic-wrapper", kind="wrapper",
                       field_offset=None, axis=None, original_value=218, new_value=1)

    def case(self) -> dict:
        with FileInput(self.source) as source:
            reader = SourceExtractor(source, "synthetic.caj")
            page = reader.read_page(1)
        return {"source_pages": [{key: page[key] for key in (
            "images", "page_number", "text_length", "text_offset", "text_sha256")}]}

    def copy_control(self, probe: content.ContentProbe, name: str) -> dict:
        return content.copy_content(
            self.source, self.root / name, probe, digest(self.original),
            self.case()["source_pages"][0]["text_sha256"], None, 2)

    def pdfs(self, probe: content.ContentProbe, *, moved: bool) -> dict:
        pages = donor_tests.pdf_pages()
        draw = pages["pages"][0]["draws"][1]
        if probe.kind == "field":
            value = probe.new_value if moved else probe.original_value
            if probe.axis == "x":
                draw["pdf_ctm"][4] = round(value * content.COORDINATE_SCALE, 4)
            else:
                draw["pdf_ctm"][5] = round(200 - value * content.COORDINATE_SCALE, 4)
        return pages

    def test_clean_batches_are_explicit_and_make_zero_private_comparisons(self) -> None:
        for batch, planned in (("content", 2), ("fields", 6)):
            report = content.run(batch=batch)
            self.assertEqual(report["status"], "NOT_RUN")
            self.assertEqual(report["counts"]["probes_planned"], planned)
            self.assertEqual({value for key, value in report["counts"].items()
                              if key != "probes_planned"}, {0})
            failed = content.run({}, None, batch=batch)
            self.assertEqual(failed["status"], "FAIL")
            self.assertEqual(failed["counts"]["skipped_probes"], planned)
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(content.main(["--batch", "fields", "--json"]), 0)
        self.assertEqual(json.loads(output.getvalue())["counts"]["probes_planned"], 6)
        self.assertEqual(content.run()["batch"], "content")
        with self.assertRaises(content.ContentError):
            content.run(batch="unbounded")

    def test_six_frozen_controls_use_separate_exact_fields_and_wrapper_flags(self) -> None:
        self.assertEqual(len(content.FIELD_PROBES), 6)
        self.assertEqual([(p.profile, p.axis, p.field_offset, p.original_value, p.new_value)
                          for p in content.FIELD_PROBES[:4]], [
            ("c8", "x", 33576, 5978, 6078), ("c8", "y", 33578, 1479, 1579),
            ("hn_a", "x", 17048, 482, 582), ("hn_a", "y", 17050, 5446, 5546)])
        self.assertEqual([p.target_offset + 25 for p in content.FIELD_PROBES[4:]],
                         [245, 953345])
        for probe in content.FIELD_PROBES:
            content._check_plan(probe)
            self.assertEqual(probe.donor_page, probe.page)
            self.assertEqual(len(probe.expected_mutated_source_sha256), 64)
        for invalid in (replace(self.probe, kind="unknown"),
                        replace(self.probe, donor_page=2),
                        replace(self.probe, axis="diagonal"),
                        replace(self.probe, new_value=494),
                        replace(self.probe, original_value=65500, new_value=65600),
                        replace(self.probe, image_number=1),
                        replace(self.probe, chunk_size=512),
                        replace(self.wrapper_probe(), new_value=0)):
            with self.subTest(invalid=invalid), self.assertRaises(content.ContentError):
                content._check_plan(invalid)

    def test_field_copies_change_only_one_logical_u16_with_valid_complete_frame(self) -> None:
        for probe in (self.probe, self.y_probe()):
            with self.subTest(axis=probe.axis):
                copied = self.copy_control(probe, probe.name)
                mutant = Path(copied["path"])
                changed = mutant.read_bytes()
                self.assertEqual(self.source.read_bytes(), self.original)
                self.assertEqual(len(changed), len(self.original))
                self.assertEqual(changed[:probe.target_offset + 24],
                                 self.original[:probe.target_offset + 24])
                self.assertEqual(changed[probe.first_descriptor:],
                                 self.original[probe.first_descriptor:])
                plain, metadata = content._frame(
                    changed[probe.target_offset:probe.first_descriptor], image_count=2)
                expected = bytearray(PLAIN)
                expected[probe.field_offset:probe.field_offset + 2] = probe.new_value.to_bytes(2, "little")
                self.assertEqual(plain, expected)
                self.assertEqual(metadata.decoded_length, len(PLAIN))
                self.assertEqual(copied["logical_field"]["decoded_span"], [probe.field_offset, 2])
                self.assertTrue(copied["logical_field"]["all_other_decoded_bytes_unchanged"])
                self.assertEqual(copied["allowed_changed_spans"], [
                    [probe.target_offset + 24, probe.target_length - 24]])
                content._check_source(mutant, "synthetic.caj", self.case(), probe,
                                      mutated_text_sha256=copied["mutated_text_sha256"])

    def test_wrapper_copy_changes_one_flag_byte_and_preserves_deflate_adler_and_plaintext(self) -> None:
        probe = self.wrapper_probe()
        copied = self.copy_control(probe, "wrapper")
        changed = Path(copied["path"]).read_bytes()
        positions = [i for i, (old, new) in enumerate(zip(self.original, changed)) if old != new]
        self.assertEqual(positions, [probe.target_offset + 25])
        self.assertEqual(changed[positions[0]], 1)
        self.assertEqual(copied["changed_byte_count"], 1)
        self.assertEqual(copied["changed_offset_runs"], [[positions[0], 1]])
        self.assertTrue(copied["wrapper_control"]["fcheck_verified"])
        self.assertTrue(copied["wrapper_control"]["deflate_and_adler_bytes_unchanged"])
        self.assertEqual(content._frame(
            changed[probe.target_offset:probe.first_descriptor], image_count=2)[0], PLAIN)

    def test_wrong_decoded_slot_value_or_recompressed_payload_fails_before_copy(self) -> None:
        for invalid in (replace(self.probe, field_offset=76),
                        replace(self.probe, original_value=394, new_value=494),
                        replace(self.probe, image_number=3)):
            with self.subTest(invalid=invalid), self.assertRaises(content.ContentError):
                self.copy_control(invalid, "invalid")
        original_frame = self.original[self.probe.target_offset + 24:self.probe.first_descriptor]
        with patch.object(content, "_compressed", return_value=original_frame):
            with self.assertRaisesRegex(content.ContentError, "declared plaintext"):
                self.copy_control(self.probe, "unchanged-plaintext")
        text = self.original[self.probe.target_offset:self.probe.first_descriptor]
        other_valid_wrapper = text[:25] + bytes([156]) + text[26:]
        with self.assertRaisesRegex(content.ContentError, "predeclared"):
            content._replacement_text(other_valid_wrapper, None, self.wrapper_probe(), 2)
        self.assertFalse((self.root / "invalid").exists())

    def test_source_hash_pin_and_unchanged_row_header_and_images_remain_mandatory(self) -> None:
        with self.assertRaisesRegex(content.ContentError, "predeclared SHA"):
            self.copy_control(replace(self.probe, expected_mutated_source_sha256="0" * 64), "hash")
        copied = self.copy_control(self.probe, "edit")
        mutant = Path(copied["path"])
        valid_mutant = mutant.read_bytes()
        for position in (80, self.probe.target_offset, self.probe.target_offset + 20,
                         self.probe.first_descriptor):
            altered = bytearray(valid_mutant)
            altered[position] ^= 1
            mutant.write_bytes(altered)
            with self.assertRaisesRegex(content.ContentError, "outside declared"):
                content._audit_diff(self.source, mutant, self.probe)

    def test_donor_path_retains_only_donor_plaintext_and_validates_all_three_frames(self) -> None:
        original, probe = donor_tests.synthetic_source()
        source = self.root / "donor.caj"
        source.write_bytes(original)
        with FileInput(source) as ranged:
            reader = SourceExtractor(ranged, "donor.caj")
            target, donor = reader.read_page(1), reader.read_page(2)
        with patch.object(content.text_frame, "inspect_frame", wraps=content.text_frame.inspect_frame) as inspect:
            copied = content.copy_content(source, self.root / "donor-copy", probe, digest(original),
                                           target["text_sha256"], donor["text_sha256"], 1)
        self.assertEqual(inspect.call_count, 3)
        self.assertEqual(sum(call.kwargs["decoded_callback"] is not None
                             for call in inspect.call_args_list), 1)
        self.assertEqual(copied["donor_inflated_sha256"], digest(donor_tests.DONOR_PLAIN))

    def test_absolute_field_predictions_allow_only_the_selected_coordinate(self) -> None:
        for probe in (self.probe, self.y_probe()):
            with self.subTest(axis=probe.axis):
                baseline, moved = self.pdfs(probe, moved=False), self.pdfs(probe, moved=True)
                result = content.compare_pdf(baseline, moved, probe, 2, 4)
                self.assertEqual(result["outcome"], "COORDINATE_FIELD_EFFECT")
                self.assertTrue(result["text_content_effect"])
                self.assertNotIn("donor_translation_comparisons", result)
                self.assertEqual(len(result["ordered_pages_after"]), 2)
                component = 4 if probe.axis == "x" else 5
                exact = result["field_prediction"]["expected_component_after"]
                near = deepcopy(moved)
                near["pages"][0]["draws"][1]["pdf_ctm"][component] = exact + 0.00004
                self.assertEqual(content.compare_pdf(baseline, near, probe, 2, 4)["outcome"],
                                 "COORDINATE_FIELD_EFFECT")
                for change in ("prediction", "opposite", "first", "other", "identity", "nan"):
                    bad = deepcopy(moved)
                    target = bad["pages"][0]["draws"][1]
                    if change == "prediction": target["pdf_ctm"][component] = exact + 0.00006
                    elif change == "opposite": target["pdf_ctm"][9 - component] += 1
                    elif change == "first": bad["pages"][0]["draws"][0]["pdf_ctm"][4] += 1
                    elif change == "other": bad["pages"][1]["draws"][1]["pdf_ctm"][4] += 1
                    elif change == "identity": target["raw_stream_sha256"] = "0" * 64
                    else: target["pdf_ctm"][component] = float("nan")
                    with self.subTest(change=change):
                        self.assertEqual(content.compare_pdf(baseline, bad, probe, 2, 4)["outcome"],
                                         "UNSUPPORTED")
                bad_baseline = deepcopy(baseline)
                bad_baseline["pages"][0]["draws"][1]["pdf_ctm"][component] += 1
                self.assertEqual(content.compare_pdf(bad_baseline, moved, probe, 2, 4)["outcome"],
                                 "UNSUPPORTED")

    def test_field_guard_rejects_another_target_supplemental_draw_change(self) -> None:
        baseline, moved = self.pdfs(self.probe, moved=False), self.pdfs(self.probe, moved=True)
        extra = deepcopy(baseline["pages"][0]["draws"][1])
        extra.update({"draw_number": 3, "object_id": 13, "xobject_name": "/Im3"})
        baseline["pages"][0]["draws"].append(deepcopy(extra))
        moved["pages"][0]["draws"].append(deepcopy(extra))
        baseline["draw_count"] = moved["draw_count"] = 5
        self.assertEqual(content.compare_pdf(baseline, moved, self.probe, 2, 5)["outcome"],
                         "COORDINATE_FIELD_EFFECT")
        moved["pages"][0]["draws"][2]["pdf_ctm"][4] += 1
        self.assertEqual(content.compare_pdf(baseline, moved, self.probe, 2, 5)["outcome"],
                         "UNSUPPORTED")

    def test_wrapper_geometry_must_remain_exactly_unchanged(self) -> None:
        probe = self.wrapper_probe()
        baseline = self.pdfs(probe, moved=False)
        unchanged = content.compare_pdf(baseline, deepcopy(baseline), probe, 2, 4)
        self.assertEqual(unchanged["outcome"], "NO_PLACEMENT_CHANGE")
        self.assertTrue(unchanged["wrapper_placement_unchanged"])
        moved = deepcopy(baseline)
        moved["pages"][0]["draws"][1]["pdf_ctm"][4] += 0.00001
        self.assertEqual(content.compare_pdf(baseline, moved, probe, 2, 4)["outcome"], "UNSUPPORTED")

    def converter(self, _source: Path, session: Path, _paths: dict, *, label: str) -> dict:
        pdf = session / f"{label}.pdf"
        pdf.write_bytes(b"original MIT synthetic PDF stand-in")
        return {"status": "PASS", "exit_code": 0, "timed_out": False,
                "elapsed_milliseconds": 1, "output_path": str(pdf),
                "pdf_sha256": digest(pdf.read_bytes()), "pdf_size_bytes": pdf.stat().st_size,
                "max_observed_temporary_bytes": 1, "max_observed_child_vmhwm_kib": 2,
                "max_observed_session_bytes": 3}

    def metadata(self, pdf: Path, _paths: dict) -> dict:
        return {**self.pdfs(self.probe, moved=True), "pdf_sha256": digest(pdf.read_bytes()),
                "tools": {name: {"sha256": content.reference.PINNED_HASHES[name]}
                          for name in ("qpdf", "mutool", "pdfimages")},
                "resources": {"max_tool_output_bytes": 4, "max_child_rss_kib": 5}}

    def test_converter_launch_counter_precedes_calls_and_survives_mid_probe_failure(self) -> None:
        profile = content.reference.Profile("c8", "synthetic.caj", "0" * 64, 2, 4, 1)
        for failure_at in (None, 1, 2):
            counts = content._report("fields")["counts"]
            call_number = 0

            def converter(source: Path, session: Path, paths: dict, *, label: str) -> dict:
                nonlocal call_number
                call_number += 1
                self.assertEqual(counts["converter_launches"], call_number)
                if call_number == failure_at:
                    raise content.ContentError("synthetic converter protocol failure")
                return self.converter(source, session, paths, label=label)

            with self.subTest(failure_at=failure_at), \
                    patch.object(content.reference, "run_converter", side_effect=converter), \
                    patch.object(content.reference, "pdf_metadata", side_effect=self.metadata):
                args = (self.probe, profile, self.source, self.case(),
                        self.pdfs(self.probe, moved=False), self.root, {}, digest(self.original),
                        content._report("fields")["resources"])
                if failure_at is None:
                    result = content._run_probe(*args, counts=counts)
                    self.assertEqual(result["outcome"], "COORDINATE_FIELD_EFFECT")
                    self.assertTrue(result["repeatable"])
                    self.assertEqual(result["converter_launches"], 2)
                else:
                    with self.assertRaisesRegex(content.ContentError, "protocol failure"):
                        content._run_probe(*args, counts=counts)
                self.assertEqual(counts["converter_launches"], failure_at or 2)

    def test_shared_runner_counts_six_controls_and_failed_launch_separately_from_returned_runs(self) -> None:
        with ExitStack() as stack:
            paths, reference_report, probe_call, _, _, _ = donor_tests.TextContentTests._mock_batch(self, stack)

            def successful(probe, *_args, counts: dict) -> dict:
                counts["converter_launches"] += 2
                return {"outcome": ("COORDINATE_FIELD_EFFECT" if probe.kind == "field"
                                    else "NO_PLACEMENT_CHANGE"), "repeatable": True,
                        "runs": [{"status": "PASS"}, {"status": "PASS"}]}

            probe_call.side_effect = successful
            passed = content.run(paths, reference_report, batch="fields")
            self.assertEqual(passed["status"], "PASS")
            self.assertEqual(passed["counts"]["probes_completed"], 6)
            self.assertEqual(passed["counts"]["converter_launches"], 12)
            self.assertEqual(passed["counts"]["probe_runs"], 12)
            self.assertEqual(passed["counts"]["coordinate_field_effect_probes"], 4)
            self.assertEqual(passed["counts"]["wrapper_unchanged_probes"], 2)
            calls = 0

            def failure(probe, *args, counts: dict) -> dict:
                nonlocal calls
                calls += 1
                if calls == 2:
                    counts["converter_launches"] += 1
                    raise content.ContentError("failed launch before returned run record")
                return successful(probe, *args, counts=counts)

            probe_call.side_effect = failure
            failed = content.run(paths, reference_report, batch="fields")
        self.assertEqual(failed["status"], "FAIL")
        self.assertEqual(failed["counts"]["probes_attempted"], 2)
        self.assertEqual(failed["counts"]["probes_completed"], 1)
        self.assertEqual(failed["counts"]["converter_launches"], 3)
        self.assertEqual(failed["counts"]["probe_runs"], 2)
        self.assertEqual(failed["counts"]["skipped_probes"], 4)
        self.assertEqual(failed["probes"][-1]["converter_launches"], 1)
        self.assertEqual(failed["source_audit"]["status"], "PASS")
        self.assertEqual(failed["environment_audit"]["status"], "PASS")
        self.assertEqual(failed["input_audit"]["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
