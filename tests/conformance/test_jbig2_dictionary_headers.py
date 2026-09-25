# SPDX-License-Identifier: MIT
"""Synthetic and clean-clone checks for the optional dictionary inventory."""

from __future__ import annotations

import contextlib
import hashlib
from io import StringIO
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_dictionary_headers as inventory  # noqa: E402


class DictionaryHeaderInventoryTests(unittest.TestCase):
    def test_clean_clone_is_not_symbol_compatibility(self) -> None:
        report = inventory.run(None)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["metadata"]["checked_images"], 0)
        self.assertEqual(report["symbol_compatibility"], {
            "status": "NOT_RUN", "checked_cases": 0, "passed": 0, "failed": 0,
            "reason": "no independent per-symbol pixel oracle or private Table E.1 fixture",
        })
        self.assertEqual((report["source_hashes_before"], report["source_hashes_after"]), (0, 0))

    def test_explicit_missing_corpus_fails_before_directory_read(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing"
            output = StringIO()
            with contextlib.redirect_stdout(output), mock.patch.object(
                inventory.full, "run_directory_inventory",
                side_effect=AssertionError("directory read occurred before source audit"),
            ):
                self.assertEqual(inventory.main(["--corpus-dir", str(missing), "--json"]), 1)
            report = json.loads(output.getvalue())
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["metadata"]["checked_images"], 0)
            self.assertEqual(report["symbol_compatibility"]["checked_cases"], 0)

    def test_committed_metadata_has_only_expected_fields_and_rejects_drift(self) -> None:
        rows, baseline = inventory.load_baseline()
        self.assertEqual(len(rows), 27)
        self.assertEqual(sum(len(sample["images"]) for sample in baseline["samples"]), 546)
        summary = inventory.aggregate([
            image for sample in baseline["samples"] for image in sample["images"]
        ])
        self.assertEqual(summary["1"]["flags"], {"0x0800": 546})
        self.assertEqual(summary["1"]["adaptive_templates"], {"2,-1": 546})
        self.assertEqual(summary["1"]["new_symbol_count_range"], [3, 514])
        self.assertEqual(summary["1"]["data_byte_range"], [60, 9959])
        self.assertEqual(summary["2"]["flags"], {"0x1802": 546})
        self.assertEqual(summary["2"]["new_symbol_count_range"], [0, 318])
        self.assertEqual(summary["2"]["zero_new_count"], 57)
        oracle = json.loads(inventory.FULL_ORACLE.read_text(encoding="utf-8"))
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["dictionaries"][0]["at"] = [3, -1]
        with self.assertRaisesRegex(inventory.InventoryError, "mode/AT differs"):
            inventory.validate_manifest(changed, rows, oracle)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["dictionaries"][0]["encoded_bytes"] = "00"
        with self.assertRaisesRegex(inventory.InventoryError, "unexpected fields"):
            inventory.validate_manifest(changed, rows, oracle)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["dictionaries"][0]["huffman"] = False
        with self.assertRaisesRegex(inventory.InventoryError, "integer"):
            inventory.validate_manifest(changed, rows, oracle)
        with tempfile.TemporaryDirectory() as directory:
            changed_path = Path(directory) / "dictionary-headers.json"
            changed_path.write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaisesRegex(inventory.InventoryError, "manifest SHA-256 differs"):
                inventory.load_baseline(changed_path)

    def test_header_reads_only_declared_observed_profile(self) -> None:
        first = b"\x08\x00\x02\xff" + (14).to_bytes(4, "big") * 2 + b"abc"
        second = b"\x18\x02\x02\xff" + (16).to_bytes(4, "big") + (2).to_bytes(4, "big") + b"xyz"
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invented.dat"
            source.write_bytes(first + second)
            case = {"id": "synthetic", "page": 1, "image": 1, "source_path": source}
            one = inventory.parse_dictionary_header(
                case, {"data_offset": 0, "data_length": len(first), "refs": []}, 1,
            )
            two = inventory.parse_dictionary_header(
                case, {"data_offset": len(first), "data_length": len(second), "refs": [1]}, 2,
            )
            self.assertEqual((one["flags"], one["at"], one["exported"], one["new"]),
                             (0x0800, [2, -1], 14, 14))
            self.assertEqual((two["flags"], two["at"], two["exported"], two["new"]),
                             (0x1802, [2, -1], 16, 2))
            self.assertEqual(one["data_sha256"], hashlib.sha256(first).hexdigest())
            self.assertEqual(two["data_sha256"], hashlib.sha256(second).hexdigest())
            source.write_bytes(b"\x18\x02" + first[2:] + second)
            with self.assertRaisesRegex(inventory.InventoryError, "flags"):
                inventory.parse_dictionary_header(
                    case, {"data_offset": 0, "data_length": len(first), "refs": []}, 1,
                )

    def test_posthash_failure_cannot_leave_metadata_pass(self) -> None:
        report = inventory.initial_report()
        rows, baseline = inventory.load_baseline()
        audit_calls = 0

        def changed_after(rows_arg: list[dict], corpus: Path) -> Path:
            nonlocal audit_calls
            audit_calls += 1
            if audit_calls == 2:
                raise inventory.InventoryError("source changed after inventory")
            return corpus

        expected = {
            (sample["id"], image["page"], image["image"]): image
            for sample in baseline["samples"] for image in sample["images"]
        }
        fake_cases = [{"coordinate": key} for key in expected]
        with mock.patch.object(inventory, "audit_sources", side_effect=changed_after), \
             mock.patch.object(inventory.full, "run_directory_inventory", return_value={}), \
             mock.patch.object(inventory.full, "checked_cases", return_value=fake_cases), \
             mock.patch.object(inventory, "inspect_case",
                               side_effect=lambda case: expected[case["coordinate"]]), \
             mock.patch.object(inventory.conformance, "contained_file",
                               return_value=Path("/tmp/synthetic-source")):
            with self.assertRaisesRegex(inventory.InventoryError, "post-run source audit"):
                inventory.run(Path("/tmp"), report=report)
        self.assertEqual(audit_calls, 2)
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["metadata"]["status"], "FAIL")
        self.assertEqual(report["metadata"]["checked_images"], 0)
        self.assertEqual(report["source_hashes_after"], 0)
        self.assertEqual(len(rows), 27)


if __name__ == "__main__":
    unittest.main()
