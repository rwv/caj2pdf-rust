# SPDX-License-Identifier: MIT
"""Original synthetic checks of the fixed-row text-content controls."""

from contextlib import ExitStack
from dataclasses import replace
import hashlib
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_text_content as content  # noqa: E402
from hnc8_layout_source import FileInput, SourceExtractor  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def text_frame(plain: bytes) -> bytes:
    return b"MIT synthetic header" + struct.pack("<I", len(plain)) + zlib.compress(plain, 9)


RECORD = b"A0" + bytes(2) + b"B1" + bytes(2) + b"C2" + bytes(6)
TARGET_PLAIN = b"header!!" + RECORD * 2 + b"sep!" + b"I" * 28
DONOR_PLAIN = b"header!!" + RECORD + b"sep!" + bytes((0, 1)) + b"J" * 26


def dib() -> bytes:
    return (struct.pack("<IiiHHI", 40, 1, 1, 1, 1, 0) + bytes(20) +
            bytes.fromhex("ffffff0000000000") + b"\x00")


def synthetic_source() -> tuple[bytes, content.ContentProbe]:
    """Different decoded lengths share an exact valid compressed-span length."""
    target, donor = text_frame(TARGET_PLAIN), text_frame(DONOR_PLAIN)
    assert len(target) == len(donor) == 61
    target_offset = 120
    first_descriptor = target_offset + len(target)
    donor_offset = first_descriptor + 12 + len(dib())
    donor_descriptor = donor_offset + len(donor)
    data = bytearray(donor_descriptor + 12 + len(dib()))
    data[:4] = bytes.fromhex("c8000000")
    data[8:12] = struct.pack("<i", 2)
    data[80:90] = struct.pack("<iih", target_offset, len(target), 1)
    data[100:110] = struct.pack("<iih", donor_offset, len(donor), 1)
    data[target_offset:first_descriptor] = target
    data[first_descriptor:first_descriptor + 12] = struct.pack(
        "<iii", 0, first_descriptor + 12, len(dib()))
    data[first_descriptor + 12:donor_offset] = dib()
    data[donor_offset:donor_descriptor] = donor
    data[donor_descriptor:donor_descriptor + 12] = struct.pack(
        "<iii", 0, donor_descriptor + 12, len(dib()))
    data[donor_descriptor + 12:] = dib()
    return bytes(data), content.ContentProbe(
        "synthetic-fixed-row", "c8", 1, 2, 80, target_offset, len(target),
        donor_offset, len(donor), first_descriptor, 9, 8,
        zlib.Z_DEFAULT_STRATEGY, 1024)


def pdf_pages(*, target_x: float = 3.0) -> dict:
    def draw(number: int, sha: str, x: float) -> dict:
        return {"draw_number": number, "object_id": number + 10,
                "generation": 0, "xobject_name": f"/Im{number}",
                "xobject_type": "Image", "filter": "/DCTDecode",
                "color_space": "DeviceRGB", "bits_per_component": 8,
                "width": 3, "height": 2, "raw_stream_sha256": sha,
                "raw_stream_length": 42, "pdf_ctm": [3.0, 0.0, 0.0, -2.0, x, 9.0]}
    return {"page_count": 2, "draw_count": 4,
            "pages": [
                {"page_number": 1, "media_box": [0.0, 0.0, 100.0, 200.0],
                 "draws": [draw(1, "a" * 64, 0.0), draw(2, "b" * 64, target_x)]},
                {"page_number": 2, "media_box": [0.0, 0.0, 100.0, 200.0],
                 "draws": [draw(1, "c" * 64, 0.0), draw(2, "d" * 64, 5.0)]},
            ]}


class TextContentTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.original, self.probe = synthetic_source()
        self.source = self.root / "synthetic.caj"
        self.source.write_bytes(self.original)
        profiles = patch.dict(content.text_frame.PROFILES, {"c8": content.text_frame.FrameProfile(
            "synthetic", digest(b"MIT synthetic header"), digest(b"A0B1C2"))})
        profiles.start()
        self.addCleanup(profiles.stop)

    def case(self) -> dict:
        with FileInput(self.source) as source:
            reader = SourceExtractor(source, "synthetic.caj")
            return {"source_pages": [reader.read_page(1), reader.read_page(2)]}

    def copy_control(self, directory: str = "copy", probe: content.ContentProbe | None = None) -> dict:
        case = self.case()
        return content.copy_content(
            self.source, self.root / directory, probe or self.probe, digest(self.original),
            case["source_pages"][0]["text_sha256"], case["source_pages"][1]["text_sha256"], 1)

    def test_clean_clone_reports_zero_private_work_and_missing_paths_fail(self) -> None:
        report = content.run()
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["probes"], [])
        self.assertEqual({value for key, value in report["counts"].items()
                          if key != "probes_planned"}, {0})
        self.assertEqual(report["counts"]["probes_planned"], 2)
        self.assertEqual(report["resources"]["max_ranged_request_bytes"], 0)
        partial = content.run({}, None)
        self.assertEqual(partial["status"], "FAIL")
        self.assertEqual(partial["counts"]["skipped_probes"], 2)

    def test_frozen_controls_preserve_row_and_descriptor_boundaries(self) -> None:
        self.assertEqual([(p.profile, p.page, p.donor_page, p.row_offset,
                           p.target_offset, p.target_length, p.donor_offset,
                           p.donor_length, p.first_descriptor, p.level,
                           p.mem_level, p.strategy, p.chunk_size) for p in content.PROBES], [
            ("c8", 1, 2, 80, 220, 14546, 132124, 10690, 14766, 6, 8, 0, 384),
            ("hn_a", 16, 22, 16664, 953320, 7501, 1354683, 5344, 960821, 1, 1, 4, 488),
        ])
        for probe in content.PROBES:
            content._check_plan(probe)
            self.assertEqual(probe.target_offset + probe.target_length, probe.first_descriptor)
        for invalid in (replace(self.probe, target_length=60),
                        replace(self.probe, chunk_size=0),
                        replace(self.probe, target_length=content.MAX_TEXT_BYTES + 1),
                        replace(self.probe, mem_level=0)):
            with self.subTest(invalid=invalid), self.assertRaises(content.ContentError):
                content._check_plan(invalid)

    def test_complete_exact_size_control_preserves_row_images_and_original(self) -> None:
        case = self.case()
        request = content._check_source(self.source, "synthetic.caj", case, self.probe)
        record = self.copy_control()
        mutant = Path(record["path"])
        request = max(request, content._check_source(
            mutant, "synthetic.caj", case, self.probe,
            mutated_text_sha256=record["mutated_text_sha256"]))
        self.assertLessEqual(request, content.COPY_CHUNK)
        changed = mutant.read_bytes()
        self.assertEqual(self.source.read_bytes(), self.original)
        self.assertEqual(len(changed), len(self.original))
        self.assertEqual(changed[80:100], self.original[80:100])
        self.assertEqual(changed[:140], self.original[:140])
        self.assertEqual(changed[self.probe.first_descriptor:],
                         self.original[self.probe.first_descriptor:])
        framed = changed[self.probe.target_offset:self.probe.first_descriptor]
        self.assertEqual(int.from_bytes(framed[20:24], "little"), len(DONOR_PLAIN))
        self.assertEqual(content._frame(framed)[0], DONOR_PLAIN)
        self.assertEqual(record["donor_inflated_length"], len(DONOR_PLAIN))
        self.assertEqual(record["donor_inflated_sha256"], digest(DONOR_PLAIN))
        self.assertEqual(record["allowed_changed_spans"], [[140, 41]])
        positions = {position for start, count in record["changed_offset_runs"]
                     for position in range(start, start + count)}
        expected = {index for index, (left, right) in enumerate(zip(self.original, changed))
                    if left != right}
        self.assertEqual(positions, expected)
        self.assertEqual(record["changed_byte_count"], len(expected))
        self.assertEqual(record["changed_offset_runs"], sorted(record["changed_offset_runs"]))

    def test_frame_rejects_tail_truncation_wrong_size_checksum_and_bombs(self) -> None:
        good = text_frame(TARGET_PLAIN)
        bad_adler = good[:-1] + bytes([good[-1] ^ 1])
        oversized = good[:20] + (content.MAX_INFLATED_BYTES + 1).to_bytes(4, "little") + good[24:]
        wrong_length = good[:20] + (len(TARGET_PLAIN) - 1).to_bytes(4, "little") + good[24:]
        for bad in (good + b"unframed tail", good[:-2], bad_adler, wrong_length,
                    oversized, good[:24]):
            with self.subTest(length=len(bad)), self.assertRaises(content.ContentError):
                content._frame(bad)
        self.assertEqual(content._frame(good)[0], TARGET_PLAIN)

    def test_compression_has_one_exact_frame_and_rejects_size_or_runtime_drift(self) -> None:
        plain = DONOR_PLAIN
        framed = text_frame(plain)[:24] + content._compressed(plain, self.probe)
        self.assertEqual(content._frame(framed)[0], plain)
        for invalid in (replace(self.probe, target_length=60),
                        replace(self.probe, target_length=62)):
            with self.assertRaises(content.ContentError):
                content._compressed(plain, invalid)
        with patch.object(content.zlib, "ZLIB_RUNTIME_VERSION", "changed"):
            with self.assertRaisesRegex(content.ContentError, "runtime"):
                self.copy_control("wrong-runtime")

    def test_copy_rejects_unpinned_source_text_and_expected_mutation_hash(self) -> None:
        case = self.case()
        target, donor = [page["text_sha256"] for page in case["source_pages"]]
        for source_hash, target_hash, donor_hash in (("0" * 64, target, donor),
                                                    (digest(self.original), "0" * 64, donor),
                                                    (digest(self.original), target, "0" * 64)):
            with self.subTest(hashes=(source_hash, target_hash, donor_hash)):
                with self.assertRaises(content.ContentError):
                    content.copy_content(self.source, self.root / str(len(list(self.root.iterdir()))),
                                         self.probe, source_hash, target_hash, donor_hash, 1)
        with self.assertRaisesRegex(content.ContentError, "predeclared SHA"):
            self.copy_control("wrong-mutated-hash", replace(
                self.probe, expected_mutated_source_sha256="0" * 64))

    def test_audit_rejects_row_header_image_and_unrelated_page_edits(self) -> None:
        for index, position in enumerate((80, 120, self.probe.first_descriptor,
                                          self.probe.donor_offset)):
            record = self.copy_control(f"changed-{index}")
            mutant = Path(record["path"])
            with mutant.open("r+b") as stream:
                stream.seek(position)
                value = stream.read(1)
                stream.seek(position)
                stream.write(bytes([value[0] ^ 1]))
            with self.assertRaisesRegex(content.ContentError, "outside declared"):
                content._audit_diff(self.source, mutant, self.probe)

    def test_pdf_guard_interprets_only_target_supplemental_translation(self) -> None:
        baseline = pdf_pages()
        moved = content.compare_pdf(baseline, pdf_pages(target_x=5.0), self.probe, 2, 4)
        self.assertEqual(moved["outcome"], "TEXT_CONTENT_DEPENDENCY")
        self.assertTrue(moved["text_content_effect"])
        self.assertEqual(moved["donor_translation_matches"], 1)
        self.assertEqual(moved["non_target_pages_checked"], 1)
        self.assertEqual(moved["ordered_pages_before"][1], moved["ordered_pages_after"][1])
        self.assertEqual(moved["ordered_pages_after"][0]["draws"][1]["pdf_ctm"][4], 5.0)
        self.assertEqual(sum(len(page["draws"]) for page in moved["ordered_pages_after"]), 4)
        unchanged = content.compare_pdf(baseline, baseline, self.probe, 2, 4)
        self.assertEqual(unchanged["outcome"], "NO_PLACEMENT_CHANGE")
        for kind in ("image", "first", "scale", "other", "box", "count"):
            changed = pdf_pages(target_x=5.0)
            if kind == "image": changed["pages"][0]["draws"][1]["raw_stream_sha256"] = "0" * 64
            elif kind == "first": changed["pages"][0]["draws"][0]["pdf_ctm"][4] += 1
            elif kind == "scale": changed["pages"][0]["draws"][1]["pdf_ctm"][0] += 1
            elif kind == "other": changed["pages"][1]["draws"][1]["pdf_ctm"][4] += 1
            elif kind == "box": changed["pages"][0]["media_box"][2] += 1
            else: changed["draw_count"] += 1
            with self.subTest(kind=kind):
                self.assertEqual(content.compare_pdf(
                    baseline, changed, self.probe, 2, 4)["outcome"], "UNSUPPORTED")

    def _converter(self, _source: Path, session: Path, _paths: dict, *, label: str) -> dict:
        pdf = session / f"{label}.pdf"
        pdf.write_bytes(b"original MIT synthetic PDF stand-in")
        return {"status": "PASS", "exit_code": 0, "timed_out": False,
                "elapsed_milliseconds": 1, "output_path": str(pdf),
                "pdf_sha256": digest(pdf.read_bytes()), "pdf_size_bytes": pdf.stat().st_size,
                "max_observed_temporary_bytes": 1, "max_observed_child_vmhwm_kib": 2,
                "max_observed_session_bytes": 3}

    def _metadata(self, pdf: Path, _paths: dict) -> dict:
        return {**pdf_pages(target_x=5.0), "pdf_sha256": digest(pdf.read_bytes()),
                "tools": {name: {"sha256": content.reference.PINNED_HASHES[name]}
                          for name in ("qpdf", "mutool", "pdfimages")},
                "resources": {"max_tool_output_bytes": 4, "max_child_rss_kib": 5}}

    def test_repeated_probe_checks_pdf_hashes_tools_and_sources(self) -> None:
        profile = content.reference.Profile("c8", "synthetic.caj", "0" * 64, 2, 4, 1)
        with patch.object(content.reference, "run_converter", side_effect=self._converter), \
                patch.object(content.reference, "pdf_metadata", side_effect=self._metadata):
            result = content._run_probe(self.probe, profile, self.source, self.case(),
                                        pdf_pages(), self.root, {}, digest(self.original),
                                        content._report()["resources"])
        self.assertEqual(result["outcome"], "TEXT_CONTENT_DEPENDENCY")
        self.assertTrue(result["repeatable"])
        self.assertEqual(len(result["runs"]), 2)
        self.assertEqual(result["unchanged_target_row_span"], [80, 20])
        for bad in ("pdf", "tool"):
            def metadata(pdf: Path, paths: dict) -> dict:
                parsed = self._metadata(pdf, paths)
                if bad == "pdf": parsed["pdf_sha256"] = "0" * 64
                else: parsed["tools"]["qpdf"]["sha256"] = "0" * 64
                return parsed
            with self.subTest(bad=bad), \
                    patch.object(content.reference, "run_converter", side_effect=self._converter), \
                    patch.object(content.reference, "pdf_metadata", side_effect=metadata):
                with self.assertRaises(content.ContentError):
                    content._run_probe(self.probe, profile, self.source, self.case(),
                                       pdf_pages(), self.root, {}, digest(self.original),
                                       content._report()["resources"])

    def _mock_batch(self, stack: ExitStack) -> tuple:
        number = len(list(self.root.iterdir()))
        batch = self.root / f"batch-{number}"
        corpus, checkout = batch / "corpus", batch / "checkout"
        corpus.mkdir(parents=True)
        checkout.mkdir()
        rows, baseline_pdfs = [], {}
        for profile in content.reference.PROFILES:
            source = corpus / profile.source_id
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_bytes(b"synthetic")
            rows.append({"id": profile.source_id, "path": profile.source_id,
                         "size_bytes": 9, "sha256": digest(b"synthetic")})
            for index in (1, 2):
                pdf = batch / f"{profile.name}-run{index}.pdf"
                pdf.write_bytes(b"synthetic PDF")
                baseline_pdfs[f"{profile.name}-run{index}"] = pdf
        audited = [{"id": row["id"], "path": str(corpus / row["path"]),
                    "size_bytes": 9, "sha256": row["sha256"]} for row in rows]
        environment = {
            "reference_revision": content.reference.REFERENCE_REVISION,
            "reference_clean": True, "pypdf2": {},
            "binaries": {name: {"sha256": sha}
                         for name, sha in content.reference.PINNED_HASHES.items()},
            "native_compiler_record": {}, "environment": {},
            "invocation": [], "timeout_seconds": 180,
        }
        paths = {name: batch / name for name in ("python", "pydeps", "libjbigdec", "git",
                                                 "qpdf", "mutool", "pdfinfo", "pdfimages")}
        paths.update({"corpus": corpus, "reference_repo": checkout,
                      "artifact_root": batch / "artifacts"})
        reference_report = batch / "reference-report.json"
        reference_report.write_text("{}", encoding="utf-8")

        def metadata(pdf: Path, _paths: dict) -> dict:
            profile = next(p for p in content.reference.PROFILES if pdf.name.startswith(p.name + "-"))
            return {"pdf_sha256": profile.expected_pdf_sha256, "page_count": profile.expected_pages,
                    "draw_count": profile.expected_draws, "pages": [],
                    "tools": {name: {"sha256": content.reference.PINNED_HASHES[name]}
                              for name in ("qpdf", "mutool", "pdfimages")},
                    "resources": {"max_tool_output_bytes": 1, "max_child_rss_kib": 2}}

        stack.enter_context(patch.object(content.reference, "load_source_rows", return_value=(
            content.reference.MATRIX_SHA256, rows)))
        source_audit = stack.enter_context(patch.object(content.reference, "audit_sources", return_value=audited))
        env_audit = stack.enter_context(patch.object(content.reference, "audit_environment", return_value=environment))
        stack.enter_context(patch.object(content.shared, "_load_oracle", return_value={
            p.name: {"pdf_pages": []} for p in content.reference.PROFILES}))
        stack.enter_context(patch.object(content.shared, "_read_reference_report", return_value=(
            {"status": "PASS"}, baseline_pdfs)))
        input_digest = stack.enter_context(patch.object(content.shared, "_file_digest", return_value="b" * 64))
        stack.enter_context(patch.object(content.reference, "pdf_metadata", side_effect=metadata))
        probe_call = stack.enter_context(patch.object(content, "_run_probe"))
        return paths, reference_report, probe_call, source_audit, env_audit, input_digest

    def test_batch_reports_partial_and_mid_probe_failure_counts_and_audits(self) -> None:
        normal = {"outcome": "NO_PLACEMENT_CHANGE", "repeatable": True,
                  "runs": [{"status": "PASS"}, {"status": "PASS"}]}
        effect = {**normal, "outcome": "TEXT_CONTENT_DEPENDENCY"}
        with ExitStack() as stack:
            paths, reference_report, probe_call, source_audit, env_audit, _ = self._mock_batch(stack)
            probe_call.side_effect = [effect, normal]
            passed = content.run(paths, reference_report)
            probe_call.side_effect = [normal, {**normal, "outcome": "UNSUPPORTED"}]
            partial = content.run(paths, reference_report)
            probe_call.side_effect = [normal, content.ContentError("synthetic failure")]
            failed = content.run(paths, reference_report)
        self.assertEqual(passed["status"], "PASS")
        self.assertEqual(passed["counts"]["text_content_effect_probes"], 1)
        self.assertEqual(passed["counts"]["probes_passing"], 2)
        self.assertEqual(passed["counts"]["probe_runs"], 4)
        self.assertEqual(passed["source_audit"]["status"], "PASS")
        self.assertEqual(passed["environment_audit"]["status"], "PASS")
        self.assertEqual(passed["input_audit"]["status"], "PASS")
        self.assertEqual(passed["counts"]["baseline_pdf_checks_after"], 6)
        self.assertEqual(partial["status"], "PARTIAL")
        self.assertEqual(partial["counts"]["unsupported_probes"], 1)
        self.assertEqual(partial["counts"]["probes_failing"], 1)
        self.assertEqual(failed["status"], "FAIL")
        self.assertEqual(failed["counts"]["probes_attempted"], 2)
        self.assertEqual(failed["counts"]["probes_completed"], 1)
        self.assertEqual(failed["counts"]["probe_runs"], 2)
        self.assertEqual(failed["counts"]["probes_failing"], 1)
        self.assertEqual(failed["counts"]["skipped_probes"], 0)
        self.assertGreaterEqual(source_audit.call_count, 6)
        self.assertGreaterEqual(env_audit.call_count, 6)

    def test_batch_rejects_changed_sources_environment_and_every_pinned_input(self) -> None:
        normal = {"outcome": "NO_PLACEMENT_CHANGE", "repeatable": True,
                  "runs": [{"status": "PASS"}, {"status": "PASS"}]}
        for changed in ("sources", "environment", *range(9)):
            with self.subTest(changed=changed), ExitStack() as stack:
                paths, reference_report, probe_call, source_audit, env_audit, input_digest = self._mock_batch(stack)
                probe_call.side_effect = [normal, normal]
                if changed == "sources": source_audit.side_effect = [source_audit.return_value, []]
                elif changed == "environment": env_audit.side_effect = [env_audit.return_value, {}]
                else: input_digest.side_effect = (["b" * 64] * 9 +
                                                   ["c" * 64 if n == changed else "b" * 64
                                                    for n in range(9)])
                report = content.run(paths, reference_report)
                self.assertEqual(report["status"], "FAIL")
                self.assertTrue(report["errors"])


if __name__ == "__main__":
    unittest.main()
