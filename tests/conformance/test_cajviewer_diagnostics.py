# SPDX-License-Identifier: MIT
"""Original diagnostic/process controls; no Docker, X11 or vendor execution."""

import base64
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_cajviewer_startup as startup

SESSION, DRIVER = startup.SESSION, startup.DRIVER
result = startup.WindowOwnershipTests.result


def report_basis():
    return {"protocol": "original-pdf-startup-v1", "input": "/input/digital.pdf", "status": "FAIL",
            "app_launch_attempts": 1, "vendor_passes": 0, "cleanup": "PASS", "actions": [],
            "controlled_helper_launches": 0, "elapsed_seconds": 1.0, "oom_kill_delta": 0,
            "before_metrics": {"memory.events": "oom_kill 0\n"},
            "after_helper_termination_metrics": {"memory.events": "oom_kill 0\n"}}


def simulate_session(helper, *, capture_error=True):
    """Drive the real supervisor using original process/helper stand-ins."""
    with tempfile.TemporaryDirectory() as temporary:
        output = Path(temporary)
        def path(value):
            original = Path(value)
            return output / original.relative_to("/output") if str(value).startswith("/output") else original
        clock = [0.0]
        def now():
            clock[0] += 0.00001
            return clock[0]
        def sleep(seconds):
            # Keep the original fake polling fast; advance the collection
            # lease without real sleeps or a reset observation deadline.
            if seconds != 0.1:
                clock[0] += max(0, seconds)
        children = [mock.Mock(pid=100 + index) for index in range(3)]
        for child in children:
            child.poll.return_value = None
        with mock.patch.object(SESSION, "Path", side_effect=path), \
                mock.patch.object(SESSION.resource, "setrlimit"), \
                mock.patch.object(SESSION, "run_bounded", side_effect=helper) as queries, \
                mock.patch.object(SESSION, "cgroup_metrics", return_value={"memory.events": "oom_kill 0\n"}) as metrics, \
                mock.patch.object(SESSION, "process_metadata", return_value=[]), \
                mock.patch.object(SESSION.subprocess, "Popen", side_effect=children) as launches, \
                mock.patch.object(SESSION.os, "getpgid", return_value=102), \
                mock.patch.object(SESSION.os, "killpg") as kill, \
                mock.patch.object(SESSION.time, "monotonic", side_effect=now), \
                mock.patch.object(SESSION.time, "sleep", side_effect=sleep), \
                mock.patch.object(SESSION, "capture_display", side_effect=SESSION.CanaryError("original capture fault")
                                  if capture_error else None) as capture, \
                contextlib.redirect_stdout(io.StringIO()):
            code = SESSION.main()
        payload = (output / "session.json").read_bytes()
        return {"code": code, "report": json.loads(payload), "size_bytes": len(payload),
                "helpers": queries.call_count, "launches": launches.call_count,
                "kills": kill.call_count, "metrics": metrics.call_count,
                "capture_calls": capture.call_count, "ready": (output / "ready").is_file()}


def simulate_host(session, *, artifact="valid"):
    """Collect a real original JSON file while mocking only Docker actions."""
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        calls = []
        def helper(argv, **kwargs):
            calls.append(argv)
            if argv[1:3] == ["container", "inspect"]:
                return result(b"[]\n", status="FAIL", code=1,
                              stderr=("No such container: " + argv[-1]).encode())
            if argv[1] == "inspect":
                return result(json.dumps([{"State": {"Running": True, "OOMKilled": False}}]).encode())
            if argv[1] == "exec":
                if kwargs.get("stdout_sink") is not None:
                    kwargs["stdout_sink"].write(b"original archive stand-in")
                else:
                    return result(b"READY\n")
            return result()
        def extract(archive, output):
            directory = output / "output"
            directory.mkdir(parents=True)
            path = directory / "session.json"
            if artifact == "valid":
                path.write_text(json.dumps(session))
            elif artifact == "corrupt":
                path.write_bytes(b"{original invalid JSON")
            elif artifact == "over-limit":
                path.write_bytes(b" " * (SESSION.SESSION_BYTES + 1))
            elif artifact == "symlink":
                target = root / "original.json"
                target.write_text(json.dumps(session))
                path.symlink_to(target)
            elif artifact != "missing":
                raise AssertionError("unknown original artifact mode")
            return {"members": 2, "size_bytes": 1}
        with mock.patch.object(DRIVER, "run_bounded", side_effect=helper), \
                mock.patch.object(DRIVER, "extract_output", side_effect=extract), \
                mock.patch.object(DRIVER, "verify_viewport_capture") as verify:
            observed = DRIVER.attempt("sha256:" + "0" * 64, root,
                                      root / "attempt", "cajviewer-124-original-diagnostic")
        return observed, calls, verify.call_count


