# SPDX-License-Identifier: MIT
"""Synthetic tests for the opt-in, black-box JBIG1 oracle runner."""

from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig1_oracle as oracle  # noqa: E402


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def blob_oid(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode("ascii") + data).hexdigest()


def dib(
    width: int = 9,
    height: int = 2,
    bits: int = 1,
    compression: int = 0,
    palette: bytes = b"\xff\xff\xff\x00\x00\x00\x00\x00",
) -> bytes:
    header = struct.pack(
        "<IiiHHIIiiII", 40, width, height, 1, bits, compression, 0, 0, 0, 2, 0
    )
    return header + palette + b"\x12\x34\x56"


def container(variant: str = "C8", multi_image: bool = True) -> tuple[bytes, dict]:
    """Build a one-page file with chained records and an allowed final suffix."""
    if variant == "C8":
        table_at, count_at, signature = 0x50, 0x08, b"\xc8\0\0\0"
    elif variant == "HN-long":
        table_at, count_at, signature = 0x15C + 308, 0x90, b"HN\0\0\x90\x01\0\0"
    else:
        table_at, count_at, signature = 0xD8, 0x90, b"HN\0\0\xc8\0\0\0"
    content = bytearray(table_at + 20)
    content[: len(signature)] = signature
    struct.pack_into("<i", content, count_at, 1)
    if variant == "HN-long":
        struct.pack_into("<i", content, 0x158, 1)
    text_at = len(content)
    content.extend(b"TEXT")
    record_at = len(content)
    first_type = 2 if multi_image else 0
    image_count = 2 if multi_image else 1
    struct.pack_into("<iihhii", content, table_at, text_at, 4, image_count, 0, 0, 0)
    content.extend(b"\0" * 12)
    content.extend(b"gap")
    first_image_at = len(content)
    first_image = b"NONZERO" if multi_image else dib()
    content.extend(first_image)
    struct.pack_into("<iii", content, record_at, first_type, first_image_at, len(first_image))
    if multi_image:
        second_record_at = len(content)
        content.extend(b"\0" * 12)
        content.extend(b"padding")
        second_image_at = len(content)
        second_image = dib()
        content.extend(second_image)
        struct.pack_into("<iii", content, second_record_at, 0, second_image_at, len(second_image))
    else:
        second_record_at = None
        second_image_at = first_image_at
    content.extend(b"final suffix is allowed")
    return bytes(content), {
        "table_at": table_at,
        "record_at": record_at,
        "second_record_at": second_record_at,
        "image_at": second_image_at,
        "image_index": image_count,
    }


class OracleRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self.matrix_path = self.root / "matrix.json"
        self.manifest_path = self.root / "manifest.json"
        self.content, self.positions = container()
        self.sample_path = self.corpus / "sample.c8"
        self.row = self.write_sample(self.content)
        self.write_matrix()
        self.write_manifest()

    def write_sample(self, data: bytes, variant: str = "C8") -> dict:
        self.sample_path.write_bytes(data)
        return {
            "id": "sample",
            "path": "sample.c8",
            "aliases": [],
            "size_bytes": len(data),
            "git_blob_oid": blob_oid(data),
            "sha256": sha256(data),
            "detected_type": variant,
            "variant": variant,
            "expected_outcome": "success",
            "page_count": 1,
            "outline_count": 0,
            "python_reference": {"show_status": "success", "convert_status": "success"},
        }

    def write_matrix(self) -> None:
        self.matrix_path.write_text(
            json.dumps({"schema_version": 1, "samples": [self.row]}), encoding="utf-8"
        )

    def write_manifest(self, failures: list[dict] | None = None) -> None:
        sample, discovered_failures, _types = oracle.discover_sample(
            self.row, self.sample_path, oracle.DEFAULT_MAX_ENCODED_BYTES, oracle.DEFAULT_MAX_BITMAP_BYTES
        )
        for image in sample["images"]:
            image.update(
                decoder_result="PASS",
                raw_stride_sha256="1" * 64,
                visible_bits_sha256="2" * 64,
            )
        manifest = {
            "schema_version": 1,
            "corpus_matrix_sha256": oracle.sha256_file(self.matrix_path),
            "oracle": oracle.oracle_metadata(20),
            "samples": [sample],
            "discovery_failures": discovered_failures if failures is None else failures,
        }
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    def test_c8_long_hn_short_hn_and_chained_records(self) -> None:
        for variant in ("C8", "HN-long", "HN-short"):
            with self.subTest(variant=variant):
                data, positions = container(variant)
                actual_variant = "C8" if variant == "C8" else "HN"
                row = self.write_sample(data, actual_variant)
                sample, failures, types = oracle.discover_sample(row, self.sample_path, 1024, 1024)
                self.assertEqual(failures, [])
                self.assertEqual(types, {2: 1, 0: 1})
                self.assertEqual(len(sample["images"]), 1)
                image = sample["images"][0]
                self.assertEqual((image["page"], image["image"]), (1, 2))
                self.assertEqual(image["offset"], positions["image_at"])
                self.assertEqual(image["encoded_sha256"], sha256(dib()))
                self.assertEqual((image["width"], image["height"], image["stride"]), (9, 2, 4))

    def test_zero_image_count_and_final_suffix(self) -> None:
        data, positions = container(multi_image=False)
        content = bytearray(data)
        struct.pack_into("<h", content, positions["table_at"] + 8, 0)
        row = self.write_sample(bytes(content))
        sample, failures, types = oracle.discover_sample(row, self.sample_path, 1024, 1024)
        self.assertEqual((sample["images"], failures, types), ([], [], {}))
        self.row = row
        self.write_matrix()
        self.write_manifest()
        fake_lib = self.root / "stub.so"
        fake_lib.write_bytes(b"not loaded by this zero-work status test")
        real_hash = oracle.sha256_file

        def pinned_hash(path: Path) -> str:
            return oracle.PINNED_LIBRARY_SHA256 if path == fake_lib else real_hash(path)

        with patch.object(oracle, "sha256_file", side_effect=pinned_hash):
            report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path, fake_lib)
        self.assertEqual(report["discovery"]["status"], "PASS")
        self.assertEqual(report["decode"]["status"], "NOT_RUN")
        self.assertEqual(report["status"], "NOT_RUN")

    def test_negative_signed_values_and_truncated_record_fail_closed(self) -> None:
        mutations = (
            ("text offset", self.positions["table_at"], -1, "page_index"),
            ("text length", self.positions["table_at"] + 4, -1, "page_index"),
            ("image count", self.positions["table_at"] + 8, -1, "page_index"),
            ("image type", self.positions["record_at"], -1, "image_record"),
            ("image offset", self.positions["record_at"] + 4, -1, "image_record"),
            ("image length", self.positions["record_at"] + 8, -1, "image_record"),
        )
        for field, at, value, kind in mutations:
            with self.subTest(field=field):
                content = bytearray(self.content)
                struct.pack_into("<h" if field == "image count" else "<i", content, at, value)
                row = self.write_sample(bytes(content))
                _sample, failures, _types = oracle.discover_sample(row, self.sample_path, 1024, 1024)
                self.assertEqual(len(failures), 1)
                self.assertEqual(failures[0]["kind"], kind)
                if kind == "image_record":
                    self.assertEqual(failures[0]["image"], 1)
        truncated = self.content[: self.positions["record_at"] + 6]
        row = self.write_sample(truncated)
        _sample, failures, _types = oracle.discover_sample(row, self.sample_path, 1024, 1024)
        self.assertEqual(failures[0]["kind"], "image_record")

    def test_type0_dib_dimensions_bits_and_allocation_limit(self) -> None:
        for width, height, bits, compression, palette, limit, fragment in (
            (-1, 2, 1, 0, None, 1024, "positive"),
            (9, -2, 1, 0, None, 1024, "positive"),
            (9, 2, 8, 0, None, 1024, "one bit"),
            (9, 2, 1, 1, None, 1024, "compression"),
            (9, 2, 1, 0, b"\x00" * 8, 1024, "palette"),
            (9, 2, 1, 0, None, 7, "exceeds limit"),
        ):
            with self.subTest(width=width, height=height, bits=bits, compression=compression, limit=limit):
                with self.assertRaisesRegex(oracle.OracleError, fragment):
                    if palette is None:
                        oracle.validate_image_dimensions(dib(width, height, bits, compression)[:48], limit)
                    else:
                        oracle.validate_image_dimensions(
                            dib(width, height, bits, compression, palette)[:48], limit
                        )

    def test_visible_bits_hash_masks_unused_bits_and_skips_padding(self) -> None:
        raw = b"\xff\xff\x11\x22\x80\x7f\x33\x44"
        raw_hash, visible_hash = oracle.bitmap_hashes(raw, 9, 2, 4)
        self.assertEqual(raw_hash, sha256(raw))
        self.assertEqual(visible_hash, sha256(b"\xff\x80\x80\x00"))
        self.assertEqual(oracle.bitmap_hashes(memoryview(raw), 9, 2, 4), (raw_hash, visible_hash))

    def test_clean_ci_not_run_and_requested_missing_paths_fail(self) -> None:
        report, observed = oracle.run(self.matrix_path, None, self.manifest_path)
        self.assertIsNone(observed)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["inventory"]["passed"], 0)
        self.assertEqual(report["decode"]["not_run"], 1)
        report, _ = oracle.run(self.matrix_path, self.root / "absent", self.manifest_path)
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["inventory"]["status"], "FAIL")
        report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path)
        self.assertEqual(report["inventory"]["status"], "PASS")
        self.assertEqual(report["discovery"]["status"], "PASS")
        self.assertEqual(report["discovery"]["image_type_counts"], {"0": 1, "2": 1})
        self.assertEqual(report["decode"]["status"], "NOT_RUN")
        report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path, self.root / "absent.so")
        self.assertEqual((report["status"], report["decode"]["status"]), ("FAIL", "FAIL"))
        report, _ = oracle.run(
            self.matrix_path, None, self.manifest_path, self.root / "absent.so"
        )
        self.assertEqual((report["status"], report["decode"]["status"]), ("FAIL", "FAIL"))

    def test_manifest_is_required_and_matrix_sha_checked_without_corpus(self) -> None:
        with self.assertRaisesRegex(oracle.OracleError, "cannot read JBIG1 manifest"):
            oracle.run(self.matrix_path, None, self.root / "absent.json")
        with self.assertRaisesRegex(oracle.OracleError, "timeout"):
            oracle.run(self.matrix_path, None, self.manifest_path, timeout_seconds=0)
        self.matrix_path.write_text(self.matrix_path.read_text() + " ", encoding="utf-8")
        with self.assertRaisesRegex(oracle.OracleError, "matrix SHA-256 differs"):
            oracle.run(self.matrix_path, None, self.manifest_path)

    def test_manifest_validation_and_exact_discovery_failure_comparison(self) -> None:
        manifest = oracle.load_manifest(self.manifest_path)
        manifest["oracle"]["compiler_sha256"] = "0" * 64
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaisesRegex(oracle.OracleError, "compiler_sha256"):
            oracle.load_manifest(self.manifest_path)
        self.write_manifest()
        manifest = oracle.load_manifest(self.manifest_path)
        observed = json.loads(json.dumps(manifest))
        observed["discovery_failures"] = [
            oracle.discovery_failure("sample", 1, "image_record", "out of bounds", 2)
        ]
        self.assertIn("discovery failures differ", oracle.compare_discovery(observed, manifest)[0])

    def test_no_corpus_rejects_truncated_or_wrong_sample_manifest(self) -> None:
        original = oracle.load_manifest(self.manifest_path)
        for change, fragment in (
            (lambda item: item["samples"][0].update(path="wrong.c8"), "path or SHA-256"),
            (lambda item: item["samples"][0].update(source_sha256="0" * 64), "path or SHA-256"),
            (lambda item: item["samples"][0].update(id="wrong"), "sample IDs"),
        ):
            with self.subTest(fragment=fragment):
                manifest = json.loads(json.dumps(original))
                change(manifest)
                self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
                with self.assertRaisesRegex(oracle.OracleError, fragment):
                    oracle.run(self.matrix_path, None, self.manifest_path)
        manifest = json.loads(json.dumps(original))
        manifest["samples"][0]["images"].clear()
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        with patch.object(oracle, "DEFAULT_MATRIX", self.matrix_path), patch.object(
            oracle, "DEFAULT_MANIFEST", self.manifest_path
        ), patch.object(oracle, "BASELINE_TYPE0_IMAGES", 1), patch.object(
            oracle, "BASELINE_INVALID_RECORDS", 0
        ):
            with self.assertRaisesRegex(oracle.OracleError, "type-0 images"):
                oracle.run(self.matrix_path, None, self.manifest_path)
            for decoder_result in ("NOT_RUN", "FAIL"):
                with self.subTest(decoder_result=decoder_result):
                    manifest = json.loads(json.dumps(original))
                    image = manifest["samples"][0]["images"][0]
                    image.update(
                        decoder_result=decoder_result,
                        raw_stride_sha256=None,
                        visible_bits_sha256=None,
                    )
                    self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
                    with self.assertRaisesRegex(oracle.OracleError, "all decoded images PASS"):
                        oracle.run(self.matrix_path, None, self.manifest_path)

    def test_default_profile_requires_expected_invalid_record_count(self) -> None:
        with patch.object(oracle, "DEFAULT_MATRIX", self.matrix_path), patch.object(
            oracle, "DEFAULT_MANIFEST", self.manifest_path
        ), patch.object(oracle, "BASELINE_TYPE0_IMAGES", 1), patch.object(
            oracle, "BASELINE_INVALID_RECORDS", 1
        ):
            with self.assertRaisesRegex(oracle.OracleError, "expected invalid records"):
                oracle.run(self.matrix_path, None, self.manifest_path)

    def test_expected_malformed_record_passes_baseline_without_extra_pixel_pass(self) -> None:
        content = bytearray(self.content)
        second_image_end = self.positions["image_at"] + len(dib())
        struct.pack_into("<h", content, self.positions["table_at"] + 8, 3)
        struct.pack_into("<iii", content, second_image_end, 0, -1, 10)
        self.row = self.write_sample(bytes(content))
        self.write_matrix()
        self.write_manifest()
        manifest = oracle.load_manifest(self.manifest_path)
        self.assertEqual(len(manifest["discovery_failures"]), 1)
        report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path)
        self.assertEqual(report["discovery"]["status"], "PASS")
        self.assertEqual(report["discovery"]["expected_invalid_records"], 1)
        self.assertEqual(report["discovery"]["failed"], 0)
        self.assertEqual(len(report["discovery"]["invalid_records"]), 1)
        self.assertEqual(report["decode"]["passed"], 0)
        self.assertEqual(report["decode"]["not_run"], 1)
        fake_lib = self.root / "stub.so"
        fake_lib.write_bytes(b"not loaded by this status-only test")
        real_hash = oracle.sha256_file

        def pinned_hash(path: Path) -> str:
            return oracle.PINNED_LIBRARY_SHA256 if path == fake_lib else real_hash(path)

        with patch.object(oracle, "sha256_file", side_effect=pinned_hash), patch.object(
            oracle,
            "decode_image",
            return_value={
                "status": "PASS", "raw_stride_sha256": "1" * 64,
                "visible_bits_sha256": "2" * 64,
            },
        ):
            report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path, fake_lib)
        self.assertEqual(report["status"], "PASS")
        self.assertEqual((report["decode"]["passed"], report["decode"]["expected"]), (1, 1))
        manifest["discovery_failures"][0]["detail"] = "different malformed record"
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        report, _ = oracle.run(self.matrix_path, self.corpus, self.manifest_path)
        self.assertEqual(report["discovery"]["status"], "FAIL")
        self.assertEqual(report["status"], "FAIL")

    def test_two_independent_prefills_and_mismatch(self) -> None:
        sample, _failures, _types = oracle.discover_sample(self.row, self.sample_path, 1024, 1024)
        image = sample["images"][0]
        seen: list[int] = []

        def stub(arguments: dict, _timeout: float) -> dict:
            seen.append(arguments["prefill"])
            return {
                "status": "PASS",
                "raw_stride_sha256": "1" * 64 if arguments["prefill"] == 0 else "2" * 64,
                "visible_bits_sha256": "3" * 64,
            }

        with patch.object(oracle, "worker_process", side_effect=stub):
            result = oracle.decode_image(self.root / "lib.so", self.sample_path, image, 2, 1024, 1024)
        self.assertEqual(seen, [0, 0xA5])
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("prefill", result["reason"])
        seen.clear()
        with patch.object(oracle, "worker_process", return_value={
            "status": "PASS", "raw_stride_sha256": "1" * 64,
            "visible_bits_sha256": "3" * 64,
        }) as worker:
            result = oracle.decode_image(self.root / "lib.so", self.sample_path, image, 2, 1024, 1024)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual([call.args[0]["prefill"] for call in worker.call_args_list], [0, 0xA5])

    def test_worker_timeout_crash_and_invalid_response(self) -> None:
        with patch.object(oracle.subprocess, "run", side_effect=subprocess.TimeoutExpired("worker", 2)):
            result = oracle.worker_process({}, 2)
        self.assertIn("timed out", result["reason"])
        with patch.object(oracle.subprocess, "run", return_value=SimpleNamespace(returncode=-11, stdout="")):
            result = oracle.worker_process({}, 2)
        self.assertIn("signal 11", result["reason"])
        with patch.object(oracle.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout="no JSON")):
            result = oracle.worker_process({}, 2)
        self.assertIn("without a JSON result", result["reason"])

    def test_cli_json_exit_codes_and_env_path_defaults(self) -> None:
        with patch.dict("os.environ", {}, clear=True), redirect_stdout(io.StringIO()) as output:
            code = oracle.main(["--matrix", str(self.matrix_path), "--manifest", str(self.manifest_path), "--json"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")
        with redirect_stdout(io.StringIO()) as output:
            code = oracle.main([
                "--matrix", str(self.matrix_path), "--manifest", str(self.manifest_path),
                "--corpus-dir", str(self.root / "absent"), "--json",
            ])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")
        with redirect_stdout(io.StringIO()) as output:
            code = oracle.main([
                "--matrix", str(self.matrix_path), "--manifest", str(self.manifest_path),
                "--corpus-dir", str(self.corpus), "--oracle-lib", str(self.root / "absent.so"),
                "--json",
            ])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["decode"]["status"], "FAIL")
        with redirect_stdout(io.StringIO()) as output:
            code = oracle.main([
                "--matrix", str(self.matrix_path), "--manifest", str(self.manifest_path),
                "--oracle-lib", str(self.root / "absent.so"), "--json",
            ])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["decode"]["status"], "FAIL")
        with patch.dict("os.environ", {"CAJ2PDF_CORPUS_DIR": str(self.corpus)}, clear=True), redirect_stdout(io.StringIO()) as output:
            code = oracle.main(["--matrix", str(self.matrix_path), "--manifest", str(self.manifest_path), "--json"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output.getvalue())["inventory"]["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
