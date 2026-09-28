# SPDX-License-Identifier: MIT
"""Public failure controls; no Docker daemon or vendor installer is needed."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import cajviewer_canary as CANARY
import cajviewer_canary_fixtures as CONTROLS


class CanaryFailureTests(unittest.TestCase):
    def test_no_input_is_zero_work_not_run_even_with_optional_environment(self):
        with mock.patch.dict(os.environ, {"CAJ2PDF_CAJVIEWER_INSTALLER": "/missing"}), \
                mock.patch.object(CANARY, "verify_installer") as verify, \
                mock.patch.object(subprocess, "Popen") as launch, \
                contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(CANARY.main(["--json"]), 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")
        self.assertEqual(json.loads(output.getvalue())["vendor_passes"], 0)
        verify.assert_not_called()
        launch.assert_not_called()

    def test_missing_explicit_installer_is_failure(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(CANARY.main(["--installer", "/missing-cajviewer", "--json"]), 1)
        result = json.loads(output.getvalue())
        self.assertEqual((result["status"], result["app_launches"], result["vendor_passes"]), ("FAIL", 0, 0))

    def test_wrong_size_hash_symlink_and_special_input_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "original.bin"
            path.write_bytes(b"abc")
            expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            self.assertEqual(CANARY.hash_regular(path, expected_size=3, expected_sha256=expected),
                             {"size_bytes": 3, "sha256": expected})
            for arguments in ({"expected_size": 4}, {"expected_sha256": "0" * 64}, {"max_bytes": 2}):
                with self.assertRaises(CANARY.CanaryError):
                    CANARY.hash_regular(path, **arguments)
            link = Path(temporary) / "link"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                CANARY.hash_regular(link)
            fifo = Path(temporary) / "fifo"
            os.mkfifo(fifo)
            with self.assertRaises(CANARY.CanaryError):
                CANARY.hash_regular(fifo)
            with mock.patch.object(CANARY, "INSTALLER_SIZE", 3), \
                    mock.patch.object(CANARY, "INSTALLER_SHA256", expected):
                self.assertEqual(CANARY.verify_installer(path)["sha256"], expected)
                path.write_bytes(b"abd")
                with self.assertRaises(CANARY.CanaryError):
                    CANARY.verify_installer(path)

    def test_failed_launch_and_nonzero_helper(self):
        with self.assertRaises(FileNotFoundError):
            CANARY.run_bounded(["/missing-canary-helper"], deadline_seconds=1)
        result = CANARY.run_bounded([sys.executable, "-c", "raise SystemExit(7)"], deadline_seconds=2)
        self.assertEqual((result["status"], result["exit_code"]), ("FAIL", 7))

    def test_stdout_and_stderr_are_limited_during_execution(self):
        for stream in ("stdout", "stderr"):
            result = CANARY.run_bounded([sys.executable, "-c",
                                        f"import sys; sys.{stream}.write('X'*1000000)"],
                                       deadline_seconds=3, output_limit=256)
            self.assertEqual(result["status"], "OUTPUT_LIMIT")
            self.assertEqual(len(result[stream]), 256)
            self.assertLess(result["elapsed_seconds"], 3)

    def test_timeout_terminates_descendants_that_hold_output_open(self):
        with tempfile.TemporaryDirectory() as temporary:
            pid_path = Path(temporary) / "child.pid"
            script = ("import subprocess,sys,time; "
                      "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
                      "open(sys.argv[1],'w').write(str(child.pid)); time.sleep(30)")
            result = CANARY.run_bounded([sys.executable, "-c", script, str(pid_path)],
                                       deadline_seconds=0.5)
            self.assertEqual(result["status"], "TIMEOUT")
            child = int(pid_path.read_text())
            status_path = Path("/proc") / str(child) / "status"
            deadline = time.monotonic() + 2
            while status_path.exists() and "State:\tZ" not in status_path.read_text():
                self.assertLess(time.monotonic(), deadline, "descendant is still running")
                time.sleep(0.01)

    def test_stale_missing_and_oversized_clipboard_never_pass(self):
        for owner, payload in ((10, b"old page"), (0, b"new page"), (11, b"sentinel"),
                               (11, None), (11, b"X" * 5)):
            with self.assertRaises(CANARY.CanaryError):
                CANARY.fresh_clipboard(sentinel_owner=10, observed_owner=owner,
                                       sentinel=b"sentinel", payload=payload, limit=4)
        self.assertEqual(CANARY.fresh_clipboard(sentinel_owner=10, observed_owner=11,
                                               sentinel=b"sentinel", payload=b""), b"")

    def test_partial_capture_and_malformed_grid_fail(self):
        pixels = CONTROLS.raster()
        width, height = CONTROLS.RASTER_WIDTH, CONTROLS.RASTER_HEIGHT
        result = CANARY.original_control_edges(pixels, width=width, height=height)
        self.assertEqual((result["status"], result["vendor_passes"]), ("PASS", 0))
        cropped = pixels[width * 3:]
        with self.assertRaises(CANARY.CanaryError):
            CANARY.original_control_edges(cropped, width=width, height=height - 1)
        with self.assertRaises(CANARY.CanaryError):
            CANARY.original_control_edges(pixels[:-1], width=width, height=height)
        with self.assertRaises(CANARY.CanaryError):
            CANARY.original_control_edges(pixels, width=True, height=height)


if __name__ == "__main__":
    unittest.main()