class HelperDiagnosticTests(unittest.TestCase):
    def test_complete_success_and_exit_failure_hash_only_captured_bytes(self):
        for status, code in (("PASS", 0), ("FAIL", 1)):
            observed = result(b"original output\n", status=status, code=code, stderr=b"original error\n")
            capture = SESSION.helper_capture(observed)
            for name in ("stdout", "stderr"):
                self.assertEqual(capture[name]["captured_bytes"], len(observed[name]))
                self.assertEqual(capture[name]["sha256"], hashlib.sha256(observed[name]).hexdigest())
                self.assertTrue(capture[name]["complete"])
                self.assertFalse(capture[name]["truncated"])
                self.assertEqual(capture[name]["hash_scope"], "complete-stream")

    def test_actual_original_stderr_exit_one_search_is_terminal_failure(self):
        observed = DRIVER.run_bounded([sys.executable, "-c",
            "import sys; sys.stderr.buffer.write(b'original search error\\x00\\xff'); sys.exit(1)"],
            deadline_seconds=3, output_limit=4096)
        observed["action_index"] = 7
        with self.assertRaises(SESSION.SessionFailure) as failure:
            SESSION.observe_owned_window(mock.Mock(return_value=observed), 123, SESSION.time.monotonic() + 5)
        diagnostic = SESSION.terminal_failure(failure.exception, "unused")
        self.assertEqual((diagnostic["stage"], diagnostic["reason"], diagnostic["action_index"]),
                         ("visible-window-search", "helper-failed", 7))
        self.assertEqual(diagnostic["helper"]["exit_code"], 1)
        self.assertEqual(base64.b64decode(diagnostic["stderr"]["data"], validate=True), observed["stderr"])
        self.assertEqual(diagnostic["stderr"]["sha256"], hashlib.sha256(observed["stderr"]).hexdigest())
        self.assertTrue(diagnostic["stderr"]["complete"])
        self.assertNotIn("stdout", diagnostic)

    def test_actual_original_timeout_hashes_prefix_even_without_truncated_flag(self):
        observed = DRIVER.run_bounded([sys.executable, "-c",
            "import sys,time; print('original prefix',flush=True); sys.stderr.write('original warning'); sys.stderr.flush(); time.sleep(5)"],
            deadline_seconds=0.2, output_limit=4096)
        self.assertEqual(observed["status"], "TIMEOUT")
        self.assertFalse(observed["prefix_truncated"])
        diagnostic = SESSION.terminal_failure(SESSION.SessionFailure("window-owner", "timeout", observed), "unused")
        for stream in ("stdout", "stderr"):
            self.assertFalse(diagnostic["helper"]["capture"][stream]["complete"])
            self.assertEqual(diagnostic["helper"]["capture"][stream]["hash_scope"], "captured-prefix")
        self.assertFalse(diagnostic["stderr"]["complete"])

    def test_actual_original_output_limit_records_retained_prefix_and_read_count(self):
        observed = DRIVER.run_bounded([sys.executable, "-c",
            "import sys; sys.stderr.buffer.write(b'x'*8192); sys.stderr.flush()"],
            deadline_seconds=3, output_limit=4096)
        self.assertEqual(observed["status"], "OUTPUT_LIMIT")
        diagnostic = SESSION.terminal_failure(SESSION.SessionFailure("window-geometry", "output-limit", observed), "unused")
        self.assertEqual(diagnostic["helper"]["bytes_read"]["stderr"], 4097)
        self.assertEqual(diagnostic["helper"]["capture"]["stderr"]["captured_bytes"], 4096)
        self.assertEqual(len(base64.b64decode(diagnostic["stderr"]["data"])), 4096)
        self.assertTrue(diagnostic["stderr"]["truncated"])
        self.assertFalse(diagnostic["stderr"]["complete"])

    def test_readiness_excerpt_limit_does_not_claim_complete_large_stderr(self):
        observed = result(status="FAIL", code=1, stderr=b"x" * 65536)
        diagnostic = SESSION.terminal_failure(SESSION.SessionFailure("display-readiness", "readiness-failed", observed), "unused")
        self.assertEqual(diagnostic["helper"]["capture"]["stderr"]["captured_bytes"], 65536)
        self.assertEqual(diagnostic["stderr"]["retained_bytes"], 4096)
        self.assertTrue(diagnostic["stderr"]["truncated"])
        self.assertFalse(diagnostic["stderr"]["complete"])

    def test_invalid_helper_capture_cannot_claim_success(self):
        mutations = [dict(status="UNKNOWN"), dict(exit_code=True), dict(prefix_truncated="false"),
                     dict(stdout="not bytes"), dict(bytes_read={"stdout": -1, "stderr": 0})]
        for mutation in mutations:
            observed = result()
            observed.update(mutation)
            with self.subTest(mutation=mutation), self.assertRaises(SESSION.SessionFailure):
                SESSION.helper_capture(observed)

    def test_malformed_title_has_location_without_retaining_stdout_or_message(self):
        outcomes = [result(b"123\n"), result(b"102\n"), result(b"original secret\0title")]
        outcomes[-1]["action_index"] = 4
        with mock.patch.object(SESSION.os, "getpgid", return_value=102), self.assertRaises(SESSION.SessionFailure) as failure:
            SESSION.observe_owned_window(mock.Mock(side_effect=outcomes), 102, SESSION.time.monotonic() + 5)
        diagnostic = SESSION.terminal_failure(failure.exception, "unused")
        self.assertEqual((diagnostic["stage"], diagnostic["reason"], diagnostic["action_index"]), ("window-title", "malformed-title", 4))
        self.assertNotIn("original secret", json.dumps(diagnostic))
        self.assertNotIn("message", diagnostic)

    def test_unclassified_process_group_lookup_failure_is_located_not_missing_owner(self):
        outcome = {**result(b"102\n"), "action_index": 8}
        with mock.patch.object(SESSION.os, "getpgid", side_effect=OSError("original forbidden OS detail")), \
                self.assertRaises(SESSION.SessionFailure) as failure:
            SESSION.observe_owned_window(mock.Mock(side_effect=[result(b"123\n"), outcome]),
                                         102, SESSION.time.monotonic() + 5)
        diagnostic = SESSION.terminal_failure(failure.exception, "unused")
        self.assertEqual((diagnostic["stage"], diagnostic["reason"], diagnostic["action_index"]),
                         ("window-owner", "owner-check-failed", 8))
        self.assertNotIn("forbidden", json.dumps(diagnostic))


