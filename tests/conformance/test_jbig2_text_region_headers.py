# SPDX-License-Identifier: MIT
"""Synthetic and clean-clone checks for the optional text-region inventory."""

from __future__ import annotations

import contextlib
from io import StringIO
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_text_region_headers as inventory  # noqa: E402


def text_data(flags: int, instances: int, width: int = 64, height: int = 32,
              x: int = 0, body: bytes = b"\xa5\x5a") -> bytes:
    region = b"".join(value.to_bytes(4, "big") for value in (width, height, x, 0))
    return region + b"\x00" + flags.to_bytes(2, "big") + instances.to_bytes(4, "big") + body


class TextRegionHeaderInventoryTests(unittest.TestCase):
    def test_clean_clone_is_not_text_compatibility(self) -> None:
        report = inventory.run(None)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["metadata"]["checked_images"], 0)
        self.assertEqual(report["text_compatibility"]["checked_cases"], 0)
        self.assertEqual(report["text_compatibility"]["passed"], 0)
        self.assertEqual((report["source_hashes_before"], report["source_hashes_after"]), (0, 0))

    def test_explicit_missing_corpus_fails_before_directory_read(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = StringIO()
            with contextlib.redirect_stdout(output), mock.patch.object(
                inventory.full, "run_directory_inventory",
                side_effect=AssertionError("directory read occurred before source audit"),
            ):
                code = inventory.main(["--corpus-dir", str(Path(directory) / "missing"), "--json"])
            self.assertEqual(code, 1)
            report = json.loads(output.getvalue())
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["metadata"]["checked_images"], 0)

    def test_committed_oracle_matches_pinned_flag_profile(self) -> None:
        rows, recorded = inventory.load_baseline()
        self.assertEqual(len(rows), 27)
        self.assertEqual(len(recorded), 546)
        self.assertEqual(sum(inventory.EXPECTED_FLAGS.values()), 546)
        anomaly = recorded[inventory.full.ANOMALY_COORDINATE]
        self.assertEqual((anomaly["offset"], anomaly["profile"]["text_flags"]), (930673, "0xa40c"))
        with mock.patch.dict(inventory.EXPECTED_FLAGS, {"0x840e": 2}):
            with self.assertRaisesRegex(inventory.InventoryError, "distribution differs"):
                inventory.load_baseline()

    def test_header_reads_only_observed_layout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invented.dat"
            data = text_data(0x900E, 6)
            source.write_bytes(b"pad" + data)
            case = {"id": "synthetic", "page": 1, "image": 1, "source_path": source,
                    "width": 64, "height": 32}
            segment = {"data_offset": 3, "data_length": len(data)}
            self.assertEqual(inventory.parse_text_header(case, segment), {
                "flags": "0x900e", "refinement": 1, "instances": 6, "header_bytes": 23,
                "data_offset": 3, "data_length": 25, "body_offset": 26, "body_length": 2,
            })
            anomaly = text_data(0xA40C, 9)
            source.write_bytes(anomaly)
            header = inventory.parse_text_header(case, {"data_offset": 0, "data_length": 25})
            self.assertEqual((header["flags"], header["refinement"]), ("0xa40c", 0))
            for flags, message in ((0x0001, "outside the observed layout"),
                                   (0x000E, "outside the observed layout")):
                source.write_bytes(text_data(flags, 6))
                with self.assertRaisesRegex(inventory.InventoryError, message):
                    inventory.parse_text_header(case, {"data_offset": 0, "data_length": 25})
            source.write_bytes(text_data(0x900E, 6, x=1))
            with self.assertRaisesRegex(inventory.InventoryError, "cover the page"):
                inventory.parse_text_header(case, {"data_offset": 0, "data_length": 25})
            with self.assertRaisesRegex(inventory.InventoryError, "declared data size"):
                inventory.parse_text_header(case, {"data_offset": 0, "data_length": 24})
            with self.assertRaisesRegex(inventory.full.OracleError, "source changed or ended"):
                inventory.parse_text_header(case, {"data_offset": 10, "data_length": 25})

    def test_case_must_match_oracle_span_segment_and_flags(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invented.dat"
            source.write_bytes(text_data(0x900E, 6))
            segment = {"type": 6, "refs": [2], "page_association": 1,
                       "data_offset": 0, "data_length": 25}
            case = {"id": "synthetic", "page": 1, "image": 1, "source_path": source,
                    "width": 64, "height": 32, "offset": 0, "length": 25,
                    "segments": [None, None, None, segment]}
            recorded = {"offset": 0, "length": 25, "profile": {"text_flags": "0x900e"}}
            self.assertEqual(inventory.inspect_case(case, recorded)["instances"], 6)
            with self.assertRaisesRegex(inventory.InventoryError, "text flags differ"):
                inventory.inspect_case(case, {**recorded, "profile": {"text_flags": "0x880e"}})
            with self.assertRaisesRegex(inventory.InventoryError, "image span differs"):
                inventory.inspect_case(case, {**recorded, "length": 26})
            segment["refs"] = [1]
            with self.assertRaisesRegex(inventory.InventoryError, "not a text region"):
                inventory.inspect_case(case, recorded)

    def test_aggregate_reports_distribution_ranges_and_anomaly(self) -> None:
        def header(flags: str, instances: int, length: int) -> dict:
            return {"flags": flags, "refinement": int(flags, 16) >> 1 & 1,
                    "instances": instances, "header_bytes": 23,
                    "data_length": length, "body_length": length - 23}
        summary = inventory.aggregate([
            (("a", 1, 1), header("0x900e", 6, 64), 10),
            (("a", 2, 1), header("0xa40c", 20, 90), 930673),
            (("b", 1, 1), header("0x900e", 7, 100), 30),
        ])
        self.assertEqual(summary, {
            "headers": 3, "flags": {"0x900e": 2, "0xa40c": 1},
            "header_bytes": {"23": 3}, "refinement": {"0": 1, "1": 2},
            "instance_range": [6, 20], "instance_sum": 33,
            "data_byte_range": [64, 100], "body_byte_range": [41, 77],
            "anomalies": [{"id": "a", "page": 2, "image": 1,
                           "record_offset": 930673, "flags": "0xa40c"}],
        })

    def _run_with(self, headers: dict, audit=None) -> tuple[dict, Exception | None]:
        _, recorded = inventory.load_baseline()
        cases = [{"coordinate": key, "offset": image["offset"]} for key, image in recorded.items()]
        report = inventory.initial_report()
        audit = audit or (lambda rows, corpus: corpus)
        with mock.patch.object(inventory, "audit_sources", side_effect=audit), \
             mock.patch.object(inventory.full, "run_directory_inventory", return_value={}), \
             mock.patch.object(inventory.full, "checked_cases", return_value=cases), \
             mock.patch.object(inventory, "inspect_case",
                               side_effect=lambda case, _: headers[case["coordinate"]]), \
             mock.patch.object(inventory.conformance, "contained_file",
                               return_value=Path("/tmp/synthetic-source")):
            try:
                return inventory.run(Path("/tmp"), report=report), None
            except inventory.InventoryError as exc:
                return report, exc

    def _pinned_headers(self) -> dict:
        """Invented per-image values that reproduce every pinned aggregate."""
        _, recorded = inventory.load_baseline()
        keys = sorted(recorded)
        low, high = inventory.EXPECTED_AGGREGATES["instance_range"]
        instances = [low] * len(keys)
        remaining = inventory.EXPECTED_AGGREGATES["instance_sum"] - sum(instances)
        for index in range(1, len(keys)):
            added = min(remaining, high - low)
            instances[index] += added
            remaining -= added
        lengths = [64] * len(keys)
        lengths[1] = 28634
        headers = {}
        for index, key in enumerate(keys):
            flags = recorded[key]["profile"]["text_flags"]
            headers[key] = {"flags": flags, "refinement": int(flags, 16) >> 1 & 1,
                            "instances": instances[index], "header_bytes": 23,
                            "data_length": lengths[index], "body_length": lengths[index] - 23}
        return headers

    def test_pinned_aggregates_pass_and_drift_fails(self) -> None:
        headers = self._pinned_headers()
        report, error = self._run_with(headers)
        self.assertIsNone(error)
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["metadata"]["checked_images"], 546)
        self.assertEqual(report["metadata"]["aggregates"], inventory.EXPECTED_AGGREGATES)
        self.assertEqual((report["source_hashes_before"], report["source_hashes_after"]), (27, 27))
        self.assertEqual(report["text_compatibility"]["checked_cases"], 0)
        key = next(iter(headers))
        headers[key] = {**headers[key], "instances": headers[key]["instances"] + 1}
        report, error = self._run_with(headers)
        self.assertRegex(str(error), "aggregates differ")
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["metadata"]["checked_images"], 0)

    def test_posthash_failure_cannot_leave_metadata_pass(self) -> None:
        calls = 0

        def changed_after(rows: list[dict], corpus: Path) -> Path:
            nonlocal calls
            calls += 1
            if calls == 2:
                raise inventory.InventoryError("source changed after inventory")
            return corpus

        report, error = self._run_with(self._pinned_headers(), changed_after)
        self.assertRegex(str(error), "post-run source audit")
        self.assertEqual(calls, 2)
        self.assertEqual(report["metadata"]["status"], "FAIL")
        self.assertEqual(report["source_hashes_after"], 0)


if __name__ == "__main__":
    unittest.main()
