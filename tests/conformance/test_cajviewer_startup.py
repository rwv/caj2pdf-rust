# SPDX-License-Identifier: MIT
"""Original simulated startup/daemon faults; never execute vendor/Docker code."""

import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import tempfile
import tarfile
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SESSION = load("original_session", ROOT / "tools/cajviewer/cajviewer_session.py")
DRIVER = load("original_startup_driver", ROOT / "tools/cajviewer/run.py")


class WindowOwnershipTests(unittest.TestCase):
    @staticmethod
    def result(payload=b"", *, status="PASS", code=0, stderr=b""):
        return {"status": status, "exit_code": code, "stdout": payload, "stderr": stderr,
                "bytes_read": {"stdout": len(payload), "stderr": len(stderr)},
                "prefix_truncated": status == "OUTPUT_LIMIT"}

    @staticmethod
    def geometry(window="123"):
        return f"WINDOW={window}\nX=0\nY=0\nWIDTH=600\nHEIGHT=400\nSCREEN=0\n".encode()

    def test_real_original_process_groups_prove_owned_and_refuse_foreign_windows(self):
        children = []
        try:
            for _ in range(2):
                children.append(subprocess.Popen([sys.executable, "-c", "import signal; signal.pause()"],
                    start_new_session=True, stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
            owned, foreign = children
            self.assertEqual(os.getpgid(owned.pid), owned.pid)
            self.assertNotEqual(os.getpgid(foreign.pid), owned.pid)
            def query(argv, **kwargs):
                if argv[1] == "search":
                    self.assertEqual(argv, ["xdotool", "search", "--onlyvisible", "--maxdepth", "2",
                                            "--limit", "17", "--name", ".*"])
                    return self.result(b"122\n123\n")
                if argv[1] == "getwindowpid":
                    return self.result(str(foreign.pid if argv[-1] == "122" else owned.pid).encode() + b"\n")
                self.assertEqual(argv[-1], "123")
                return self.result(b"Original application\n" if argv[1] == "getwindowname" else self.geometry())
            command = mock.Mock(side_effect=query)
            observed = SESSION.observe_owned_window(command, owned.pid, time.monotonic() + 5)
            self.assertEqual((observed["id"], observed["pid"], observed["process_group"]), ("123", owned.pid, owned.pid))
            self.assertEqual(observed["scope"], "startup-owned-visible-window-only")
            self.assertEqual(observed["document_identity"], "UNVERIFIED")
            self.assertNotIn("digital.pdf", observed["title"])
            self.assertEqual(command.call_count, 6)
            self.assertTrue(all(0 < call.kwargs["deadline_seconds"] <= 2 for call in command.call_args_list))
        finally:
            for child in children:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=5)

    def test_missing_zero_and_malformed_pid_never_measure_a_window(self):
        outcomes = [self.result(data) for data in (b"", b"0\n", b"unknown\n", b"12 34\n", b"2147483648\n")]
        outcomes.append(self.result(status="FAIL", code=1, stderr=b"missing _NET_WM_PID\n"))
        for outcome in outcomes:
            with self.subTest(outcome=outcome):
                command = mock.Mock(side_effect=[self.result(b"123\n"), outcome])
                with mock.patch.object(SESSION.os, "getpgid") as group:
                    self.assertIsNone(SESSION.observe_owned_window(command, 101, time.monotonic() + 5))
                group.assert_not_called()
                self.assertEqual(command.call_count, 2)

    def test_disappeared_or_inaccessible_process_is_not_an_owned_window(self):
        for error in (ProcessLookupError(), PermissionError()):
            with self.subTest(error=type(error).__name__):
                command = mock.Mock(side_effect=[self.result(b"123\n"), self.result(b"101\n")])
                with mock.patch.object(SESSION.os, "getpgid", side_effect=error):
                    self.assertIsNone(SESSION.observe_owned_window(command, 101, time.monotonic() + 5))
                self.assertEqual(command.call_count, 2)

    def test_stale_pid_and_group_after_measurement_refuse_the_candidate(self):
        for final_pid, groups in ((b"102\n", [101, 101]), (b"101\n", [101, 102]),
                                  (b"101\n", [101, ProcessLookupError()])):
            with self.subTest(pid=final_pid, groups=groups):
                command = mock.Mock(side_effect=[self.result(b"123\n"), self.result(b"101\n"),
                    self.result(b"Original title\n"), self.result(self.geometry()), self.result(final_pid)])
                with mock.patch.object(SESSION.os, "getpgid", side_effect=groups):
                    self.assertIsNone(SESSION.observe_owned_window(command, 101, time.monotonic() + 5))
                self.assertEqual(command.call_count, 5)

    def test_window_count_limit_is_checked_before_owner_queries(self):
        for count in (16, 17):
            command = mock.Mock(side_effect=[self.result(b"".join(f"{number}\n".encode() for number in range(1, count + 1)))]
                                           + [self.result(b"0\n")] * count)
            with self.subTest(count=count), mock.patch.object(SESSION.os, "getpgid") as group:
                if count == 16:
                    self.assertIsNone(SESSION.observe_owned_window(command, 101, time.monotonic() + 5))
                    self.assertEqual(command.call_count, 17)
                else:
                    with self.assertRaisesRegex(SESSION.CanaryError, "candidate count"):
                        SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
                    self.assertEqual(command.call_count, 1)
                group.assert_not_called()

    def test_search_framing_and_helper_faults_never_become_empty_searches(self):
        invalid = [self.result(data) for data in (b"0\n", b"-1\n", b"bad\n", b"123\n123\n", b"01\n1\n", b"123\n\n")]
        invalid += [self.result(status=status, code=-9) for status in ("TIMEOUT", "OUTPUT_LIMIT")]
        invalid += [self.result(status="FAIL", code=1, stderr=b"display unavailable\n"),
                    self.result(status="FAIL", code=2),
                    self.result(stderr=b"unexpected warning\n")]
        for outcome in invalid:
            with self.subTest(outcome=outcome):
                command = mock.Mock(return_value=outcome)
                with self.assertRaises(SESSION.CanaryError):
                    SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
                self.assertEqual(command.call_count, 1)
        for outcome in (self.result(), self.result(status="FAIL", code=1)):
            self.assertIsNone(SESSION.observe_owned_window(mock.Mock(return_value=outcome), 101, time.monotonic() + 5))

    def test_timeout_or_output_limit_at_every_owner_measurement_is_failure(self):
        valid = [self.result(b"123\n"), self.result(b"101\n"), self.result(b"Original title\n"),
                 self.result(self.geometry()), self.result(b"101\n")]
        for index in range(1, len(valid)):
            for status in ("TIMEOUT", "OUTPUT_LIMIT"):
                command = mock.Mock(side_effect=valid[:index] + [self.result(status=status, code=-9)])
                with self.subTest(index=index, status=status), mock.patch.object(SESSION.os, "getpgid", return_value=101):
                    with self.assertRaises(SESSION.CanaryError):
                        SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
                    self.assertEqual(command.call_count, index + 1)

    def test_malformed_geometry_title_and_unclassified_refinement_stderr_fail(self):
        geometries = [b"WIDTH=600\nHEIGHT=400\n", self.geometry().replace(b"WINDOW=123", b"WINDOW=124"),
                      self.geometry().replace(b"WIDTH=600", b"WIDTH=0"),
                      self.geometry() + b"SCREEN=0\n", self.geometry().replace(b"X=0", b"X=unknown")]
        for payload in geometries:
            command = mock.Mock(side_effect=[self.result(b"123\n"), self.result(b"101\n"),
                self.result(b"Original title\n"), self.result(payload)])
            with self.subTest(payload=payload), mock.patch.object(SESSION.os, "getpgid", return_value=101):
                with self.assertRaises(SESSION.CanaryError):
                    SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
        for payload in (b"bad\0title", b"\xff"):
            command = mock.Mock(side_effect=[self.result(b"123\n"), self.result(b"101\n"), self.result(payload)])
            with self.subTest(payload=payload), mock.patch.object(SESSION.os, "getpgid", return_value=101):
                with self.assertRaises((SESSION.CanaryError, UnicodeError)):
                    SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
        for stage in (2, 3):
            valid = [self.result(b"123\n"), self.result(b"101\n"), self.result(b"Original title\n")]
            command = mock.Mock(side_effect=valid[:stage] + [self.result(status="FAIL", code=1, stderr=b"BadWindow\n")])
            with self.subTest(stage=stage), mock.patch.object(SESSION.os, "getpgid", return_value=101):
                with self.assertRaises(SESSION.SessionFailure) as failure:
                    SESSION.observe_owned_window(command, 101, time.monotonic() + 5)
                self.assertEqual(failure.exception.reason, "helper-failed")

    def test_queries_share_deadline_and_do_not_reset_after_elapsed_query(self):
        command = mock.Mock(return_value=self.result(b"123\n"))
        with mock.patch.object(SESSION.time, "monotonic", side_effect=[0, 1]):
            with self.assertRaisesRegex(SESSION.CanaryError, "deadline"):
                SESSION.observe_owned_window(command, 101, 1)
        self.assertEqual(command.call_count, 1)
        self.assertEqual(command.call_args.kwargs["deadline_seconds"], 1)
        command.reset_mock()
        with mock.patch.object(SESSION.time, "monotonic", return_value=1):
            with self.assertRaisesRegex(SESSION.CanaryError, "deadline"):
                SESSION.observe_owned_window(command, 101, 1)
        command.assert_not_called()


class StartupFailureTests(unittest.TestCase):
    def test_original_process_limits_are_observed_and_disappeared_process_is_omitted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            process = root / "123"
            process.mkdir()
            command = process / "cmdline"
            command.write_bytes(b"original-app\0original.pdf\0")
            (process / "limits").write_bytes(b"Limit Soft Limit Hard Limit Units\n"
                                             b"Max file size 67108864 67108864 bytes\n")
            paths = [command, root / "124" / "cmdline"]
            with mock.patch.object(SESSION, "Path", return_value=mock.Mock(glob=mock.Mock(return_value=paths))):
                observations = SESSION.process_metadata()
            self.assertEqual(observations, [{"pid": 123, "argv": ["original-app", "original.pdf"],
                                             "observed_file_size_limit": "Max file size 67108864 67108864 bytes"}])

    def test_oversized_process_limit_metadata_fails_without_truncating(self):
        with tempfile.TemporaryDirectory() as temporary:
            process = Path(temporary) / "123"
            process.mkdir()
            command = process / "cmdline"
            command.write_bytes(b"original-app\0")
            (process / "limits").write_bytes(b"x" * 8193)
            with mock.patch.object(SESSION, "Path", return_value=mock.Mock(glob=mock.Mock(return_value=[command]))):
                with self.assertRaisesRegex(SESSION.CanaryError, "limits metadata over limit"):
                    SESSION.process_metadata()

    def test_original_child_file_limit_preserves_parent_and_refuses_over_limit(self):
        # Three original Python processes, no Docker/vendor invocation. The
        # file operations prove the inherited rejection and amended ceiling.
        script = r'''
import errno, importlib.util, json, os, resource, signal, sys
from pathlib import Path
root = Path(sys.argv[1])
sys.path.insert(0, str(root / "scripts"))
spec = importlib.util.spec_from_file_location("original_session", root / "tools/cajviewer/cajviewer_session.py")
session = importlib.util.module_from_spec(spec)
spec.loader.exec_module(session)
resource.setrlimit(resource.RLIMIT_FSIZE, (1024 ** 2, session.RUNTIME_FILE_LIMIT))
if sys.argv[2] != "inherited":
    session.runtime_file_limit()
signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
target = Path(sys.argv[3])
refused = False
with target.open("xb") as file:
    try:
        if sys.argv[2] == "positive":
            for _ in range(16):
                file.write(b"x" * 65536)
            file.write(b"x")
            file.flush()
        else:
            size = 1024 ** 2 + 1 if sys.argv[2] == "inherited" else session.RUNTIME_FILE_LIMIT + 1
            os.ftruncate(file.fileno(), size)
    except OSError as error:
        if error.errno != errno.EFBIG:
            raise
        refused = True
print(json.dumps({"refused": refused, "size": target.stat().st_size,
                  "limits": resource.getrlimit(resource.RLIMIT_FSIZE)}))
'''
        parent_limits = resource.getrlimit(resource.RLIMIT_FSIZE)
        with tempfile.TemporaryDirectory() as temporary:
            results = {}
            for mode in ("inherited", "positive", "over-limit"):
                observed = DRIVER.run_bounded([sys.executable, "-c", script, str(ROOT), mode,
                                               str(Path(temporary) / mode)],
                                              deadline_seconds=5, output_limit=4096)
                self.assertEqual(observed["status"], "PASS", observed)
                results[mode] = json.loads(observed["stdout"])
        self.assertTrue(results["inherited"]["refused"])
        self.assertEqual(results["inherited"]["size"], 0)
        self.assertEqual(results["inherited"]["limits"], [1024 ** 2, SESSION.RUNTIME_FILE_LIMIT])
        self.assertEqual(results["positive"], {"refused": False, "size": 1024 ** 2 + 1,
                                              "limits": [SESSION.RUNTIME_FILE_LIMIT] * 2})
        self.assertTrue(results["over-limit"]["refused"])
        self.assertEqual(results["over-limit"]["size"], 0)
        self.assertEqual(resource.getrlimit(resource.RLIMIT_FSIZE), parent_limits)

    def test_original_transport_streams_known_files_and_refuses_unsafe_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            output.mkdir()
            (output / "session.json").write_bytes(b'{"original": true}\n')
            (output / "ready").write_bytes(b"original-ready\n")
            archive = root / "control.tar"
            with archive.open("xb") as sink:
                result = DRIVER.run_bounded([sys.executable, "-c", DRIVER.COLLECT_PROGRAM, str(output)],
                                            deadline_seconds=5, output_limit=65536, stdout_sink=sink)
            self.assertEqual(result["status"], "PASS", result)
            DRIVER.extract_output(archive, root / "capture")
            self.assertEqual((root / "capture/output/session.json").read_bytes(), b'{"original": true}\n')
            with tarfile.open(archive) as stream:
                self.assertEqual({member.name for member in stream}, {"output", "output/session.json", "output/ready"})
            (output / "unexpected").write_bytes(b"original extra")
            result = DRIVER.run_bounded([sys.executable, "-c", DRIVER.COLLECT_PROGRAM, str(output)],
                                        deadline_seconds=5, output_limit=65536)
            self.assertEqual(result["status"], "FAIL")
            (output / "unexpected").unlink()
            (output / "startup.ppm").symlink_to(root / "outside")
            (root / "outside").write_bytes(b"original external bytes")
            result = DRIVER.run_bounded([sys.executable, "-c", DRIVER.COLLECT_PROGRAM, str(output)],
                                        deadline_seconds=5, output_limit=65536)
            self.assertEqual(result["status"], "FAIL")

    def test_original_transport_refuses_oversized_regular_files_and_fifos(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            file = output / "startup.ppm"
            with file.open("wb") as sink:
                sink.truncate(6 * 1024 ** 2 + 1)
            result = DRIVER.run_bounded([sys.executable, "-c", DRIVER.COLLECT_PROGRAM, str(output)],
                                        deadline_seconds=5, output_limit=65536)
            self.assertEqual(result["status"], "FAIL")
            file.unlink()
            os.mkfifo(file)
            result = DRIVER.run_bounded([sys.executable, "-c", DRIVER.COLLECT_PROGRAM, str(output)],
                                        deadline_seconds=5, output_limit=65536)
            self.assertEqual(result["status"], "FAIL")

    def test_window_observed_then_capture_failure_is_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            def path(value):
                original = Path(value)
                return output / original.relative_to("/output") if str(value).startswith("/output") else original
            elapsed = [0]
            def now():
                elapsed[0] += 0.2
                return elapsed[0]
            def sleep(value):
                elapsed[0] += value
            def command(argv, **kwargs):
                payload = (b"123\n" if "search" in argv else b"102\n" if "getwindowpid" in argv
                           else b"Original application\n" if "getwindowname" in argv
                           else WindowOwnershipTests.geometry() if "getwindowgeometry" in argv else b"display ready\n")
                return {"status": "PASS", "exit_code": 0, "stdout": payload, "stderr": b"",
                        "bytes_read": {"stdout": len(payload), "stderr": 0}, "prefix_truncated": False}
            children = [mock.Mock(pid=100 + index) for index in range(3)]
            for child in children:
                child.poll.return_value = None
            with mock.patch.object(SESSION, "Path", side_effect=path), \
                    mock.patch.dict(SESSION.os.environ, {"QTWEBENGINE_DISABLE_SANDBOX": "1"}), \
                    mock.patch.object(SESSION.resource, "setrlimit"), \
                    mock.patch.object(SESSION, "run_bounded", side_effect=command), \
                    mock.patch.object(SESSION, "cgroup_metrics", return_value={"memory.peak": "12", "memory.events": "oom_kill 0\n"}), \
                    mock.patch.object(SESSION, "process_metadata", return_value=[]), \
                    mock.patch.object(SESSION.subprocess, "Popen", side_effect=children) as launch, \
                    mock.patch.object(SESSION.os, "getpgid", return_value=102), \
                    mock.patch.object(SESSION.os, "killpg") as kill, \
                    mock.patch.object(SESSION.time, "monotonic", side_effect=now), \
                    mock.patch.object(SESSION.time, "sleep", side_effect=sleep), \
                    mock.patch.object(SESSION, "capture_display", side_effect=SESSION.CanaryError("capture failed")), \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(SESSION.main(), 1)
            report = json.loads((output / "session.json").read_text())
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["observed_window"]["id"], "123")
            self.assertEqual(report["observed_window"]["pid"], 102)
            self.assertEqual(report["observed_window"]["process_group"], 102)
            self.assertEqual(report["observed_window"]["document_identity"], "UNVERIFIED")
            self.assertEqual(report["vendor_passes"], 0)
            self.assertEqual(report["observed_supervisor_qtwebengine_disable_sandbox"], "1")
            self.assertEqual(report["loaded_qt_version"], "UNKNOWN")
            self.assertEqual(launch.call_args_list[2].args[0], ["/opt/cajviewer/bin/start.sh", "/input/digital.pdf"])
            self.assertEqual(kill.call_count, 3)
            self.assertTrue((output / "ready").is_file())
            self.assertIs(launch.call_args_list[0].kwargs["preexec_fn"], SESSION.runtime_file_limit)
            self.assertIsNone(launch.call_args_list[1].kwargs["preexec_fn"])
            self.assertIs(launch.call_args_list[2].kwargs["preexec_fn"], SESSION.runtime_file_limit)

    def test_failed_create_client_still_cleans_daemon_container_by_known_name(self):
        def result(status="PASS", code=0, stdout=b"", stderr=b""):
            return {"status": status, "exit_code": code, "stdout": stdout, "stderr": stderr,
                    "bytes_read": {"stdout": len(stdout), "stderr": len(stderr)}, "prefix_truncated": False}
        calls = []
        def helper(argv, **kwargs):
            calls.append(argv)
            if argv[1:3] == ["container", "inspect"]:
                return result("FAIL", 1, stdout=b"[]\n", stderr=("No such container: " + argv[-1]).encode())
            if argv[1] == "create":
                return result("TIMEOUT", -9)
            if argv[1] == "rm":
                return result(stdout=b"removed\n")
            return result()
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(DRIVER, "run_bounded", side_effect=helper):
            report = DRIVER.attempt("sha256:" + "0" * 64, Path(temporary),
                                    Path(temporary) / "attempt", "cajviewer-124-original-1")
        self.assertEqual((report["status"], report["cleanup"], report["app_launch_attempts"]), ("FAIL", "PASS", 0))
        self.assertIn(["docker", "rm", "--force", "cajviewer-124-original-1"], calls)
        self.assertFalse(any(call[1] == "start" for call in calls))

    def test_preexisting_container_is_never_removed(self):
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(DRIVER, "run_bounded", return_value={"status": "PASS", "exit_code": 0,
                    "stdout": b"[]", "stderr": b"", "bytes_read": {"stdout": 2, "stderr": 0}}) as helper:
            report = DRIVER.attempt("sha256:" + "0" * 64, Path(temporary), Path(temporary) / "attempt",
                                    "cajviewer-124-original-1")
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(helper.call_count, 1)
        self.assertEqual(report["cleanup"], "NOT_RUN")

    def test_logs_exception_never_skips_container_removal_or_final_audit(self):
        calls = []
        def helper(argv, **kwargs):
            calls.append(argv)
            if argv[1] == "logs":
                raise OSError("original simulated logs failure")
            absent = argv[1:3] == ["container", "inspect"]
            return {"status": "FAIL" if absent or argv[1] == "create" else "PASS",
                    "exit_code": 1 if absent else 7 if argv[1] == "create" else 0,
                    "stdout": b"[]\n" if absent else b"",
                    "stderr": ("No such container: " + argv[-1]).encode() if absent else b"",
                    "bytes_read": {"stdout": 0, "stderr": 0}, "prefix_truncated": False}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(DRIVER, "run_bounded", side_effect=helper):
            report = DRIVER.attempt("sha256:" + "0" * 64, Path(temporary), Path(temporary) / "attempt",
                                    "cajviewer-124-original-1")
        self.assertEqual((report["status"], report["cleanup"]), ("FAIL", "PASS"))
        self.assertEqual(calls[-2:], [["docker", "rm", "--force", "cajviewer-124-original-1"],
                                     ["docker", "container", "inspect", "cajviewer-124-original-1"]])

    def test_missing_truncated_and_mismatched_viewport_never_pass(self):
        session = {"protocol": "original-pdf-startup-v1", "status": "STARTUP_OBSERVED",
                   "input": "/input/digital.pdf", "app_launch_attempts": 1, "vendor_passes": 0, "cleanup": "PASS",
                   "before_metrics": {"memory.events": "oom_kill 0\n"},
                   "after_helper_termination_metrics": {"memory.events": "oom_kill 0\n"},
                   "diagnostic_capture": {"origin": "viewport-diagnostic-only", "complete_page": False,
                       "width": 1600, "height": 1200, "depth": 24, "bits_per_pixel": 32, "pixel_sha256": "0" * 64}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(FileNotFoundError):
                DRIVER.validate_session(session, root)
            path = root / "startup.ppm"
            path.write_bytes(b"P6\n1600 1200\n255\n\x00")
            with self.assertRaises(DRIVER.CanaryError):
                DRIVER.validate_session(session, root)
            session["after_helper_termination_metrics"]["memory.events"] = "oom_kill 1\n"
            with self.assertRaisesRegex(DRIVER.CanaryError, "OOM-killed"):
                DRIVER.validate_session(session, root)
            session["after_helper_termination_metrics"]["memory.events"] = "oom_kill 0\n"
            digest = hashlib.sha256()
            with path.open("wb") as file:
                file.write(b"P6\n1600 1200\n255\n")
                for _ in range(1200):
                    row = b"\xff" * 4800
                    file.write(row)
                    digest.update(row)
            with self.assertRaises(DRIVER.CanaryError):
                DRIVER.validate_session(session, root)
            session["diagnostic_capture"]["pixel_sha256"] = digest.hexdigest()
            self.assertEqual(DRIVER.validate_session(session, root)["vendor_passes"], 0)
            with path.open("ab") as file:
                file.write(b"tail")
            with self.assertRaises(DRIVER.CanaryError):
                DRIVER.validate_session(session, root)

    def test_no_input_driver_does_not_call_docker(self):
        with mock.patch.object(DRIVER, "run_bounded") as helper, \
                contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(DRIVER.main([]), 0)
        helper.assert_not_called()
        self.assertEqual(json.loads(output.getvalue()), {"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0})

    def test_scheduling_deadline_during_closing_does_not_skip_cleanup(self):
        calls = []
        def helper(argv, **kwargs):
            calls.append(argv)
            if argv[1] == "logs":
                DRIVER.SCHEDULING_DEADLINE = 0
            absent = argv[1:3] == ["container", "inspect"]
            return {"status": "FAIL" if absent or argv[1] == "create" else "PASS",
                    "exit_code": 1 if absent else 7 if argv[1] == "create" else 0,
                    "stdout": b"[]\n" if absent else b"",
                    "stderr": ("No such container: " + argv[-1]).encode() if absent else b"",
                    "bytes_read": {"stdout": 0, "stderr": 0}, "prefix_truncated": False}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(DRIVER, "SCHEDULING_DEADLINE", None), \
                mock.patch.object(DRIVER, "run_bounded", side_effect=helper):
            report = DRIVER.attempt("sha256:" + "0" * 64, Path(temporary), Path(temporary) / "attempt",
                                    "cajviewer-124-original-1")
        self.assertEqual((report["status"], report["cleanup"], report["phase_deadline_exceeded"]), ("FAIL", "PASS", True))
        self.assertEqual(calls[-2][1], "rm")
        self.assertEqual(calls[-1][1:3], ["container", "inspect"])


if __name__ == "__main__":
    unittest.main()