class SessionReceiptTests(unittest.TestCase):
    def test_terminal_search_failure_receipt_keeps_cleanup_and_final_measurements(self):
        def helper(argv, **kwargs):
            return result(b"display ready\n") if argv == ["xdpyinfo"] else result(status="FAIL", code=1, stderr=b"original search failure\n")
        observed = simulate_session(helper)
        report = observed["report"]
        self.assertEqual((observed["code"], report["status"], report["app_launch_attempts"]), (1, "FAIL", 1))
        self.assertEqual((report["terminal_failure"]["stage"], report["terminal_failure"]["action_index"]), ("visible-window-search", 5))
        action = report["actions"][4]
        self.assertEqual(action["capture"]["stderr"], report["terminal_failure"]["helper"]["capture"]["stderr"])
        self.assertEqual((observed["kills"], observed["metrics"], observed["capture_calls"]), (3, 3, 0))
        self.assertTrue(observed["ready"])
        self.assertEqual((report["cleanup"], report["oom_kill_delta"]), ("PASS", 0))
        self.assertLessEqual(observed["size_bytes"], SESSION.SESSION_BYTES)
        self.assertTrue(report["receipt_complete"])

    def test_readiness_timeout_and_output_limit_stop_before_app_launch(self):
        for status in ("TIMEOUT", "OUTPUT_LIMIT"):
            observed = simulate_session(lambda argv, **kwargs: result(status=status, code=-9, stderr=b"original prefix"))
            self.assertEqual((observed["code"], observed["launches"], observed["helpers"]), (1, 1, 1))
            report = observed["report"]
            self.assertEqual(report["app_launch_attempts"], 0)
            self.assertEqual(report["terminal_failure"]["stage"], "display-readiness")
            self.assertFalse(report["terminal_failure"]["helper"]["capture"]["stderr"]["complete"])
            self.assertEqual((report["cleanup"], observed["kills"], observed["metrics"]), ("PASS", 1, 3))

    def test_expected_readiness_negative_can_progress_but_search_stderr_cannot(self):
        outcomes = [result(status="FAIL", code=1, stderr=b"original display not ready"), result(b"display ready\n"),
                    result(status="FAIL", code=1, stderr=b"original search error")]
        observed = simulate_session(mock.Mock(side_effect=outcomes))
        self.assertEqual((observed["launches"], observed["helpers"]), (3, 3))
        self.assertEqual(observed["report"]["terminal_failure"]["stage"], "visible-window-search")
        self.assertEqual(observed["report"]["status"], "FAIL")

    def test_400_helper_ledger_and_limit_are_bounded_without_401st_launch(self):
        def helper(argv, **kwargs):
            return result(b"display ready\n") if argv == ["xdpyinfo"] else result(status="FAIL", code=1)
        observed = simulate_session(helper)
        self.assertEqual(observed["helpers"], 400)
        self.assertEqual(observed["report"]["controlled_helper_launches"], 400)
        self.assertEqual(observed["report"]["terminal_failure"]["reason"], "helper-call-limit")
        self.assertEqual(observed["report"]["status"], "FAIL")
        self.assertTrue(observed["report"]["receipt_complete"])
        self.assertEqual(sum(action["action"] == "helper-command" for action in observed["report"]["actions"]), 400)
        self.assertLessEqual(observed["size_bytes"], SESSION.SESSION_BYTES)

    def test_launch_error_has_action_index_without_arbitrary_exception_text(self):
        observed = simulate_session(mock.Mock(side_effect=OSError("original forbidden error detail")))
        failure = observed["report"]["terminal_failure"]
        self.assertEqual((failure["stage"], failure["reason"], failure["action_index"]),
                         ("display-readiness", "helper-result-unavailable", 2))
        self.assertEqual(failure["helper"], {"status": "UNAVAILABLE", "exit_code": None, "bytes_read": None, "capture": None})
        self.assertNotIn("forbidden", json.dumps(observed["report"]))
        self.assertEqual((observed["helpers"], observed["kills"], observed["metrics"]), (1, 1, 3))

    def test_unclassified_fault_and_interruption_fail_without_skipping_closing_checks(self):
        for error in (RuntimeError("original unclassified detail"), KeyboardInterrupt()):
            observed = simulate_session(mock.Mock(side_effect=error))
            self.assertEqual(observed["report"]["status"], "FAIL")
            self.assertEqual(observed["report"]["terminal_failure"]["reason"], "helper-result-unavailable")
            self.assertEqual((observed["kills"], observed["metrics"], observed["ready"]), (1, 3, True))
            self.assertNotIn("unclassified detail", json.dumps(observed["report"]))

    def test_exact_serialization_limit_and_one_byte_over_have_explicit_outcomes(self):
        basis = report_basis()
        basis["padding"] = ""
        empty_size = len(SESSION.session_payload(basis))
        basis["padding"] = "x" * (SESSION.SESSION_BYTES - empty_size)
        self.assertEqual(len(SESSION.session_payload(basis)), SESSION.SESSION_BYTES)
        self.assertTrue(basis["receipt_complete"])
        basis["padding"] += "x"
        payload = SESSION.session_payload(basis)
        self.assertLessEqual(len(payload), SESSION.SESSION_BYTES)
        self.assertEqual((basis["status"], basis["receipt_complete"]), ("FAIL", False))
        self.assertEqual(basis["receipt_refusal"]["unserialized_size_bytes"], SESSION.SESSION_BYTES + 1)
        self.assertTrue(basis["receipt_refusal"]["ledger_omitted"])

    def test_size_refusal_preserves_primary_failure_and_audits_not_large_process_records(self):
        basis = report_basis()
        basis["terminal_failure"] = SESSION.terminal_failure(SESSION.SessionFailure("visible-window-search", "helper-failed",
            {**result(status="FAIL", code=1, stderr=b"original terminal error"), "action_index": 400}), "unused")
        primary = copy.deepcopy(basis["terminal_failure"])
        basis["actions"] = [{"action": "helper-command"}] * 400
        basis["controlled_helper_launches"] = 400
        basis["processes_sampled_before_termination"] = [{"argv": ["x" * 8192]}] * 128
        payload = SESSION.session_payload(basis)
        self.assertEqual(basis["terminal_failure"], primary)
        self.assertEqual(basis["receipt_refusal"]["actions_omitted"], 400)
        self.assertNotIn("actions", basis)
        self.assertNotIn("processes_sampled_before_termination", basis)
        self.assertEqual((basis["cleanup"], basis["oom_kill_delta"]), ("PASS", 0))
        self.assertLessEqual(len(payload), SESSION.SESSION_BYTES)
        self.assertEqual(DRIVER.validate_session(basis, Path("/original/not-opened"))["reason"], "session-receipt-refused")
        self.assertEqual(SESSION.session_payload(basis), payload)
        self.assertFalse(basis["receipt_complete"])

    def test_maximum_query_capture_metadata_does_not_exceed_receipt_cap_silently(self):
        basis = report_basis()
        outcome = result(b"o" * 4096, status="FAIL", code=1, stderr=b"e" * 4096)
        argv = ["xdotool", "search", "--onlyvisible", "--maxdepth", "2", "--limit", "17", "--name", ".*"]
        basis["actions"] = [{"action": "helper-command", "stage": "visible-window-search", "argv": argv,
            "status": outcome["status"], "exit_code": outcome["exit_code"], "bytes_read": outcome["bytes_read"],
            "prefix_truncated": False, "capture": SESSION.helper_capture(outcome)} for _ in range(400)]
        basis["controlled_helper_launches"] = 400
        basis["terminal_failure"] = SESSION.terminal_failure(SESSION.SessionFailure("visible-window-search", "helper-failed",
            {**outcome, "action_index": 400, "argv": argv}), "unused")
        payload = SESSION.session_payload(basis)
        self.assertLessEqual(len(payload), SESSION.SESSION_BYTES)
        if basis["receipt_complete"]:
            self.assertEqual(len(basis["actions"]), 400)
        else:
            self.assertEqual(basis["status"], "FAIL")
            self.assertEqual(basis["receipt_refusal"]["actions_omitted"], 400)
            self.assertGreater(basis["receipt_refusal"]["unserialized_size_bytes"], SESSION.SESSION_BYTES)
        self.assertEqual(len(base64.b64decode(basis["terminal_failure"]["stderr"]["data"])), 4096)


