# SPDX-License-Identifier: MIT
"""Clean-clone checks for the optional JBIG2 black-box oracle."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_oracle as oracle  # noqa: E402


class PixelOracleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_all_27_source_hashes_are_checked_before_scan(self) -> None:
        rows = []
        for number in range(27):
            path = self.root / f"sample-{number}.caj"
            data = f"owned synthetic source {number}".encode()
            path.write_bytes(data)
            rows.append({"id": path.name, "path": path.name, "variant": "HN", "size_bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        with patch.object(oracle.conformance, "load_matrix", return_value=rows):
            actual, paths = oracle.validate_sources(self.root / "matrix.json", self.root)
            self.assertEqual(len(actual), 27)
            self.assertEqual(len(paths), 27)
            (self.root / "sample-26.caj").write_bytes(b"X" * rows[26]["size_bytes"])
            with self.assertRaisesRegex(oracle.OracleError, "sample-26.caj: source SHA-256 differs"):
                oracle.validate_sources(self.root / "matrix.json", self.root)

    def test_pbm_normalization_masks_padding_and_rejects_bad_pixels(self) -> None:
        left = self.root / "left.pbm"
        right = self.root / "right.pbm"
        left.write_bytes(b"P4\n9 2\n" + bytes((0x80, 0xFF, 0x00, 0xFF)))
        right.write_bytes(b"P4\n9 2\n" + bytes((0x80, 0x80, 0x00, 0x80)))
        digest, bits = oracle.compare_pbm(left, right, 9, 2)
        self.assertEqual(digest, hashlib.sha256(bytes((0x80, 0x80, 0x00, 0x80))).hexdigest())
        self.assertEqual(bits, 3)
        right.write_bytes(b"P4\n9 2\n" + bytes((0x80, 0x80, 0x01, 0x80)))
        with self.assertRaisesRegex(oracle.OracleError, "pixel mismatch at row 1"):
            oracle.compare_pbm(left, right, 9, 2)
        right.write_bytes(b"P4\n9 2\n" + bytes((0x80, 0x80, 0x00, 0x80, 0x00)))
        with self.assertRaisesRegex(oracle.OracleError, "trailing bytes"):
            oracle.compare_pbm(left, right, 9, 2)
        right.write_bytes(b"P4\n10 2\n" + bytes((0x80, 0x80, 0x00, 0x80)))
        with self.assertRaisesRegex(oracle.OracleError, "dimensions differ"):
            oracle.compare_pbm(left, right, 9, 2)

    def test_pdf_spools_one_span_and_cleans_up_after_a_tool_failure(self) -> None:
        source = self.root / "tiny.caj"
        source.write_bytes(b"X" * 48 + b"owned bytes")
        case = {"source_path": source, "offset": 0, "length": source.stat().st_size, "width": 9, "height": 2}
        destination = self.root / "proof.pdf"
        encoded_sha = oracle.sha256_file(source)
        oracle.write_pdf(case, destination, encoded_sha)
        pdf = destination.read_bytes()
        self.assertIn(b"/Filter /JBIG2Decode", pdf)
        self.assertIn(b"/Decode [0 1]", pdf)
        self.assertIn(b"stream\nowned bytes\nendstream", pdf)
        observed = []

        def fail_tool(_argv: list[str], log: Path, _timeout: int = 60) -> None:
            observed.append(log.parent)
            raise oracle.OracleError("synthetic qpdf failure")

        with patch.object(oracle, "run_tool", side_effect=fail_tool):
            with self.assertRaisesRegex(oracle.OracleError, "synthetic qpdf failure"):
                oracle.decode_case(
                    case, {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")},
                    encoded_sha,
                )
        self.assertEqual(len(observed), 1)
        self.assertFalse(observed[0].exists())

    def test_spool_rejects_mutation_between_profile_hash_and_pdf_validation(self) -> None:
        source = self.root / "tiny.caj"
        original = b"X" * 48 + b"owned bytes"
        source.write_bytes(original)
        case = {"source_path": source, "offset": 0, "length": len(original), "width": 8, "height": 1}
        encoded_sha = oracle.sha256_file(source)
        for changed in (b"Y" + original[1:], original[:48] + b"other bytes"):
            with self.subTest(changed=changed[:1]):
                source.write_bytes(changed)
                with patch.object(oracle, "run_tool") as run_tool:
                    with self.assertRaisesRegex(oracle.OracleError, "changed between profile read and PDF spool"):
                        oracle.decode_case(
                            case, {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")},
                            encoded_sha,
                        )
                run_tool.assert_not_called()

    def test_two_tool_pipeline_reports_mismatch_and_cleans_up(self) -> None:
        source = self.root / "tiny.caj"
        source.write_bytes(b"X" * 48 + b"owned bytes")
        case = {"source_path": source, "offset": 0, "length": source.stat().st_size, "width": 8, "height": 1}
        observed = []

        def fake_tool(argv: list[str], log: Path, _timeout: int = 60) -> None:
            observed.append(log.parent)
            log.write_bytes(b"")
            if argv[0] == "pdfimages":
                Path(argv[-1] + "-000.pbm").write_bytes(b"P4\n8 1\n\x80")
            elif argv[0] == "mutool":
                Path(argv[argv.index("-o") + 1]).write_bytes(b"P4\n8 1\n\x40")

        with patch.object(oracle, "run_tool", side_effect=fake_tool):
            with self.assertRaisesRegex(oracle.OracleError, "pixel mismatch at row 0"):
                oracle.decode_case(
                    case, {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")},
                    oracle.sha256_file(source),
                )
        self.assertEqual(len(observed), 3)
        self.assertFalse(observed[0].exists())

    def test_early_pixel_mismatch_closes_both_pbm_streams(self) -> None:
        closed = []

        def rows(label: str, value: bytes):
            try:
                yield value
                yield b"\0"
            finally:
                closed.append(label)

        def fake_rows(path: Path, _width: int, _height: int):
            return rows(path.name, b"\x80" if path.name == "left.pbm" else b"\x40")

        with patch.object(oracle, "pbm_rows", side_effect=fake_rows):
            with self.assertRaisesRegex(oracle.OracleError, "pixel mismatch at row 0"):
                oracle.compare_pbm(Path("left.pbm"), Path("right.pbm"), 8, 2)
        self.assertEqual(set(closed), {"left.pbm", "right.pbm"})

    def test_qpdf_warning_is_not_accepted_as_validation(self) -> None:
        log = self.root / "qpdf.log"
        log.write_bytes(b"checking temporary.pdf\nWARNING: xref stream repaired\n")
        with self.assertRaisesRegex(oracle.OracleError, "qpdf warned"):
            oracle.check_qpdf_log(log)

    def test_qpdf_warning_stops_before_any_pixel_tool(self) -> None:
        source = self.root / "tiny.caj"
        source.write_bytes(b"X" * 48 + b"owned bytes")
        case = {"source_path": source, "offset": 0, "length": source.stat().st_size, "width": 8, "height": 1}
        commands = []

        def fake_tool(argv: list[str], log: Path, _timeout: int = 60) -> None:
            commands.append(argv[0])
            log.write_bytes(b"WARNING: repaired PDF\n")

        with patch.object(oracle, "run_tool", side_effect=fake_tool):
            with self.assertRaisesRegex(oracle.OracleError, "qpdf warned"):
                oracle.decode_case(
                    case, {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")},
                    oracle.sha256_file(source),
                )
        self.assertEqual(commands, ["qpdf"])

    def test_profile_records_single_text_flag_anomaly(self) -> None:
        width, height = 8, 1
        dib = struct.pack("<IiiHHI", 40, width, height, 1, 1, 0) + b"\0" * 20 + b"\xff\xff\xff\0\0\0\0\0"
        page = struct.pack(">IIII", width, height, 0, 0) + b"\x01\0\0"
        symbol1 = b"\x08\0\x02\xff" + struct.pack(">II", 1, 1)
        symbol2 = b"\x18\x02\x02\xff" + struct.pack(">II", 1, 0)
        region = struct.pack(">IIII", width, height, 0, 0) + b"\0"
        text = region + b"\xa4\x0c" + struct.pack(">I", 1)
        generic = region + b"\x04\x02\xff"
        parts = [page, symbol1, symbol2, text, generic]
        source = self.root / "synthetic.caj"
        source.write_bytes(dib + b"".join(parts))
        segments = []
        offset = 48
        for part in parts:
            segments.append({"data_offset": offset, "data_length": len(part)})
            offset += len(part)
        case = {"source_path": source, "offset": 0, "length": offset, "width": width, "height": height, "segments": segments, "coordinate": oracle.ANOMALY_COORDINATE}
        profile, encoded_sha = oracle.profile_and_hash(case)
        self.assertEqual(profile["text_flags"], "0xa40c")
        self.assertEqual(profile["text_flag_anomaly"], "SBREFINE=0 with SBRTEMPLATE=1")
        self.assertEqual(encoded_sha, oracle.sha256_file(source))
        case["coordinate"] = ("another source", 11, 1)
        with self.assertRaisesRegex(oracle.OracleError, "anomaly moved or changed"):
            oracle.profile_and_hash(case)

    def test_manifest_schema_rejects_unverified_or_duplicate_images(self) -> None:
        digest = "a" * 64
        profile = {"text_flags": "0x900e", "text_flag_anomaly": None}
        image = {"page": 1, "image": 1, "offset": 48, "length": 100, "width": 8, "height": 1, "encoded_sha256": digest, "normalized_pixel_sha256": digest, "profile": profile, "status": "PASS"}
        manifest = {"schema_version": 1, "matrix_sha256": digest, "backend_independence": "UNVERIFIED", "toolchains": {}, "samples": [{"id": "synthetic", "source_sha256": digest, "images": [image]}]}
        oracle.validate_manifest(manifest, expected_images=1)
        image["status"] = "NOT_RUN"
        with self.assertRaisesRegex(oracle.OracleError, "unverified image"):
            oracle.validate_manifest(manifest, expected_images=1)
        image["status"] = "PASS"
        manifest["samples"][0]["images"].append(image.copy())
        with self.assertRaisesRegex(oracle.OracleError, "duplicate image coordinate"):
            oracle.validate_manifest(manifest, expected_images=2)

    def test_committed_manifest_has_all_pinned_images_and_anomaly(self) -> None:
        manifest = json.loads(oracle.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        oracle.validate_manifest(manifest)
        self.assertEqual(
            {sample["id"]: len(sample["images"]) for sample in manifest["samples"]},
            oracle.EXPECTED_TYPE3,
        )
        anomalies = [
            (sample["id"], image["page"], image["image"])
            for sample in manifest["samples"] for image in sample["images"]
            if image["profile"]["text_flag_anomaly"] is not None
        ]
        self.assertEqual(anomalies, [oracle.ANOMALY_COORDINATE])

    def test_manifest_rejects_unexpected_bytes_at_every_metadata_level(self) -> None:
        baseline = json.loads(oracle.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        paths = [
            (),
            ("toolchains",),
            ("toolchains", "tools"),
            ("toolchains", "tools", "qpdf"),
            ("toolchains", "backend_evidence"),
            ("toolchains", "backend_evidence", "dynamic_libjbig2dec"),
            ("samples", 0),
            ("samples", 0, "images", 0),
            ("samples", 0, "images", 0, "profile"),
        ]
        for path in paths:
            with self.subTest(path=path):
                changed = json.loads(json.dumps(baseline))
                target = changed
                for part in path:
                    target = target[part]
                target["raw_pixel_bytes"] = "synthetic data must not enter a metadata manifest"
                with self.assertRaisesRegex(oracle.OracleError, "unknown fields: raw_pixel_bytes"):
                    oracle.validate_manifest(changed)

    def test_manifest_comparison_reports_pixel_drift(self) -> None:
        baseline = {"schema_version": 1, "matrix_sha256": "a" * 64, "corpus_revision": "test", "normalization": "P4", "backend_independence": "UNVERIFIED", "toolchains": {"version": "one"}, "samples": [{"id": "synthetic", "path": "sample.caj", "variant": "HN", "source_sha256": "b" * 64, "images": [{"page": 1, "image": 1, "normalized_pixel_sha256": "c" * 64}]}]}
        changed = json.loads(json.dumps(baseline))
        changed["toolchains"]["version"] = "two"
        changed["samples"][0]["images"][0]["normalized_pixel_sha256"] = "d" * 64
        self.assertEqual(
            oracle.compare_manifest(changed, baseline),
            ["synthetic: page 1 image 1"],
        )

    def test_manifest_comparison_allows_another_tool_build_with_same_pixels(self) -> None:
        baseline = {
            "schema_version": 1,
            "matrix_sha256": "a" * 64,
            "corpus_revision": "test",
            "normalization": "P4",
            "backend_independence": "UNVERIFIED",
            "toolchains": {"qpdf": "original build"},
            "samples": [{
                "id": "synthetic",
                "path": "sample.caj",
                "variant": "HN",
                "source_sha256": "b" * 64,
                "images": [{"page": 1, "image": 1, "normalized_pixel_sha256": "c" * 64}],
            }],
        }
        changed = json.loads(json.dumps(baseline))
        changed["toolchains"] = {"qpdf": "another build"}
        self.assertEqual(oracle.compare_manifest(changed, baseline), [])
        self.assertNotEqual(changed["toolchains"], baseline["toolchains"])

    def test_missing_corpus_reports_not_run(self) -> None:
        from io import StringIO
        output = StringIO()
        with patch("sys.stdout", output):
            result = oracle.main(["--corpus-dir", str(self.root / "missing"), "--manifest", str(self.root / "missing-manifest.json"), "--json"])
        self.assertEqual(result, 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")
        self.assertEqual(json.loads(output.getvalue())["tool_agreements"], 0)

    def test_missing_black_box_tool_reports_not_run_after_source_checks(self) -> None:
        from io import StringIO
        output = StringIO()
        with patch.object(oracle, "validate_sources", return_value=([{}] * 27, {})), patch.object(oracle, "tool_path", return_value=None), patch("sys.stdout", output):
            result = oracle.main(["--corpus-dir", str(self.root), "--manifest", str(self.root / "missing-manifest.json"), "--json"])
        report = json.loads(output.getvalue())
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["source_hashes_checked"], 27)
        self.assertEqual(report["tool_agreements"], 0)


if __name__ == "__main__":
    unittest.main()
