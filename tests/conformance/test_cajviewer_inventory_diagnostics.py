# SPDX-License-Identifier: MIT
"""Invented inventory transcripts only; no child, socket or vendor operation."""

import ast
import base64
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import socket
import subprocess
import sys
import tokenize
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
RUNNER_PATH = ROOT / "tools/cajviewer/run.py"
SYNTHETIC_CALLBACKS = FORBIDDEN_ATTEMPTS = 0


def forbidden_effect(*args, **kwargs):
    global FORBIDDEN_ATTEMPTS
    FORBIDDEN_ATTEMPTS += 1
    raise AssertionError("actual process/socket operation forbidden")


spec = importlib.util.spec_from_file_location("original_inventory_diagnostics", RUNNER_PATH)
DRIVER = importlib.util.module_from_spec(spec)
with mock.patch.object(subprocess, "Popen", side_effect=forbidden_effect) as import_process, \
        mock.patch.object(socket, "socket", side_effect=forbidden_effect) as import_socket:
    spec.loader.exec_module(DRIVER)
    import_process.assert_not_called()
    import_socket.assert_not_called()
COMMANDS = (["dpkg-query", "-W", "-f=${Package}\t${Version}\t${Architecture}\n"],
            ["fc-list", "--format", "%{file}\t%{family}\t%{style}\n"])
SOURCE_SPECS = (("cajviewer_canary", "process-helper", "/opt/canary/cajviewer_canary.py"),
                ("inventory", "runtime-inventory", "/opt/canary/inventory.py"))