class HostFailureTests(unittest.TestCase):
    def test_missing_special_corrupt_and_over_limit_receipts_never_verify_capture(self):
        for artifact in ("missing", "symlink", "corrupt", "over-limit"):
            observed, calls, capture_calls = simulate_host(report_basis(), artifact=artifact)
            self.assertEqual((observed["status"], observed["cleanup"], capture_calls), ("FAIL", "PASS", 0))
            self.assertEqual(observed["diagnostic_integrity"]["status"], "NOT_RUN")
            self.assertEqual(observed["app_launch_attempts"], "UNKNOWN_AFTER_START")
            self.assertEqual([call[1] for call in calls[-3:]], ["logs", "rm", "container"])

    def test_missing_capture_preserves_primary_failure_and_always_closes(self):
        basis = report_basis()
        basis["error_type"] = "SessionFailure"
        basis["terminal_failure"] = {"stage": "visible-window-search", "reason": "helper-failed", "action_index": 5}
        observed, calls, capture_calls = simulate_host(basis)
        self.assertEqual((observed["status"], observed["session_status"], observed["cleanup"]), ("FAIL", "FAIL", "PASS"))
        self.assertEqual(observed["primary_failure"]["terminal_failure"], basis["terminal_failure"])
        self.assertEqual(observed["diagnostic_integrity"]["status"], "NOT_RUN")
        self.assertEqual(observed["diagnostic_integrity"]["reason"], "capture-missing")
        self.assertNotIn("error_type", observed)
        self.assertEqual(capture_calls, 0)
        self.assertEqual([call[1] for call in calls[-3:]], ["logs", "rm", "container"])

    def test_missing_capture_cannot_complete_a_claimed_startup(self):
        basis = report_basis()
        basis["status"] = "STARTUP_OBSERVED"
        observed, _, capture_calls = simulate_host(basis)
        self.assertEqual((observed["status"], observed["cleanup"], capture_calls), ("FAIL", "PASS", 0))
        self.assertEqual(observed["diagnostic_integrity"]["status"], "NOT_RUN")
        self.assertNotIn("error_type", observed)

    def test_missing_capture_still_requires_cleanup_and_oom_evidence(self):
        for mutation in ({"cleanup": "FAIL"}, {"after_helper_termination_metrics": {"memory.events": "oom_kill 1\n"}},
                         {"after_helper_termination_metrics": {"memory.events": "UNAVAILABLE"}}):
            basis = report_basis()
            basis.update(mutation)
            with self.subTest(mutation=mutation), self.assertRaises(DRIVER.CanaryError):
                DRIVER.validate_session(basis, Path("/original/not-opened"))

    def test_nonobject_or_malformed_audit_receipt_is_explicit_failure(self):
        for basis in ([], {**report_basis(), "before_metrics": []}):
            observed, calls, capture_calls = simulate_host(basis)
            self.assertEqual((observed["status"], observed["cleanup"], capture_calls), ("FAIL", "PASS", 0))
            self.assertEqual(observed["error_type"], "CanaryError")
            self.assertEqual([call[1] for call in calls[-3:]], ["logs", "rm", "container"])

    def test_actual_no_input_cli_reports_zero_work(self):
        observed = DRIVER.run_bounded([sys.executable, str(startup.ROOT / "tools/cajviewer/run.py")],
                                      deadline_seconds=3, output_limit=4096)
        self.assertEqual(observed["status"], "PASS")
        self.assertEqual(json.loads(observed["stdout"]), {"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0})


if __name__ == "__main__":
    unittest.main()
