# SPDX-License-Identifier: MIT
"""Synthetic, source-independent tests for the finite T.82 CLI probe."""

from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig1_oracle as oracle  # noqa: E402
import jbig1_standard_probe as probe  # noqa: E402


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def blob_oid(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode("ascii") + data).hexdigest()


class StandardProbeTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self.matrix = self.root / "matrix.json"
        self.manifest = self.root / "manifest.json"
        self.source = self.corpus / "sample.c8"
        self.decoder = self.root / "fake_jbgtopbm.py"
        self.encoder = self.root / "fake_pbmtojbg.py"
        self.make_fixture()
        self.write_fake_clis()

    def make_fixture(self) -> None:
        dib = struct.pack("<IiiHHIIiiII", 40, 9, 2, 1, 1, 0, 0, 0, 0, 2, 0)
        encoded = dib + b"\xff\xff\xff\x00\x00\x00\x00\x00" + b"\x12\x34\x00\x00"
        content = bytearray(0x50 + 20)
        content[:4] = b"\xc8\x00\x00\x00"
        struct.pack_into("<i", content, 0x08, 1)
        struct.pack_into("<iihhii", content, 0x50, 0x64, 0, 1, 0, 0, 0)
        content.extend(b"\x00" * 12)
        image_offset = len(content)
        content.extend(encoded)
        struct.pack_into("<iii", content, 0x64, 0, image_offset, len(encoded))
        source_bytes = bytes(content)
        self.source.write_bytes(source_bytes)
        row = {
            "id": "sample", "path": "sample.c8", "aliases": [],
            "size_bytes": len(source_bytes), "git_blob_oid": blob_oid(source_bytes),
            "sha256": sha256(source_bytes), "detected_type": "C8", "variant": "C8",
            "expected_outcome": "success", "page_count": 1, "outline_count": 0,
            "python_reference": {"show_status": "success", "convert_status": "success"},
        }
        self.matrix.write_text(json.dumps({"schema_version": 1, "samples": [row]}), encoding="utf-8")
        image = {
            "page": 1, "image": 1, "offset": image_offset, "length": len(encoded),
            "encoded_sha256": sha256(encoded), "width": 9, "height": 2, "stride": 4,
            "decoder_result": "PASS", "raw_stride_sha256": sha256(bytes(8)),
            "visible_bits_sha256": sha256(bytes(4)),
        }
        manifest = {
            "schema_version": 1, "corpus_matrix_sha256": oracle.sha256_file(self.matrix),
            "oracle": oracle.oracle_metadata(20),
            "samples": [{"id": "sample", "path": "sample.c8", "source_sha256": row["sha256"],
                         "images": [image]}],
            "discovery_failures": [],
        }
        self.manifest.write_text(json.dumps(manifest), encoding="utf-8")

    def write_fake_clis(self) -> None:
        self.write_fake_decoder(bytes(4))
        self.encoder.write_text(
            "#!/usr/bin/env python3\n"
            "# SPDX-License-Identifier: MIT\n"
            "import sys\n"
            "if sys.argv[1:]!=['-q','-p','0','-m','0','-s','2','-o','0','-']: sys.exit(2)\n"
            "pbm=sys.stdin.buffer.read()\n"
            "if pbm!=b'P4\\n9 2\\n'+bytes(4): sys.exit(3)\n"
            "h=bytes([0,0,1,0])+ (9).to_bytes(4,'big')+(2).to_bytes(4,'big')"
            "+(2).to_bytes(4,'big')+bytes([0,0,0,0])\n"
            "sys.stdout.buffer.write(h+b'\\x12\\x34\\xff\\x02')\n",
            encoding="utf-8",
        )
        self.encoder.chmod(0o755)

    def write_fake_decoder(self, bitmap: bytes) -> None:
        self.decoder.write_text(
            "#!/usr/bin/env python3\n"
            "# SPDX-License-Identifier: MIT\n"
            "import sys\n"
            "data=sys.stdin.buffer.read()\n"
            "w=int.from_bytes(data[4:8],'big'); h=int.from_bytes(data[8:12],'big')\n"
            f"pixels=bytes.fromhex('{bitmap.hex()}')\n"
            "if len(pixels)!=((w+7)//8)*h: sys.exit(4)\n"
            "sys.stdout.buffer.write(f'P4\\n{w:9d}\\n{h:9d}\\n'.encode()+pixels)\n",
            encoding="utf-8",
        )
        self.decoder.chmod(0o755)

    def set_reference_hashes(self, visible: bytes, raw: bytes) -> None:
        manifest = json.loads(self.manifest.read_text(encoding="utf-8"))
        image = manifest["samples"][0]["images"][0]
        image["visible_bits_sha256"] = sha256(visible)
        image["raw_stride_sha256"] = sha256(raw)
        self.manifest.write_text(json.dumps(manifest), encoding="utf-8")

    def call_run(self, **changes: object) -> dict:
        arguments = {
            "corpus_dir": self.corpus, "decoder_path": self.decoder,
            "sample_id": "sample", "page": 1,
            "matrix_path": self.matrix, "manifest_path": self.manifest,
        }
        arguments.update(changes)
        return probe.run(**arguments)

    def test_finite_grid_and_blank_control_are_explicit(self) -> None:
        report = self.call_run(encoder_path=self.encoder)
        self.assertEqual(report["status"], "BLANK_MATCH_NON_DISCRIMINATING")
        self.assertEqual(report["decoder_probe"]["settings_count"], 64)
        self.assertEqual(report["decoder_probe"]["status_counts"], {
            "MATCH_NON_DISCRIMINATING": 64
        })
        self.assertTrue(report["decoder_probe"]["blank_reference"])
        self.assertEqual(report["encoded_sha256"], sha256(self.source.read_bytes()[0x70:]))
        self.assertEqual(report["expected_raw_stride_sha256"], sha256(bytes(8)))
        control = report["blank_control"]
        self.assertEqual(control["status"], "COMPLETE")
        self.assertEqual(control["scd_length"], 2)
        self.assertEqual(control["scd_sha256"], sha256(b"\x12\x34"))
        self.assertEqual(control["selected_coded_zero_tail_bytes"], 2)
        self.assertTrue(control["selected_coded_is_scd_plus_zero_tail"])
        self.assertEqual(report["decoder_sha256"], oracle.sha256_file(self.decoder))
        self.assertEqual(report["encoder_sha256"], oracle.sha256_file(self.encoder))

    def test_nonblank_reference_is_a_mismatch_not_a_blank_match(self) -> None:
        self.set_reference_hashes(b"\x80\x00\x00\x00", b"\x80\x00\x00\x00" + bytes(4))
        report = self.call_run()
        self.assertEqual(report["status"], "NO_MATCH_IN_TESTED_GRID")
        self.assertFalse(report["decoder_probe"]["blank_reference"])
        self.assertEqual(report["decoder_probe"]["status_counts"], {"HASH_MISMATCH": 64})
        self.assertEqual(report["blank_control"]["status"], "NOT_RUN")

    def test_nonblank_exact_match_is_only_a_finite_grid_result(self) -> None:
        self.set_reference_hashes(b"\x80\x00\x00\x00", b"\x80\x00\x00\x00" + bytes(4))
        self.write_fake_decoder(b"\x80\x00\x00\x00")
        report = self.call_run()
        self.assertEqual(report["status"], "MATCH_IN_TESTED_GRID")
        self.assertEqual(report["decoder_probe"]["status_counts"], {"MATCH": 64})
        self.assertFalse(report["decoder_probe"]["blank_reference"])
        first = report["decoder_probe"]["results"][0]
        self.assertEqual(first["matching_orientations"], ["direct"])
        self.assertEqual(first["candidate_hashes"]["direct"], {
            "visible_bits_sha256": sha256(b"\x80\x00\x00\x00"),
            "raw_stride_sha256": sha256(b"\x80\x00\x00\x00" + bytes(4)),
        })

    def test_reversed_row_order_requires_both_matching_hashes(self) -> None:
        self.set_reference_hashes(
            b"\x40\x00\x80\x00",
            b"\x40\x00\x00\x00\x80\x00\x00\x00",
        )
        self.write_fake_decoder(b"\x80\x00\x40\x00")
        report = self.call_run()
        self.assertEqual(report["status"], "MATCH_IN_TESTED_GRID")
        self.assertEqual(report["decoder_probe"]["status_counts"], {"MATCH": 64})
        first = report["decoder_probe"]["results"][0]
        self.assertEqual(first["matching_orientations"], ["reversed_rows"])
        self.assertEqual(first["candidate_hashes"]["direct"], {
            "visible_bits_sha256": sha256(b"\x80\x00\x40\x00"),
            "raw_stride_sha256": sha256(b"\x80\x00\x00\x00\x40\x00\x00\x00"),
        })
        self.assertEqual(first["candidate_hashes"]["reversed_rows"], {
            "visible_bits_sha256": sha256(b"\x40\x00\x80\x00"),
            "raw_stride_sha256": sha256(b"\x40\x00\x00\x00\x80\x00\x00\x00"),
        })

    def test_visible_match_with_different_unused_bits_is_not_full_match(self) -> None:
        self.set_reference_hashes(b"\x80\x00\x00\x00", b"\x80\x00" + bytes(6))
        self.write_fake_decoder(b"\x80\x7f\x00\x00")
        report = self.call_run()
        self.assertEqual(report["status"], "VISIBLE_ONLY_IN_TESTED_GRID")
        self.assertEqual(report["decoder_probe"]["status_counts"], {
            "VISIBLE_MATCH_RAW_MISMATCH": 64
        })
        first = report["decoder_probe"]["results"][0]
        self.assertEqual(first["visible_matching_orientations"], ["direct"])
        self.assertEqual(first["candidate_hashes"]["direct"]["visible_bits_sha256"],
                         sha256(b"\x80\x00\x00\x00"))
        self.assertEqual(first["candidate_hashes"]["direct"]["raw_stride_sha256"],
                         sha256(b"\x80\x7f" + bytes(6)))

    def test_visible_match_with_nonzero_oracle_padding_is_not_full_match(self) -> None:
        self.set_reference_hashes(b"\x80\x00\x00\x00", b"\x80\x00\x00\x01" + bytes(4))
        self.write_fake_decoder(b"\x80\x00\x00\x00")
        report = self.call_run()
        self.assertEqual(report["status"], "VISIBLE_ONLY_IN_TESTED_GRID")
        self.assertEqual(report["decoder_probe"]["status_counts"], {
            "VISIBLE_MATCH_RAW_MISMATCH": 64
        })
        first = report["decoder_probe"]["results"][0]
        self.assertEqual(first["visible_matching_orientations"], ["direct"])
        self.assertEqual(first["candidate_hashes"]["direct"]["raw_stride_sha256"],
                         sha256(b"\x80\x00" + bytes(6)))

    def test_blank_visible_only_match_is_explicitly_non_discriminating(self) -> None:
        self.set_reference_hashes(bytes(4), b"\x00\x00\x00\x01" + bytes(4))
        report = self.call_run()
        self.assertEqual(report["status"], "VISIBLE_ONLY_BLANK_NON_DISCRIMINATING")
        self.assertTrue(report["decoder_probe"]["blank_reference"])
        self.assertEqual(report["decoder_probe"]["status_counts"], {
            "VISIBLE_MATCH_RAW_MISMATCH": 64
        })
        self.assertNotIn("matching_orientations", report["decoder_probe"]["results"][0])

    def test_raw_pbm_header_accepts_whitespace_comments_and_preserves_raster(self) -> None:
        valid_headers = (
            b"P4\n9 2\n",
            b"P4 \t9\r\n2\v",
            b"P4\n# before width\n9 # between dimensions\r2# before raster\n",
            b"P4\n9 2# final comment\r",
        )
        for header in valid_headers:
            for raster in (b"\x20\x0a\x23\x09", b"\x23\x00\x20\x0a"):
                with self.subTest(header=header, raster=raster):
                    self.assertEqual(bytes(probe.parse_raw_pbm(header + raster, 9, 2)), raster)

    def test_raw_pbm_header_rejects_malformed_or_extra_image(self) -> None:
        raster = b"\x20\x0a\x23\x09"
        malformed = (
            b"P1\n9 2\n" + raster,
            b"P4\n9 x\n" + raster,
            b"P4\n0 2\n" + raster,
            b"P4\n10 2\n" + raster,
            b"P4\n9 2\n\n" + raster,
            b"P4\n9 2 # comment\n" + raster,
            b"P4\n9 2\n#xy\n" + raster,
            b"P4\n9 2\n" + raster + b"P4\n9 2\n" + raster,
            b"P4\n" + b" " * 1024 + b"9 2\n" + raster,
            b"P4\n9 2# unterminated" + raster,
        )
        for output in malformed:
            with self.subTest(output_length=len(output)):
                with self.assertRaises(probe.ProbeError):
                    probe.parse_raw_pbm(output, 9, 2)

    def test_source_and_encoded_hashes_are_checked_before_cli(self) -> None:
        self.source.write_bytes(self.source.read_bytes()[:-1] + b"X")
        with self.assertRaisesRegex(probe.ProbeError, "source bytes differ"):
            self.call_run()
        self.make_fixture()
        manifest = json.loads(self.manifest.read_text())
        manifest["samples"][0]["images"][0]["encoded_sha256"] = "0" * 64
        self.manifest.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaisesRegex(probe.ProbeError, "encoded SHA-256"):
            self.call_run()

    def test_bad_pbm_and_timeout_are_reported_per_setting(self) -> None:
        image, coded = probe.selected_image(
            self.corpus, self.matrix, self.manifest, "sample", 1, 1
        )
        with patch.object(probe, "call_cli", return_value=(0, b"bad PBM")):
            report = probe.decoder_probe(self.decoder, coded, image, 1)
        self.assertEqual(report["status_counts"], {"BAD_PBM": 64})
        self.assertEqual(report["status"], "FAIL")
        for payload in (
            b"P4\n1000000000\n2\n",
            b"P4\n" + b"9" * 100 + b"\n2\n",
            b"P4\n9\n2\n" + bytes(2000),
        ):
            with self.subTest(payload_length=len(payload)), patch.object(
                probe, "call_cli", return_value=(0, payload)
            ):
                report = probe.decoder_probe(self.decoder, coded, image, 1)
            self.assertEqual(report["status_counts"], {"BAD_PBM": 64})
            self.assertEqual(report["status"], "FAIL")
        with patch.object(probe, "call_cli", return_value=(None, None)):
            report = probe.decoder_probe(self.decoder, coded, image, 1)
        self.assertEqual(report["status_counts"], {"TIMEOUT": 64})
        self.assertEqual(report["status"], "FAIL")
        with patch.object(probe, "call_cli", return_value=(1, b"")):
            report = probe.decoder_probe(self.decoder, coded, image, 1)
        self.assertEqual(report["status_counts"], {"DECODE_ERROR": 64})
        self.assertEqual(report["status"], "INCONCLUSIVE_NO_DECODABLE_SETTINGS")

    def test_cli_output_is_spooled_and_loaded_only_below_limit(self) -> None:
        bie = probe.standard_bie(9, 2, 2, 0, 0, b"\x12\x34", False, b"\xff\x02")
        code, output = probe.call_cli(self.decoder, ["-"], bie, 1, 10)
        self.assertIsNotNone(code)
        self.assertIsNone(output)
        self.decoder.write_text(
            "#!/usr/bin/env python3\n"
            "# SPDX-License-Identifier: MIT\n"
            "import sys\n"
            "sys.stdout.buffer.write(bytes(1_000_000))\n",
            encoding="utf-8",
        )
        code, output = probe.call_cli(self.decoder, [], b"", 1, 10)
        self.assertIsNotNone(code)
        self.assertIsNone(output)
        with patch.object(probe, "MAX_STDOUT_BYTES", 128):
            with self.assertRaisesRegex(probe.ProbeError, "input exceeds"):
                probe.call_cli(self.decoder, ["-"], bytes(129), 1, 10)

    def test_oversized_pbm_is_rejected_before_decoder_launch(self) -> None:
        image, coded = probe.selected_image(
            self.corpus, self.matrix, self.manifest, "sample", 1, 1
        )
        with patch.object(probe, "MAX_STDOUT_BYTES", 1027), patch.object(
            probe, "call_cli"
        ) as cli:
            with self.assertRaisesRegex(probe.ProbeError, "PBM output exceeds"):
                probe.decoder_probe(self.decoder, coded, image, 1)
        cli.assert_not_called()

    def test_clean_not_run_and_incomplete_or_missing_request_fail(self) -> None:
        with redirect_stdout(io.StringIO()) as output:
            code = probe.main(["--json"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")
        with redirect_stdout(io.StringIO()) as output:
            code = probe.main(["--decoder", str(self.decoder), "--json"])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")
        with patch.object(probe, "call_cli", return_value=(1, b"")):
            with redirect_stdout(io.StringIO()) as output:
                code = probe.main([
                    "--corpus-dir", str(self.corpus), "--decoder", str(self.decoder),
                    "--sample-id", "sample", "--page", "1", "--matrix", str(self.matrix),
                    "--manifest", str(self.manifest), "--json",
                ])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["status"],
                         "INCONCLUSIVE_NO_DECODABLE_SETTINGS")
        for arguments in (
            ["--page", "0"], ["--image", "2"], ["--timeout-seconds", "2"],
            ["--matrix", str(self.matrix)],
        ):
            with self.subTest(arguments=arguments), redirect_stdout(io.StringIO()) as output:
                code = probe.main([*arguments, "--json"])
            self.assertEqual(code, 1)
            self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")
        with self.assertRaisesRegex(probe.ProbeError, "missing"):
            self.call_run(decoder_path=self.root / "missing")
        with self.assertRaisesRegex(probe.ProbeError, "missing"):
            self.call_run(encoder_path=self.root / "missing")
        with self.assertRaisesRegex(probe.ProbeError, "corpus directory is missing"):
            self.call_run(corpus_dir=self.root / "missing")
        with redirect_stdout(io.StringIO()) as output:
            code = probe.main([
                "--corpus-dir", str(self.root / "missing"), "--decoder", str(self.decoder),
                "--sample-id", "sample", "--page", "1", "--matrix", str(self.matrix),
                "--manifest", str(self.manifest), "--json",
            ])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")


if __name__ == "__main__":
    unittest.main()