def identity(raw):
    return {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def result(stdout=b"original\n", stderr=b"", *, status="PASS", exit_code=0, counts=None):
    return {"status": status, "exit_code": exit_code, "stdout": stdout, "stderr": stderr,
            "bytes_read": {"stdout": len(stdout), "stderr": len(stderr)} if counts is None else counts,
            "prefix_truncated": status == "OUTPUT_LIMIT", "elapsed_seconds": 0.01}


def invented_caps():
    return {"memory_max_bytes": 536870912, "memory_swap_max_bytes": 0, "memory_peak_bytes": 1024,
            "pids_max": 64, "pids_peak": 1, "cpu_quota": 200000, "cpu_period": 100000,
            "memory_events": {"oom": 0, "oom_kill": 0}}


def independent_action(ordinal, stdout=b"original\n", stderr=b"", *, failed=False):
    return {"ordinal": ordinal, "argv": list(COMMANDS[ordinal - 1]), "status": "FAIL" if failed else "PASS",
            "spawned": True, "helper_status": "PASS", "exit_code": 0,
            "bytes_read": {"stdout": len(stdout), "stderr": len(stderr)},
            "captures": {name: {**identity(raw), "complete": True, "hash_scope": "retained-stream"}
                         for name, raw in (("stdout", stdout), ("stderr", stderr))},
            "elapsed_seconds": 0.01, "error_type": "ValueError" if failed else None}


def independent_diagnostic(action, stderr):
    return {"schema": "cajviewer-inventory-helper-diagnostic/1", "action_ordinal": action["ordinal"],
            "stage": "metadata-helper-validation", "reason": "HELPER_STDERR_NOT_EMPTY", "error_type": "ValueError",
            "helper_status": "PASS", "exit_code": 0, "spawned": True,
            "captures": copy.deepcopy(action["captures"]), "bytes_read": dict(action["bytes_read"]),
            "stderr": {"encoding": "base64", "data": base64.b64encode(stderr).decode("ascii"),
                       "retained_bytes": len(stderr), "sha256": hashlib.sha256(stderr).hexdigest(),
                       "complete": True, "truncated": False, "hash_scope": "retained-stream"}}


def independent_source_rows(pins):
    return [{"ordinal": ordinal, "role": pin[1], "path": pin[2],
             "expected_identity": {"size_bytes": pin[3], "sha256": pin[4]},
             "actual_identity": {"size_bytes": pin[3], "sha256": pin[4]},
             "stages": {"read": "PASS", "pin": "PASS", "compile": "PASS", "exec": "PASS"},
             "status": "PASS", "failure_stage": None, "reason": None, "error_type": None}
            for ordinal, pin in enumerate(pins, 1)]


def original_envelope(actions, diagnostic=None):
    environment = identity(b'{"LANG":"C.UTF-8"}\n')
    text = '{"original":true}'
    return {"schema": "cajviewer-inventory-accounting/1", "status": "FAIL" if diagnostic else "PASS",
            "scope": "opaque-runtime-inventory-only", "app_launches": 0, "vendor_passes": 0,
            "actions": actions, "terminal_helper": diagnostic,
            "source_loads": independent_source_rows(DRIVER.PUBLIC_SOURCE_PINS),
            "caps_before": invented_caps(), "caps_after": invented_caps(),
            "environment_identity": environment, "environment_after_identity": dict(environment),
            "uid_gid": [1000, 1000], "uid_gid_after": [1000, 1000],
            "closing_audits": {"caps": "PASS", "environment": "PASS", "user": "PASS"}, "closing_failures": [],
            "inventory_json": None if diagnostic else text, "inventory_identity": None if diagnostic else identity(text.encode())}


class InventoryDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.report = {"actions": [], "terminal_helper": None}
        self.callbacks = []
        self.process_patch = mock.patch.object(subprocess, "Popen", side_effect=forbidden_effect)
        self.socket_patch = mock.patch.object(socket, "socket", side_effect=forbidden_effect)
        self.process_calls, self.socket_calls = self.process_patch.start(), self.socket_patch.start()
        self.addCleanup(self.process_patch.stop)
        self.addCleanup(self.socket_patch.stop)

    def tearDown(self):
        self.process_calls.assert_not_called()
        self.socket_calls.assert_not_called()

    def call(self, value, ordinal=1, report=None):
        report = self.report if report is None else report
        def callback(argv, **kwargs):
            global SYNTHETIC_CALLBACKS
            SYNTHETIC_CALLBACKS += 1
            self.callbacks.append((list(argv), kwargs))
            self.assertEqual(report["actions"][-1]["status"], "PENDING")
            if isinstance(value, BaseException):
                raise value
            return value
        return DRIVER.inventory_helper_call(callback, report, COMMANDS[ordinal - 1],
                                             deadline_seconds=10, output_limit=262144)

    def raw_source(self):
        with RUNNER_PATH.open("rb", buffering=0) as stream:
            source = stream.read(65537)
        self.assertLessEqual(len(source), 65536)
        return source

    def entry(self, values, *, closing_fault=None, source_failure=False, output_value=None, write_fault=None):
        """Execute the actual full entry AST with invented boundary providers.

        All original statement nodes, including its real finally/serialization,
        run unchanged. Only source-reader/module data, caps/ENV/UID/tool boundary
        providers are injected. No external inline draft or runtime is opened.
        """
        raw = DRIVER.inventory_entry_source(self.raw_source())
        tree = ast.parse(raw, "original-complete-inventory-entry")
        start = next(index for index, node in enumerate(tree.body) if isinstance(node, ast.Try))
        self.assertEqual(sum(isinstance(node, ast.Try) for node in tree.body), 1)
        prefix, body = ast.Module(tree.body[:start], []), ast.Module(tree.body[start:], [])
        namespace = {}
        exec(compile(prefix, "original-entry-definitions", "exec"), namespace)
        first = b"ORIGINAL_MARKER = 1\n"
        second = b'''def run_bounded(argv, **kwargs):
    return original_callback(argv, **kwargs)
def hash_regular(path, **kwargs):
    return {"size_bytes": 1, "sha256": tool_sha}
def inventory():
    for name in ("dpkg-query", "fc-list"):
        hash_regular("/original/" + name)
    for argv in original_commands:
        run_bounded(argv, deadline_seconds=10, output_limit=262144)
    return original_output
'''
        pins = tuple((*fixed, len(value), hashlib.sha256(value).hexdigest())
                     for fixed, value in zip(SOURCE_SPECS, (first, second)))
        registry, events = {}, []
        class OriginalOutput(io.BytesIO):
            def write(self, raw):
                events.append("write")
                if write_fault == "write": raise OSError("invented write failure")
                return super().write(raw)
            def flush(self):
                events.append("flush")
                if write_fault == "flush": raise KeyboardInterrupt("invented flush interruption")
                return super().flush()
        output = OriginalOutput()
        environment = {"LANG": "C.UTF-8"}
        tool_sha = hashlib.sha256(b"x").hexdigest()
        tools = {name: {"path": "/original/" + name, "size_bytes": 1, "sha256": tool_sha}
                 for name in ("dpkg-query", "fc-list")}
        class OriginalPath:
            def __init__(self, value): self.value = value
            def resolve(self, **kwargs): return self
            def __str__(self): return self.value
        def callback(argv, **kwargs):
            global SYNTHETIC_CALLBACKS
            SYNTHETIC_CALLBACKS += 1
            events.append("helper")
            value = values[len([item for item in events if item == "helper"]) - 1]
            if isinstance(value, BaseException): raise value
            return value
        def load(read, records):
            events.append("source")
            def original_read(path, maximum):
                self.assertEqual(maximum, 65536)
                if source_failure: raise FileNotFoundError("invented source failure")
                return {pins[0][2]: first, pins[1][2]: second}[path]
            modules = DRIVER.load_public_sources(original_read, records, pins=pins, module_registry=registry)
            modules["inventory"].__dict__.update(original_callback=callback, tool_sha=tool_sha,
                original_commands=copy.deepcopy(COMMANDS), original_output={"original": True} if output_value is None else output_value)
            return modules
        counts = {"caps": 0, "environment": 0, "uid": 0}
        def caps():
            counts["caps"] += 1
            events.append("caps")
            if counts["caps"] == 2 and closing_fault in ("caps", "all"):
                raise OSError("invented closing cap failure")
            return invented_caps()
        def snapshot():
            counts["environment"] += 1
            events.append("environment")
            if counts["environment"] == 2 and closing_fault in ("environment", "all"):
                raise RuntimeError("invented closing environment failure")
            return dict(environment)
        def uid():
            counts["uid"] += 1
            events.append("uid")
            if counts["uid"] == 2 and closing_fault in ("user", "all"):
                raise KeyboardInterrupt("invented closing user interruption")
            return 1000
        def stop(code): raise SystemExit(code)
        namespace.update(load_public_sources=load, caps=caps, environment_snapshot=snapshot, Path=OriginalPath,
            shutil=types.SimpleNamespace(which=lambda name: "/original/" + name),
            os=types.SimpleNamespace(getuid=uid, getgid=lambda: 1000),
            sys=types.SimpleNamespace(argv=["original-entry", json.dumps(environment), json.dumps(tools)],
                modules=registry, stdout=types.SimpleNamespace(buffer=output), exit=stop))
        if write_fault is not None:
            error = OSError if write_fault == "write" else KeyboardInterrupt
            with self.assertRaises(error):
                exec(compile(body, "original-complete-entry-and-finally", "exec"), namespace)
            self.assertEqual(events[-1], write_fault)
            return namespace["report"], None, events, pins, output.getvalue()
        with self.assertRaises(SystemExit) as caught:
            exec(compile(body, "original-complete-entry-and-finally", "exec"), namespace)
        envelope = DRIVER.parse_inventory_envelope(output.getvalue())
        return envelope, caught.exception.code, events, pins, output.getvalue()

    def test_two_ordered_helpers_pass_without_terminal_diagnostic(self):
        self.assertEqual(tuple(tuple(value) for value in COMMANDS), DRIVER.INVENTORY_COMMANDS)
        self.assertEqual(DRIVER.INVENTORY_LIMITS, {"stdout": 262144, "stderr": 65536})
        self.call(result())
        self.call(result(stdout=b"fonts\n"), 2)
        self.assertIsNone(self.report["terminal_helper"])
        self.assertEqual([row["status"] for row in DRIVER.validate_inventory_actions(self.report["actions"])], ["PASS", "PASS"])
        self.assertIsNone(DRIVER.validate_inventory_diagnostic(None, self.report["actions"]))

    def test_v12_shaped_exit_zero_nonempty_stderr_is_failed_verbatim(self):
        self.call(result(stdout=b"p" * 8060))
        with self.assertRaises(ValueError): self.call(result(stdout=b"f" * 8208, stderr=b"o" * 48), 2)
        diagnostic = self.report["terminal_helper"]
        self.assertEqual((diagnostic["action_ordinal"], diagnostic["stage"], diagnostic["reason"], diagnostic["helper_status"], diagnostic["exit_code"]),
                         (2, "metadata-helper-validation", "HELPER_STDERR_NOT_EMPTY", "PASS", 0))
        self.assertEqual(base64.b64decode(diagnostic["stderr"]["data"]), b"o" * 48)
        self.assertEqual(diagnostic, DRIVER.validate_inventory_diagnostic(diagnostic, self.report["actions"]))
        original = copy.deepcopy(diagnostic)
        with self.assertRaises(ValueError): self.call(result(), 2)
        self.assertEqual(self.report["terminal_helper"], original)
        self.assertEqual(len(self.callbacks), 2)

    def test_unavailable_and_raised_result_leave_spawn_unknown(self):
        for value in (None, FileNotFoundError("invented path"), KeyboardInterrupt("invented interruption")):
            with self.subTest(kind=type(value).__name__):
                report = {"actions": [], "terminal_helper": None}
                with self.assertRaises(BaseException): self.call(value, report=report)
                diagnostic = DRIVER.validate_inventory_diagnostic(report["terminal_helper"], report["actions"])
                self.assertEqual(diagnostic["reason"], "HELPER_RESULT_UNAVAILABLE")
                self.assertIsNone(diagnostic["spawned"])
                self.assertIsNone(diagnostic["stderr"])
                observed = DRIVER.inventory_helper_observation({"status": "FAIL", **report}, complete=True)
                self.assertEqual(observed["attempted"], 1)
                self.assertIsNone(observed["spawned"])
                self.assertEqual((observed["observed_spawned"], observed["spawn_unknown"]), (0, 1))
                self.assertNotIn("invented path", json.dumps(diagnostic))

    def test_malformed_helper_result_cannot_gain_captures_or_spawn(self):
        variants = ["not a result", {}, {**result(), "exit_code": True}, {**result(), "elapsed_seconds": float("nan")},
                    {**result(), "stdout": bytearray(b"x")}, {**result(), "extra": "invented"},
                    {**result(), "bytes_read": {"stdout": 262145, "stderr": 0}},
                    {**result(), "bytes_read": {"stdout": 9, "stderr": 65537}}]
        for value in variants:
            with self.subTest(value_type=type(value).__name__):
                report = {"actions": [], "terminal_helper": None}
                with self.assertRaises(ValueError): self.call(value, report=report)
                self.assertEqual(report["terminal_helper"]["reason"], "HELPER_RESULT_MALFORMED")
                self.assertIsNone(report["actions"][0]["spawned"])
                DRIVER.validate_inventory_diagnostic(report["terminal_helper"], report["actions"])

    def test_nonzero_exit_is_terminal_even_with_empty_stderr(self):
        for status, code, reason in (("FAIL", 7, "HELPER_NONZERO_EXIT"), ("PASS", 7, "HELPER_NONZERO_EXIT"),
                                     ("FAIL", 0, "HELPER_RESULT_MALFORMED")):
            with self.subTest(status=status, code=code):
                report = {"actions": [], "terminal_helper": None}
                with self.assertRaises(ValueError): self.call(result(status=status, exit_code=code), report=report)
                diagnostic = DRIVER.validate_inventory_diagnostic(report["terminal_helper"], report["actions"])
                self.assertEqual((diagnostic["reason"], diagnostic["exit_code"], diagnostic["spawned"]), (reason, code, True))
                self.assertEqual(diagnostic["stderr"]["data"], "")
                self.assertTrue(diagnostic["stderr"]["complete"])

    def test_timeout_false_truncation_flag_still_is_incomplete(self):
        value = result(stderr=b"original timeout", status="TIMEOUT", exit_code=-9)
        self.assertFalse(value["prefix_truncated"])
        with self.assertRaises(ValueError): self.call(value)
        diagnostic = DRIVER.validate_inventory_diagnostic(self.report["terminal_helper"], self.report["actions"])
        self.assertEqual(diagnostic["reason"], "HELPER_TIMEOUT")
        self.assertFalse(diagnostic["stderr"]["complete"])
        self.assertTrue(diagnostic["stderr"]["truncated"])
        self.assertTrue(all(not value["complete"] for value in diagnostic["captures"].values()))

    def test_output_limit_distinguishes_both_sentinels_capture_and_excerpt(self):
        for name, count, raw_size, retained in (("stderr", 65537, 65537, 65536), ("stdout", 262145, 262144, 262144)):
            with self.subTest(name=name):
                report = {"actions": [], "terminal_helper": None}
                payloads = {"stdout": b"", "stderr": b""}
                payloads[name] = b"x" * raw_size
                value = result(**payloads, status="OUTPUT_LIMIT", exit_code=-9, counts={"stdout": 0, "stderr": 0, name: count})
                with self.assertRaises(ValueError): self.call(value, report=report)
                diagnostic = DRIVER.validate_inventory_diagnostic(report["terminal_helper"], report["actions"])
                self.assertEqual(diagnostic["reason"], "HELPER_OUTPUT_LIMIT")
                self.assertEqual(diagnostic["bytes_read"][name], count)
                self.assertEqual(diagnostic["captures"][name]["size_bytes"], retained)
                self.assertFalse(diagnostic["captures"][name]["complete"])
                self.assertEqual(diagnostic["stderr"]["retained_bytes"], 4096 if name == "stderr" else 0)

    def test_incomplete_capture_precedes_nonempty_stderr(self):
        value = result(stdout=b"prefix", stderr=b"original stderr", counts={"stdout": 20, "stderr": 15})
        with self.assertRaises(ValueError): self.call(value)
        self.assertEqual(self.report["terminal_helper"]["reason"], "HELPER_CAPTURE_INCOMPLETE")
        DRIVER.validate_inventory_diagnostic(self.report["terminal_helper"], self.report["actions"])

    def test_diagnostic_4096_boundary_and_larger_prefix(self):
        for size in (4095, 4096, 4097, 65536):
            with self.subTest(size=size):
                report = {"actions": [], "terminal_helper": None}
                raw = bytes(index % 251 for index in range(size))
                with self.assertRaises(ValueError): self.call(result(stderr=raw), report=report)
                diagnostic = DRIVER.validate_inventory_diagnostic(report["terminal_helper"], report["actions"])
                kept = diagnostic["stderr"]
                self.assertEqual(base64.b64decode(kept["data"]), raw[:4096])
                self.assertEqual(kept["retained_bytes"], min(size, 4096))
                self.assertEqual(kept["complete"], size <= 4096)
                self.assertEqual(kept["truncated"], size > 4096)
                self.assertLessEqual(len(kept["data"]), 5464)

    def test_unknown_error_type_and_first_diagnostic_are_preserved(self):
        class InventedPrivateType(Exception): pass
        with self.assertRaises(InventedPrivateType): self.call(InventedPrivateType("not retained"))
        diagnostic = copy.deepcopy(self.report["terminal_helper"])
        self.assertEqual(diagnostic["error_type"], "OTHER_ERROR_TYPE")
        self.assertNotIn("InventedPrivateType", json.dumps(diagnostic))
        with self.assertRaises(ValueError): self.call(result())
        self.assertEqual(self.report["terminal_helper"], diagnostic)
        self.assertEqual(len(self.callbacks), 1)

    def test_unplanned_helper_does_not_register_or_invoke_an_action(self):
        callback = mock.Mock(side_effect=AssertionError("unplanned callback"))
        for argv, kwargs in ((["other"], {"deadline_seconds": 10, "output_limit": 262144}),
                             (COMMANDS[0], {"deadline_seconds": 11, "output_limit": 262144}),
                             (COMMANDS[0], {"deadline_seconds": 10.0, "output_limit": 262144}),
                             (COMMANDS[0], {"deadline_seconds": 10, "output_limit": 262144.0}),
                             (COMMANDS[0], {"deadline_seconds": 10, "output_limit": 65536})):
            with self.subTest(argv=argv), self.assertRaises(ValueError):
                DRIVER.inventory_helper_call(callback, self.report, argv, **kwargs)
        callback.assert_not_called()
        self.assertEqual(self.report, {"actions": [], "terminal_helper": None})

    def test_independent_literal_diagnostic_is_accepted_and_copied(self):
        action = independent_action(1, stderr=b"f", failed=True)
        diagnostic = independent_diagnostic(action, b"f")
        observed = DRIVER.validate_inventory_diagnostic(diagnostic, [action])
        self.assertEqual(observed, diagnostic)
        diagnostic["captures"]["stderr"]["size_bytes"] = 999
        self.assertEqual(observed["captures"]["stderr"]["size_bytes"], 1)

    def test_host_refuses_scalar_order_extra_and_nested_type_mutations(self):
        basis = independent_action(1, stderr=b"f", failed=True)
        mutations = {
            "boolean ordinal": lambda value: value.update(action_ordinal=True),
            "floating ordinal": lambda value: value.update(action_ordinal=1.0),
            "other ordinal": lambda value: value.update(action_ordinal=2),
            "other stage": lambda value: value.update(stage="private-stage"),
            "other reason": lambda value: value.update(reason="invented-warning"),
            "other schema": lambda value: value.update(schema="other/1"),
            "extra": lambda value: value.update(exception="invented text"),
            "boolean exit": lambda value: value.update(exit_code=False),
            "numeric spawn": lambda value: value.update(spawned=1),
            "floating count": lambda value: value["bytes_read"].update(stderr=1.0),
            "boolean nested capture": lambda value: value["captures"]["stderr"].update(size_bytes=True),
        }
        for label, change in mutations.items():
            with self.subTest(label=label):
                diagnostic = independent_diagnostic(basis, b"f")
                change(diagnostic)
                with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.validate_inventory_diagnostic(diagnostic, [basis])

    def test_host_refuses_noncanonical_base64_hash_length_and_completeness(self):
        basis = independent_action(1, stderr=b"f", failed=True)
        changes = {"padding bits": {"data": "Zh=="}, "whitespace": {"data": "Zg==\n"},
                   "bad alphabet": {"data": "!!!!"}, "nonascii": {"data": "\u4e2d"},
                   "overlarge": {"data": "A" * 5465}, "changed hash": {"sha256": "a" * 64},
                   "boolean length": {"retained_bytes": True}, "wrong length": {"retained_bytes": 2},
                   "false completeness": {"complete": False}, "false truncation": {"truncated": True},
                   "wrong scope": {"hash_scope": "captured-prefix"}, "other encoding": {"encoding": "utf-8"},
                   "extra": {"path": "invented"}}
        for label, change in changes.items():
            with self.subTest(label=label):
                diagnostic = independent_diagnostic(basis, b"f")
                diagnostic["stderr"].update(change)
                with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.validate_inventory_diagnostic(diagnostic, [basis])

    def test_host_rejects_action_mutations_and_nonlimit_sentinel_counts(self):
        good = independent_action(1)
        mutations = {"boolean ordinal": lambda row: row.update(ordinal=True),
                     "wrong argv": lambda row: row.update(argv=COMMANDS[1]),
                     "unknown spawn": lambda row: row.update(spawned=None),
                     "numeric spawn": lambda row: row.update(spawned=1),
                     "boolean exit": lambda row: row.update(exit_code=False),
                     "other status": lambda row: row.update(status="STARTUP_OBSERVED"),
                     "extra field": lambda row: row.update(stdout="not permitted")}
        for label, change in mutations.items():
            with self.subTest(label=label):
                row = copy.deepcopy(good)
                change(row)
                with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.validate_inventory_actions([row])
        row = independent_action(1, stderr=b"x" * 65536, failed=True)
        row["bytes_read"]["stderr"] = 65537
        row["captures"]["stderr"].update(complete=False, hash_scope="captured-prefix")
        with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.validate_inventory_actions([row])
        for rows in ([independent_action(2)], [good, good], [good] * 3,
                     [independent_action(1, stderr=b"f", failed=True), independent_action(2)]):
            with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.validate_inventory_actions(rows)

    def test_incomplete_or_malformed_observation_is_unknown_not_zero(self):
        action = independent_action(1, stderr=b"f", failed=True)
        envelope = original_envelope([action], independent_diagnostic(action, b"f"))
        for value, complete in ((envelope, False), (envelope, 1), (None, True),
                               ({"status": "PASS", "actions": [], "terminal_helper": None}, True)):
            observed = DRIVER.inventory_helper_observation(value, complete=complete)
            self.assertEqual(observed["observation"], "UNKNOWN")
            self.assertIsNone(observed["attempted"])
            self.assertIsNone(observed["spawned"])

    def test_complete_failure_is_adopted_before_actual_outer_nonzero_refusal(self):
        action = independent_action(1, stderr=b"f", failed=True)
        diagnostic = independent_diagnostic(action, b"f")
        envelope = original_envelope([action], diagnostic)
        raw = json.dumps(envelope).encode()
        report = {}
        with self.assertRaisesRegex(DRIVER.InventoryDiagnosticError, "^required-inventory-helper$"):
            DRIVER.observe_inventory_result(result(stdout=raw, status="FAIL", exit_code=1), report, envelope["environment_identity"])
        self.assertEqual(report["nested_helpers"]["terminal_helper"], diagnostic)
        self.assertEqual((report["nested_helpers"]["attempted"], report["nested_helpers"]["spawned"]), (1, 1))
        self.assertEqual(report["container_environment"]["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})
        for status in ("TIMEOUT", "OUTPUT_LIMIT"):
            prefix_report = {}
            with self.assertRaisesRegex(DRIVER.InventoryDiagnosticError, "^incomplete-inventory-envelope$"):
                DRIVER.observe_inventory_result(result(stdout=raw, status=status, exit_code=-9), prefix_report, envelope["environment_identity"])
            self.assertIsNone(prefix_report["nested_helpers"]["attempted"])

    def test_outer_pass_requires_source_caps_environment_user_and_complete_helpers(self):
        envelope = original_envelope([independent_action(1), independent_action(2)])
        raw = json.dumps(envelope).encode()
        self.assertEqual(DRIVER.observe_inventory_result(result(stdout=raw), {}, envelope["environment_identity"]), envelope)
        for count in (False, 1, 65537):
            with self.subTest(outer_stderr_read=count), self.assertRaises(DRIVER.InventoryDiagnosticError):
                DRIVER.observe_inventory_result(result(stdout=raw, counts={"stdout": len(raw), "stderr": count}), {}, envelope["environment_identity"])
        changes = {"missing source": lambda value: value.update(source_loads=[]),
                   "boolean vendor zero": lambda value: value.update(vendor_passes=False),
                   "boolean uid": lambda value: value.update(uid_gid=[True, 1000]),
                   "missing closing": lambda value: value["closing_audits"].pop("caps"),
                   "closing failure": lambda value: value["closing_failures"].append({"kind": "caps"}),
                   "cap mismatch": lambda value: value["caps_after"].update(memory_max_bytes=1),
                   "boolean cap": lambda value: value["caps_after"].update(memory_swap_max_bytes=False),
                   "wrong env": lambda value: value["environment_after_identity"].update(sha256="a" * 64),
                   "wrong inventory": lambda value: value["inventory_identity"].update(size_bytes=0)}
        for label, change in changes.items():
            with self.subTest(label=label):
                value = copy.deepcopy(envelope)
                change(value)
                with self.assertRaises(DRIVER.InventoryDiagnosticError):
                    DRIVER.observe_inventory_result(result(stdout=json.dumps(value).encode()), {}, envelope["environment_identity"])

    def test_json_parser_refuses_duplicates_depth_bounds_utf8_and_trailing_data(self):
        for raw in (b'{"status":"FAIL","status":"PASS"}', b'{"nested":{"x":1,"x":2}}',
                    b"[" * 33 + b"0" + b"]" * 33, b"\xff", b"{} trailing", b"x" * (4 * 1024**2 + 1),
                    b'{"status":"FAIL","value":NaN}', b'{"status":"FAIL","value":Infinity}'):
            with self.subTest(size=len(raw)), self.assertRaises(DRIVER.InventoryDiagnosticError):
                DRIVER.parse_inventory_envelope(raw)

    def test_live_producer_and_independent_host_fragments_agree(self):
        source = self.raw_source()
        namespace = {"hashlib": hashlib, "json": json}
        exec(compile(DRIVER.inventory_helper_fragment(source, "PRODUCER"), "original-producer-fragment", "exec"), namespace)
        common = DRIVER.public_source_fragment(source, "LOADER").split(b"# BEGIN PUBLIC SOURCE LOADER\n", 1)[0]
        self.assertTrue(DRIVER.inventory_helper_fragment(source, "PRODUCER").startswith(common))
        self.assertIs(namespace["INVENTORY_ERROR_TYPES"], namespace["PUBLIC_SOURCE_ERROR_TYPES"])
        report = {"actions": [], "terminal_helper": None}
        def callback(*args, **kwargs):
            global SYNTHETIC_CALLBACKS
            SYNTHETIC_CALLBACKS += 1
            return result(stderr=b"f")
        with self.assertRaises(ValueError):
            namespace["inventory_helper_call"](callback, report, COMMANDS[0], deadline_seconds=10, output_limit=262144)
        independent = {"hashlib": hashlib, "json": json}
        exec(compile(DRIVER.inventory_helper_fragment(source, "HOST"), "original-independent-host-fragment", "exec"), independent)
        self.assertEqual(independent["validate_inventory_diagnostic"](report["terminal_helper"], report["actions"]), report["terminal_helper"])
        validator = {"hashlib": hashlib, "json": json}
        raw_validator = DRIVER.inventory_helper_fragment(source, "VALIDATOR")
        self.assertTrue(raw_validator.startswith(common))
        exec(compile(raw_validator, "original-standalone-validator", "exec"), validator)
        self.assertEqual(validator["validate_inventory_diagnostic"](report["terminal_helper"], report["actions"]), report["terminal_helper"])

    def test_full_assembly_preserves_loader_and_significant_tokens_under_both_caps(self):
        source = self.raw_source()
        assembled = DRIVER.inventory_entry_source(source)
        self.assertLessEqual(len(assembled), 16384)
        self.assertIn(DRIVER.public_source_fragment(source, "LOADER"), assembled)
        self.assertIn(DRIVER.INVENTORY_ENTRY_PRELUDE.encode(), assembled)
        self.assertIn(DRIVER.INVENTORY_ENTRY_BODY.encode(), assembled)
        def significant(text):
            return [(token.type, "INDENT" if token.type == tokenize.INDENT else token.string)
                    for token in tokenize.generate_tokens(io.StringIO(text).readline)
                    if token.type not in (tokenize.COMMENT, tokenize.NL)]
        producer = DRIVER.inventory_helper_fragment(source, "PRODUCER").decode()
        producer = producer[producer.index("# BEGIN INVENTORY HELPER COMMON\n"):]
        self.assertEqual(significant(producer), significant(DRIVER._compact_inventory_entry_piece(producer, 4)))
        self.assertEqual(ast.dump(ast.parse(producer)), ast.dump(ast.parse(DRIVER._compact_inventory_entry_piece(producer, 4))))

    def test_actual_complete_entry_success_has_all_closing_checks(self):
        envelope, code, events, pins, raw = self.entry([result(), result(stdout=b"fonts\n")])
        self.assertEqual((code, envelope["status"]), (0, "PASS"))
        self.assertIsNone(envelope["terminal_helper"])
        self.assertEqual(events.count("helper"), 2)
        self.assertEqual(events.count("caps"), 2)
        self.assertEqual(events.count("environment"), 2)
        self.assertEqual(events.count("uid"), 2)
        DRIVER.require_inventory_success(envelope, envelope["environment_identity"], pins=pins)

    def test_actual_entry_primary_diagnostic_survives_all_protected_closing_faults(self):
        for fault in (None, "caps", "environment", "user", "all"):
            with self.subTest(fault=fault):
                envelope, code, events, pins, raw = self.entry([result(stderr=b"original refusal")], closing_fault=fault)
                self.assertEqual((code, envelope["status"]), (1, "FAIL"))
                self.assertEqual(envelope["error_type"], "ValueError")
                self.assertEqual(envelope["terminal_helper"]["reason"], "HELPER_STDERR_NOT_EMPTY")
                self.assertEqual(base64.b64decode(envelope["terminal_helper"]["stderr"]["data"]), b"original refusal")
                self.assertEqual((events.count("caps"), events.count("environment"), events.count("uid")), (2, 2, 2))
                self.assertEqual(events.count("helper"), 1)
                if fault == "all": self.assertEqual(envelope["closing_audits"], {"caps": "FAIL", "environment": "FAIL", "user": "FAIL"})
                DRIVER.validate_inventory_diagnostic(envelope["terminal_helper"], envelope["actions"])

    def test_actual_entry_missing_source_has_no_helpers_but_closes(self):
        envelope, code, events, pins, raw = self.entry([], source_failure=True)
        self.assertEqual((code, envelope["status"]), (1, "FAIL"))
        self.assertEqual(envelope["source_loads"][0]["reason"], "PUBLIC_SOURCE_READ_FAILED")
        self.assertEqual(envelope["actions"], [])
        self.assertIsNone(envelope["terminal_helper"])
        self.assertEqual(envelope["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})
        self.assertEqual(events.count("helper"), 0)
        DRIVER.validate_source_loads(envelope["source_loads"], pins=pins)

    def test_actual_entry_oversized_inventory_becomes_bounded_fail(self):
        envelope, code, events, pins, raw = self.entry([result(), result()], output_value={"original": "x" * (4 * 1024**2)})
        self.assertEqual((code, envelope["status"]), (1, "FAIL"))
        self.assertEqual(envelope["envelope_refusal"], "envelope-size-refusal")
        self.assertTrue(envelope["inventory_bytes_omitted"])
        self.assertIsNone(envelope["inventory_json"])
        self.assertEqual(len(envelope["actions"]), 2)
        self.assertEqual(envelope["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})
        self.assertLess(len(raw), 256 * 1024)

    def test_actual_entry_write_failure_preserves_primary_and_closing_ledger(self):
        for fault in ("write", "flush"):
            with self.subTest(fault=fault):
                envelope, code, events, pins, raw = self.entry([result(stderr=b"original refusal")], write_fault=fault)
                self.assertIsNone(code)
                self.assertEqual(envelope["error_type"], "ValueError")
                self.assertEqual(envelope["terminal_helper"]["reason"], "HELPER_STDERR_NOT_EMPTY")
                self.assertEqual(envelope["closing_audits"], {"caps": "PASS", "environment": "PASS", "user": "PASS"})
                self.assertEqual((events.count("helper"), events.count("caps"), events.count("environment"), events.count("uid")), (1, 2, 2, 2))
                if fault == "write": self.assertEqual(raw, b"")
                else: self.assertEqual(DRIVER.parse_inventory_envelope(raw), envelope)

    def test_size_refusal_preserves_primary_diagnostic_and_maximum_metadata_fits(self):
        self.call(result())
        with self.assertRaises(ValueError): self.call(result(stderr=b"x" * 65537, status="OUTPUT_LIMIT", exit_code=-9), 2)
        envelope = original_envelope(copy.deepcopy(self.report["actions"]), copy.deepcopy(self.report["terminal_helper"]))
        envelope.update(reason="original-primary", error_type="ValueError", inventory_json="x" * (4 * 1024**2))
        for key in ("caps_before", "caps_after"):
            envelope[key]["memory_events"].update({("original_" + str(index)).ljust(64, "\U0001f642"): 2**63 - 1 for index in range(62)})
        original = copy.deepcopy(envelope["terminal_helper"])
        raw = DRIVER.inventory_envelope_bytes(envelope, maximum=256 * 1024)
        # This actual inline envelope's maximum bounded metadata also fits
        # the unchanged external host's 256 KiB receipt budget.
        self.assertLess(len(raw), 256 * 1024)
        self.assertEqual(envelope["reason"], "original-primary")
        self.assertEqual(envelope["terminal_helper"], original)
        self.assertEqual(envelope["envelope_refusal"], "envelope-size-refusal")
        report = {}
        with self.assertRaisesRegex(DRIVER.InventoryDiagnosticError, "^required-inventory-helper$"):
            DRIVER.observe_inventory_result(result(stdout=raw, status="FAIL", exit_code=1), report, envelope["environment_identity"])
        self.assertLess(len(json.dumps(report, indent=2, ensure_ascii=True).encode()), 256 * 1024)
        self.assertEqual(report["nested_helpers"]["terminal_helper"], original)
        self.assertEqual(report["nested_helpers"]["attempted"], 2)
        with self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.inventory_envelope_bytes(envelope, maximum=256)
        self.assertEqual(envelope["terminal_helper"], original)

    def test_no_arguments_return_zero_before_parser_environment_clock_and_execute(self):
        class RefusingEnvironment(dict):
            def get(self, *args): raise AssertionError("environment read")
            def __getitem__(self, key): raise AssertionError("environment read")
            def __iter__(self): raise AssertionError("environment read")
        for argv in ([], None):
            with self.subTest(argv=argv), mock.patch.object(sys, "argv", ["original-startup"]), \
                    mock.patch.object(DRIVER.argparse, "ArgumentParser", side_effect=AssertionError("parser constructed")) as parser, \
                    mock.patch.object(DRIVER.time, "monotonic", side_effect=AssertionError("clock read")) as clock, \
                    mock.patch.object(DRIVER, "execute", side_effect=AssertionError("execute called")) as execute, \
                    mock.patch.dict("os.environ", {}, clear=True), mock.patch("os.environ", RefusingEnvironment()), \
                    mock.patch("sys.stdout", new_callable=io.StringIO) as output:
                self.assertEqual(DRIVER.main(argv), 0)
                self.assertEqual(json.loads(output.getvalue()), {"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0})
                parser.assert_not_called(); clock.assert_not_called(); execute.assert_not_called()

    def test_fragment_markers_refuse_missing_duplicate_order_and_unbounded_source(self):
        source = self.raw_source()
        validator = source.index(b"# BEGIN INVENTORY HELPER VALIDATOR\n")
        validator_end = source.index(b"# END INVENTORY HELPER VALIDATOR\n") + len(b"# END INVENTORY HELPER VALIDATOR\n")
        host = source.index(b"# BEGIN INVENTORY HELPER HOST\n")
        host_end = source.index(b"# END INVENTORY HELPER HOST\n") + len(b"# END INVENTORY HELPER HOST\n")
        reordered = source[:validator] + source[host:host_end] + source[validator_end:host] + source[validator:validator_end] + source[host_end:]
        for raw, part in ((b"original", "PRODUCER"), (source + b"# BEGIN INVENTORY HELPER COMMON\n", "PRODUCER"),
                          (reordered, "HOST"), (b"x" * 65537, "HOST"), (source, "PRIVATE"), (source.decode(), "PRODUCER")):
            with self.subTest(part=part), self.assertRaises(DRIVER.InventoryDiagnosticError): DRIVER.inventory_helper_fragment(raw, part)


if __name__ == "__main__":
    program = unittest.main(verbosity=2, exit=False)
    print(json.dumps({"synthetic_helper_callbacks": SYNTHETIC_CALLBACKS, "forbidden_effect_attempts": FORBIDDEN_ATTEMPTS,
                      "actual_children": 0, "vendor_passes": 0, "compatibility": "NOT_RUN"}, sort_keys=True))
    raise SystemExit(0 if program.result.wasSuccessful() and not program.result.skipped and FORBIDDEN_ATTEMPTS == 0 else 1)
