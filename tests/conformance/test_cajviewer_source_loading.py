# SPDX-License-Identifier: MIT
"""Original module/receipt controls; no child, Docker, X11 or vendor action."""

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
RUNNER_PATH = ROOT / "tools/cajviewer/run.py"
spec = importlib.util.spec_from_file_location("original_source_loading_runner", RUNNER_PATH)
DRIVER = importlib.util.module_from_spec(spec)
spec.loader.exec_module(DRIVER)
CONTRACT_SPECS = (("cajviewer_canary", "process-helper", "/opt/canary/cajviewer_canary.py"),
                  ("inventory", "runtime-inventory", "/opt/canary/inventory.py"))
CONTRACT_STAGES = ("read", "pin", "compile", "exec")
CONTRACT_REASONS = {"read": "PUBLIC_SOURCE_READ_FAILED", "pin": "PUBLIC_SOURCE_PIN_FAILED",
                    "compile": "PUBLIC_SOURCE_COMPILE_FAILED", "exec": "PUBLIC_SOURCE_EXEC_FAILED"}


def original_sources():
    raw = (b"ORIGINAL_MARKER = 11\n", b"ORIGINAL_MARKER = 22\n")
    pins = tuple((*fixed[:3], len(value), hashlib.sha256(value).hexdigest())
                 for fixed, value in zip(CONTRACT_SPECS, raw))
    return pins, dict(zip((pin[2] for pin in pins), raw))


def independent_row(ordinal, pin, *, failure=None):
    _, role, path, size, sha = pin
    stages = dict.fromkeys(CONTRACT_STAGES, "PASS")
    actual = {"size_bytes": size, "sha256": sha}
    if failure is not None:
        index = CONTRACT_STAGES.index(failure)
        stages = {name: "PASS" if offset < index else "FAIL" if offset == index else "NOT_RUN"
                  for offset, name in enumerate(CONTRACT_STAGES)}
        if failure == "read":
            actual = None
    return {"ordinal": ordinal, "role": role, "path": path,
            "expected_identity": {"size_bytes": size, "sha256": sha}, "actual_identity": actual,
            "stages": stages, "status": "PASS" if failure is None else "FAIL", "failure_stage": failure,
            "reason": None if failure is None else CONTRACT_REASONS[failure],
            "error_type": None if failure is None else "FileNotFoundError"}


