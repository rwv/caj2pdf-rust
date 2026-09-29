# SPDX-License-Identifier: MIT
"""Fixed cache-profile controls with invented ENV and helper observations."""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
HELPER_PATH = ROOT / "tests/conformance/test_cajviewer_inventory_diagnostics.py"
FORBIDDEN_ATTEMPTS = 0


def forbidden_effect(*args, **kwargs):
    global FORBIDDEN_ATTEMPTS
    FORBIDDEN_ATTEMPTS += 1
    raise AssertionError("original profile must not perform an actual effect")


class RefusingEnvironment:
    __iter__ = __getitem__ = get = items = keys = values = copy = forbidden_effect


spec = importlib.util.spec_from_file_location("original_cache_entry_controls", HELPER_PATH)
ENTRY_CONTROLS = importlib.util.module_from_spec(spec)
with mock.patch.object(subprocess, "Popen", side_effect=forbidden_effect) as process, \
        mock.patch.object(socket, "socket", side_effect=forbidden_effect) as connection:
    spec.loader.exec_module(ENTRY_CONTROLS)
    process.assert_not_called()
    connection.assert_not_called()
DRIVER = ENTRY_CONTROLS.DRIVER


def declared_environment():
    return {"HOME": "/home/canary", "XDG_CONFIG_HOME": "/home/canary/.config",
            "XDG_CACHE_HOME": "/home/canary/.cache", "HOSTNAME": "original-inventory-cache",
            "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "TZ": "UTC"}


class InventoryCacheTests(unittest.TestCase):
    def setUp(self):
        self.process_patch = mock.patch.object(subprocess, "Popen", side_effect=forbidden_effect)
        self.socket_patch = mock.patch.object(socket, "socket", side_effect=forbidden_effect)
        self.process = self.process_patch.start()
        self.connection = self.socket_patch.start()
        self.addCleanup(self.process_patch.stop)
        self.addCleanup(self.socket_patch.stop)
        self.entry = ENTRY_CONTROLS.InventoryDiagnosticsTests("runTest")

    def tearDown(self):
        self.process.assert_not_called()
        self.connection.assert_not_called()

    def test_profile_copies_only_cache_and_returns_one_exact_override(self):
        original = declared_environment()
        before = copy.deepcopy(original)
        environment, arguments = DRIVER.inventory_cache_profile(original)
        self.assertEqual(original, before)
        self.assertIsNot(environment, original)
        self.assertEqual(set(environment), set(original))
        self.assertEqual([key for key in original if environment[key] != original[key]], ["XDG_CACHE_HOME"])
        self.assertEqual(environment["XDG_CACHE_HOME"], "/tmp")
        self.assertEqual(arguments, ["--env", "XDG_CACHE_HOME=/tmp"])
        second_environment, second_arguments = DRIVER.inventory_cache_profile(original)
        environment["HOME"] = "/original/mutation"
        arguments.append("--env")
        self.assertEqual(second_environment["HOME"], "/home/canary")
        self.assertEqual(second_arguments, ["--env", "XDG_CACHE_HOME=/tmp"])

    def test_exact_old_home_config_and_cache_are_required(self):
        for key in ("HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
            for value in (None, "/original/foreign", "/tmp", 1, False):
                with self.subTest(key=key, value=value):
                    environment = declared_environment()
                    if value is None:
                        del environment[key]
                    else:
                        environment[key] = value
                    with self.assertRaisesRegex(DRIVER.InventoryDiagnosticError, "^inventory-cache-profile$"):
                        DRIVER.inventory_cache_profile(environment)

    def test_environment_types_and_existing_entry_bounds_are_enforced(self):
        class Subclass(dict):
            pass
        values = [None, [], Subclass(declared_environment()),
                  {**declared_environment(), 1: "original"},
                  {**declared_environment(), "ORIGINAL": []},
                  {**declared_environment(), "": "original"},
                  {**declared_environment(), "ORIGINAL=KEY": "original"},
                  {**declared_environment(), "x" * 129: "original"},
                  {**declared_environment(), "ORIGINAL": "x" * 4097},
                  {**declared_environment(), "ORIGINAL": "x\0y"},
                  {**declared_environment(), "x\0y": "original"}]
        values.append({**declared_environment(), **{f"ORIGINAL_{index}": "x" for index in range(58)}})
        for environment in values:
            with self.subTest(kind=type(environment).__name__), self.assertRaises(DRIVER.InventoryDiagnosticError):
                DRIVER.inventory_cache_profile(environment)
        boundary = {**declared_environment(), **{f"ORIGINAL_{index}": "x" for index in range(57)}}
        boundary["x" * 128] = boundary.pop("ORIGINAL_0")
        boundary["ORIGINAL_1"] = "x" * 4096
        self.assertEqual(len(DRIVER.inventory_cache_profile(boundary)[0]), 64)

    def test_profile_does_not_read_ambient_environment_clock_or_files(self):
        original = declared_environment()
        with mock.patch.object(os, "environ", new=RefusingEnvironment()), \
                mock.patch.object(DRIVER.time, "monotonic", side_effect=forbidden_effect) as clock, \
                mock.patch.object(Path, "open", side_effect=forbidden_effect) as opened:
            environment, arguments = DRIVER.inventory_cache_profile(original)
            clock.assert_not_called()
            opened.assert_not_called()
        self.assertEqual(environment["XDG_CACHE_HOME"], "/tmp")
        self.assertEqual(arguments, ["--env", "XDG_CACHE_HOME=/tmp"])

    def test_actual_entry_accepts_matching_profile_and_keeps_all_closing_checks(self):
        environment, _ = DRIVER.inventory_cache_profile(declared_environment())
        envelope, code, events, pins, _ = self.entry.entry(
            [ENTRY_CONTROLS.result(), ENTRY_CONTROLS.result(stdout=b"original fonts\n")],
            expected_environment=environment, actual_environment=environment)
        self.assertEqual((code, envelope["status"]), (0, "PASS"))
        self.assertEqual(events.count("helper"), 2)
        self.assertEqual(envelope["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})
        DRIVER.require_inventory_success(envelope, envelope["environment_identity"], pins=pins)

    def test_initial_actual_cache_mismatch_refuses_before_module_or_helper(self):
        environment, _ = DRIVER.inventory_cache_profile(declared_environment())
        for cache in ("/home/canary/.cache", "/original/foreign"):
            with self.subTest(cache=cache):
                actual = {**environment, "XDG_CACHE_HOME": cache}
                envelope, code, events, _, _ = self.entry.entry([], expected_environment=environment,
                                                               actual_environment=actual)
                self.assertEqual((code, envelope["status"]), (1, "FAIL"))
                self.assertEqual(events.count("source"), 0)
                self.assertEqual(events.count("helper"), 0)
                self.assertEqual(envelope["source_loads"], [])
                self.assertEqual(envelope["actions"], [])
                self.assertEqual((envelope["app_launches"], envelope["vendor_passes"]), (0, 0))

    def test_closing_cache_mutation_refuses_even_after_two_successful_helpers(self):
        environment, _ = DRIVER.inventory_cache_profile(declared_environment())
        envelope, code, events, pins, _ = self.entry.entry(
            [ENTRY_CONTROLS.result(), ENTRY_CONTROLS.result()], expected_environment=environment,
            actual_environment=environment, closing_environment={**environment, "XDG_CACHE_HOME": "/original/changed"})
        self.assertEqual((code, envelope["status"]), (1, "FAIL"))
        self.assertEqual(events.count("helper"), 2)
        self.assertEqual(envelope["closing_audits"], {"caps": "PASS", "environment": "FAIL", "user": "PASS"})
        self.assertNotEqual(envelope["environment_identity"], envelope["environment_after_identity"])
        with self.assertRaises(DRIVER.InventoryDiagnosticError):
            DRIVER.require_inventory_success(envelope, envelope["environment_identity"], pins=pins)

    def test_cache_profile_does_not_relax_stderr_or_primary_and_closing_failure(self):
        environment, _ = DRIVER.inventory_cache_profile(declared_environment())
        stderr = b"synthetic metadata warning\n"
        envelope, code, _, pins, raw = self.entry.entry(
            [ENTRY_CONTROLS.result(), ENTRY_CONTROLS.result(stderr=stderr)],
            expected_environment=environment, actual_environment=environment,
            closing_environment={**environment, "XDG_CACHE_HOME": "/original/changed"})
        self.assertEqual((code, envelope["status"]), (1, "FAIL"))
        self.assertEqual(envelope["terminal_helper"]["reason"], "HELPER_STDERR_NOT_EMPTY")
        self.assertEqual(envelope["terminal_helper"]["stderr"]["sha256"], hashlib.sha256(stderr).hexdigest())
        self.assertEqual(envelope["closing_audits"]["environment"], "FAIL")
        report = {}
        with self.assertRaisesRegex(DRIVER.InventoryDiagnosticError, "^required-inventory-helper$"):
            DRIVER.observe_inventory_result(ENTRY_CONTROLS.result(stdout=raw, status="FAIL", exit_code=1),
                                            report, envelope["environment_identity"], pins=pins)
        self.assertEqual(report["nested_helpers"]["terminal_helper"], envelope["terminal_helper"])
        self.assertEqual(report["container_environment"]["closing_audits"]["environment"], "FAIL")

    def test_canonical_entry_and_public_source_pins_remain_exact(self):
        source = self.entry.raw_source()
        assembled = DRIVER.inventory_entry_source(source)
        self.assertEqual(len(assembled), 16328)
        self.assertEqual(hashlib.sha256(assembled).hexdigest(),
                         "8fd9503d1e4d7d8692fd3589f2dc0e6044e212c03adf438f82c5ed489e955f88")
        self.assertEqual([(row[2], row[3], row[4]) for row in DRIVER.PUBLIC_SOURCE_PINS], [
            ("/opt/canary/cajviewer_canary.py", 10992, "ba9abc3be6285cfb7d0af3840b88197fd5aa4ec22d7277ecee109ea0eae60c77"),
            ("/opt/canary/inventory.py", 2656, "cb1cec2a74be10413ce628cf5320180c1d2d950212f59b5750bbad434a24d556")])


if __name__ == "__main__":
    program = unittest.main(verbosity=2, exit=False)
    attempts = FORBIDDEN_ATTEMPTS + ENTRY_CONTROLS.FORBIDDEN_ATTEMPTS
    print(json.dumps({"synthetic_helper_callbacks": ENTRY_CONTROLS.SYNTHETIC_CALLBACKS,
                      "forbidden_effect_attempts": attempts, "actual_children": 0,
                      "vendor_passes": 0, "compatibility": "NOT_RUN"}, sort_keys=True))
    raise SystemExit(0 if program.result.wasSuccessful() and not program.result.skipped and attempts == 0 else 1)