class SourceLoadingTests(unittest.TestCase):
    def setUp(self):
        self.pins, self.files = original_sources()
        self.records, self.registry, self.reads = [], {}, []
        # A mistaken process action must fail instead of running a public or
        # proprietary program. There are no expected process calls in this file.
        self.process_guard = mock.patch.object(subprocess, "Popen", side_effect=AssertionError("process action forbidden"))
        self.process_calls = self.process_guard.start()
        self.addCleanup(self.process_guard.stop)

    def tearDown(self):
        self.process_calls.assert_not_called()

    def read(self, path, maximum):
        self.reads.append((path, maximum))
        self.assertEqual(self.records[-1]["stages"]["read"], "PENDING")
        self.assertEqual(maximum, 65536)
        return self.files[path]

    def load(self, read=None, pins=None):
        return DRIVER.load_public_sources(self.read if read is None else read, self.records,
            pins=self.pins if pins is None else pins, module_registry=self.registry)

    def test_ordered_original_modules_load_and_validate(self):
        self.assertEqual(tuple(pin[:3] for pin in DRIVER.PUBLIC_SOURCE_PINS), CONTRACT_SPECS)
        self.assertEqual(DRIVER.PUBLIC_SOURCE_STAGES, CONTRACT_STAGES)
        self.assertEqual(DRIVER.PUBLIC_SOURCE_REASONS, CONTRACT_REASONS)
        modules = self.load()
        self.assertEqual([modules[name].ORIGINAL_MARKER for name in ("cajviewer_canary", "inventory")], [11, 22])
        self.assertEqual(list(self.registry), ["cajviewer_canary", "inventory"])
        self.assertEqual(self.reads, [(pin[2], 65536) for pin in self.pins])
        observed = DRIVER.validate_source_loads(self.records, pins=self.pins)
        self.assertEqual((observed["records_attempted"], observed["records_completed"], observed["records_failed"],
                          observed["records_remaining"]), (2, 2, 0, 0))
        self.assertTrue(all(value == {"attempted": 2, "completed": 2, "failed": 0} for value in observed["stages"].values()))

    def test_missing_first_source_has_no_identity_or_second_attempt(self):
        error = FileNotFoundError("private path must not enter receipt")
        def read(path, maximum):
            self.reads.append((path, maximum))
            self.assertEqual(self.records[-1]["stages"]["read"], "PENDING")
            raise error
        with self.assertRaises(FileNotFoundError) as caught:
            self.load(read)
        self.assertIs(caught.exception, error)
        self.assertEqual(len(self.reads), 1)
        self.assertEqual(self.records, [independent_row(1, self.pins[0], failure="read")])
        self.assertNotIn("private path", json.dumps(self.records))
        self.assertEqual(DRIVER.validate_source_loads(self.records, pins=self.pins)["records_remaining"], 1)

    def test_missing_second_preserves_first_complete_load(self):
        def read(path, maximum):
            if path == self.pins[1][2]:
                self.reads.append((path, maximum))
                raise FileNotFoundError("private second path")
            return self.read(path, maximum)
        with self.assertRaises(FileNotFoundError):
            self.load(read)
        self.assertEqual(self.records, [independent_row(1, self.pins[0]), independent_row(2, self.pins[1], failure="read")])
        self.assertEqual(list(self.registry), ["cajviewer_canary"])
        observed = DRIVER.validate_source_loads(self.records, pins=self.pins)
        self.assertEqual((observed["records_completed"], observed["records_failed"], observed["records_remaining"]), (1, 1, 0))

    def test_failed_partial_reader_never_labels_prefix_as_full_identity(self):
        def read(path, maximum):
            self.reads.append((path, maximum))
            prefix = self.files[path][:7]
            self.assertTrue(prefix)
            raise OSError("private read detail")
        with self.assertRaises(OSError):
            self.load(read)
        self.assertIsNone(self.records[0]["actual_identity"])
        self.assertEqual(self.records[0]["error_type"], "OSError")
        self.assertEqual(DRIVER.validate_source_loads(self.records, pins=self.pins)["stages"]["read"],
                         {"attempted": 1, "completed": 0, "failed": 1})

    def test_pin_mismatch_stops_before_compile_and_next_read(self):
        self.files[self.pins[0][2]] = b"ORIGINAL_MARKER = 99\n"
        with mock.patch.object(DRIVER, "compile", create=True, side_effect=AssertionError("compile must not run")) as compile_call:
            with self.assertRaises(ValueError):
                self.load()
        compile_call.assert_not_called()
        self.assertEqual(len(self.reads), 1)
        self.assertEqual(self.records[0]["failure_stage"], "pin")
        self.assertEqual(self.records[0]["stages"], {"read": "PASS", "pin": "FAIL", "compile": "NOT_RUN", "exec": "NOT_RUN"})
        self.assertNotEqual(self.records[0]["actual_identity"], self.records[0]["expected_identity"])
        DRIVER.validate_source_loads(self.records, pins=self.pins)

    def test_compile_failure_preserves_read_and_pin(self):
        raw = b"def original broken syntax\n"
        self.files[self.pins[0][2]] = raw
        pins = ((*self.pins[0][:3], len(raw), hashlib.sha256(raw).hexdigest()), self.pins[1])
        with self.assertRaises(SyntaxError):
            self.load(pins=pins)
        self.assertEqual(self.records[0]["stages"], {"read": "PASS", "pin": "PASS", "compile": "FAIL", "exec": "NOT_RUN"})
        self.assertEqual(self.records[0]["error_type"], "SyntaxError")
        self.assertFalse(self.registry)
        DRIVER.validate_source_loads(self.records, pins=pins)

    def test_exec_failure_has_fixed_reason_and_preserves_primary_exception(self):
        raw = b"raise ModuleNotFoundError('private import name')\n"
        self.files[self.pins[0][2]] = raw
        pins = ((*self.pins[0][:3], len(raw), hashlib.sha256(raw).hexdigest()), self.pins[1])
        with self.assertRaises(ModuleNotFoundError):
            self.load(pins=pins)
        self.assertEqual(self.records[0]["stages"], {"read": "PASS", "pin": "PASS", "compile": "PASS", "exec": "FAIL"})
        self.assertEqual((self.records[0]["reason"], self.records[0]["error_type"]), ("PUBLIC_SOURCE_EXEC_FAILED", "ModuleNotFoundError"))
        self.assertNotIn("private import", json.dumps(self.records))
        self.assertEqual(len(self.reads), 1)
        DRIVER.validate_source_loads(self.records, pins=pins)

    def test_cancellation_and_unclassified_errors_are_safe_and_fatal(self):
        class PrivateTypeName(Exception):
            pass
        for error, expected in ((KeyboardInterrupt("private cancellation"), "KeyboardInterrupt"),
                                (SystemExit("private exit"), "SystemExit"),
                                (PrivateTypeName("private error"), "OTHER_ERROR_TYPE")):
            with self.subTest(kind=expected):
                self.records.clear()
                def read(path, maximum):
                    raise error
                with self.assertRaises(type(error)) as caught:
                    self.load(read)
                self.assertIs(caught.exception, error)
                self.assertEqual(self.records[0]["error_type"], expected)
                self.assertIsNone(self.records[0]["actual_identity"])
                self.assertNotIn("PrivateTypeName", json.dumps(self.records))
                DRIVER.validate_source_loads(self.records, pins=self.pins)

    def test_each_operation_is_marked_before_it_runs(self):
        real_compile, real_exec = compile, exec
        seen = []
        def checked_compile(*args):
            seen.append(self.records[-1]["stages"]["compile"])
            return real_compile(*args)
        def checked_exec(*args):
            seen.append(self.records[-1]["stages"]["exec"])
            return real_exec(*args)
        with mock.patch.object(DRIVER, "compile", checked_compile, create=True), \
                mock.patch.object(DRIVER, "exec", checked_exec, create=True):
            self.load()
        self.assertEqual(seen, ["PENDING"] * 4)

    def test_invalid_pin_profile_refuses_before_any_read(self):
        changes = {"wrong-name": ("other", *self.pins[0][1:]),
                   "wrong-role": (self.pins[0][0], "other", *self.pins[0][2:]),
                   "wrong-path": (*self.pins[0][:2], "/private/input", *self.pins[0][3:]),
                   "boolean-size": (*self.pins[0][:3], True, self.pins[0][4]),
                   "float-size": (*self.pins[0][:3], 20.0, self.pins[0][4]),
                   "oversized": (*self.pins[0][:3], 65537, self.pins[0][4]),
                   "malformed-sha": (*self.pins[0][:4], "private")}
        for label, changed in changes.items():
            with self.subTest(label=label), self.assertRaises(DRIVER.SourceLoadError):
                self.load(pins=(changed, self.pins[1]))
        self.assertFalse(self.reads)
        self.assertFalse(self.records)

    def test_invalid_read_results_remain_failed_without_a_full_hash(self):
        for raw in ("not bytes", bytearray(b"original"), b"X" * 65537):
            with self.subTest(kind=type(raw).__name__, size=len(raw)):
                self.records.clear()
                with self.assertRaises(DRIVER.SourceLoadError):
                    self.load(lambda path, maximum: raw)
                self.assertIsNone(self.records[0]["actual_identity"])
                DRIVER.validate_source_loads(self.records, pins=self.pins)

    def test_independent_validator_copies_only_safe_known_records(self):
        rows = [independent_row(index, pin) for index, pin in enumerate(self.pins, 1)]
        observed = DRIVER.validate_source_loads(rows, pins=self.pins)
        self.assertEqual(observed["records"], rows)
        rows[0]["expected_identity"]["size_bytes"] = 999
        rows[0]["stages"]["exec"] = "FAIL"
        self.assertEqual(observed["records"][0], independent_row(1, self.pins[0]))
        with self.assertRaises(DRIVER.SourceLoadError):
            DRIVER.validate_source_loads(observed["records"])

    def test_independent_validator_rejects_scalar_and_identity_mutations(self):
        basis = [independent_row(index, pin) for index, pin in enumerate(self.pins, 1)]
        mutations = {
            "boolean-ordinal": lambda row: row.update(ordinal=True),
            "float-ordinal": lambda row: row.update(ordinal=1.0),
            "foreign-path": lambda row: row.update(path="/private/source"),
            "foreign-role": lambda row: row.update(role="viewer"),
            "boolean-expected-size": lambda row: row["expected_identity"].update(size_bytes=True),
            "changed-expected-sha": lambda row: row["expected_identity"].update(sha256="a" * 64),
            "float-actual-size": lambda row: row["actual_identity"].update(size_bytes=20.0),
            "oversized-actual": lambda row: row["actual_identity"].update(size_bytes=65537),
            "malformed-actual-sha": lambda row: row["actual_identity"].update(sha256="a" * 63),
            "raw-exception-field": lambda row: row.update(exception="private text"),
            "missing-stage": lambda row: row["stages"].pop("read"),
            "unknown-stage-state": lambda row: row["stages"].update(read="SUCCESS"),
            "missing-actual": lambda row: row.update(actual_identity=None),
            "wrong-pin-actual": lambda row: row["actual_identity"].update(sha256="b" * 64),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                rows = copy.deepcopy(basis)
                mutate(rows[0])
                with self.assertRaises(DRIVER.SourceLoadError):
                    DRIVER.validate_source_loads(rows, pins=self.pins)

    def test_validator_rejects_order_and_impossible_stage_chains(self):
        good = [independent_row(index, pin) for index, pin in enumerate(self.pins, 1)]
        failure = independent_row(1, self.pins[0], failure="read")
        variants = {"misordered": list(reversed(good)), "duplicate": [good[0], good[0]],
                    "second-after-failure": [failure, good[1]], "third": good + [good[1]],
                    "read-fail-with-identity": [{**failure, "actual_identity": good[0]["actual_identity"]}],
                    "read-pending-with-identity": [{**good[0], "stages": {"read": "PENDING", "pin": "NOT_RUN", "compile": "NOT_RUN", "exec": "NOT_RUN"},
                                                    "status": "PENDING"}],
                    "compile-before-pin": [{**good[0], "stages": {"read": "PASS", "pin": "NOT_RUN", "compile": "PASS", "exec": "NOT_RUN"}}],
                    "unobserved-read": [{**good[0], "actual_identity": None, "stages": dict.fromkeys(DRIVER.PUBLIC_SOURCE_STAGES, "NOT_RUN")}],
                    "unclassified-message": [{**failure, "error_type": "PrivateTypeName"}]}
        for label, rows in variants.items():
            with self.subTest(label=label), self.assertRaises(DRIVER.SourceLoadError):
                DRIVER.validate_source_loads(rows, pins=self.pins)

    def test_complete_failure_is_known_before_outer_nonzero_refusal(self):
        row = independent_row(1, self.pins[0], failure="read")
        envelope = {"status": "FAIL", "source_loads": [row], "closing_audits": {"caps": "PASS", "environment": "PASS", "user": "PASS"}}
        report = {"source": DRIVER.source_loading_observation(envelope, complete=True, pins=self.pins),
                  "closing_audits": copy.deepcopy(envelope["closing_audits"])}
        with self.assertRaises(DRIVER.CanaryError):
            if envelope["status"] != "PASS":
                raise DRIVER.CanaryError("original outer child refusal")
        self.assertEqual((report["source"]["records_attempted"], report["source"]["records_failed"],
                          report["source"]["records_remaining"]), (1, 1, 1))
        self.assertEqual(report["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})

    def test_prefix_malformed_or_impossible_pass_remains_unknown(self):
        failure = {"status": "FAIL", "source_loads": [independent_row(1, self.pins[0], failure="read")]}
        for label, envelope, complete in (("prefix", failure, False), ("nonboolean-complete", failure, 1),
                    ("missing", {"status": "FAIL"}, True), ("malformed", {"status": "FAIL", "source_loads": {}}, True),
                    ("partial-pass", {"status": "PASS", "source_loads": failure["source_loads"]}, True),
                    ("empty-pass", {"status": "PASS", "source_loads": []}, True), ("absent-envelope", None, True)):
            with self.subTest(label=label):
                observed = DRIVER.source_loading_observation(envelope, complete=complete, pins=self.pins)
                self.assertEqual(observed["observation"], "UNKNOWN")
                self.assertIsNone(observed["records"])
                self.assertNotIn("records_completed", observed)

    def test_valid_pending_read_keeps_attempt_and_unobserved_outcomes(self):
        row = independent_row(1, self.pins[0], failure="read")
        row.update(status="PENDING", failure_stage=None, reason=None, error_type=None)
        row["stages"]["read"] = "PENDING"
        observed = DRIVER.source_loading_observation({"status": "FAIL", "source_loads": [row]}, complete=True, pins=self.pins)
        self.assertEqual(observed["observation"], "COMPLETE_ORDERED_RECORDS")
        self.assertEqual((observed["records_attempted"], observed["records_completed"], observed["records_failed"],
                          observed["records_remaining"]), (1, 0, 0, 1))
        self.assertEqual(observed["stages"]["read"], {"attempted": 1, "completed": 0, "failed": 0})
        self.assertTrue(all(observed["stages"][stage] == {"attempted": 0, "completed": 0, "failed": 0}
                            for stage in ("pin", "compile", "exec")))

    def test_live_byte_fragments_match_loaded_functions(self):
        with RUNNER_PATH.open("rb", buffering=0) as stream:
            raw = stream.read(65537)
        self.assertLessEqual(len(raw), 65536)
        loader = DRIVER.public_source_fragment(raw, "LOADER")
        validator = DRIVER.public_source_fragment(raw, "VALIDATOR")
        namespace = {"hashlib": hashlib, "types": types, "sys": types.SimpleNamespace(modules={})}
        exec(compile(loader, "original-live-loader-fragment", "exec"), namespace)
        records = []
        modules = namespace["load_public_sources"](lambda path, maximum: self.files[path], records,
                                                   pins=self.pins, module_registry={})
        self.assertEqual(modules["inventory"].ORIGINAL_MARKER, 22)
        independent = {}
        exec(compile(validator, "original-live-validator-fragment", "exec"), independent)
        self.assertEqual(independent["validate_source_loads"](records, pins=self.pins),
                         DRIVER.validate_source_loads(records, pins=self.pins))

    def test_fragment_extraction_refuses_missing_duplicate_and_unbounded_blocks(self):
        with RUNNER_PATH.open("rb", buffering=0) as stream:
            source = stream.read(65537)
        for label, raw, part in (("missing", b"original\n", "LOADER"),
                  ("duplicate", source + b"# BEGIN PUBLIC SOURCE COMMON\n", "LOADER"),
                  ("oversized", b"X" * 65537, "LOADER"), ("wrong-type", source.decode(), "LOADER"),
                  ("unknown-block", source, "PRIVATE")):
            with self.subTest(label=label), self.assertRaises(DRIVER.SourceLoadError):
                DRIVER.public_source_fragment(raw, part)


if __name__ == "__main__":
    unittest.main()
