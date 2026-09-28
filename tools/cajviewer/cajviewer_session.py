#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""One frozen, original-PDF startup attempt inside the offline container.

This initial protocol observes startup only. It never accepts a dialog, chooses
an undocumented switch, opens bundled documents, or claims a complete page.
"""

from __future__ import annotations

import ctypes
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import time

from cajviewer_canary import CanaryError, oom_kill_delta, run_bounded


APPLICATION_FILE_LIMIT = 64 * 1024 ** 2


def application_file_limit():
    """Set the single-threaded fork child's bounded limit before exec.

    Supervisor/query children keep the smaller soft limit. Writable mounts
    and diagnostic collection retain their independent aggregate/file caps.
    """
    resource.setrlimit(resource.RLIMIT_FSIZE, (APPLICATION_FILE_LIMIT,) * 2)


class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int),
               ("xoffset", ctypes.c_int), ("format", ctypes.c_int),
               ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
               ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
               ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
               ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int),
               ("red_mask", ctypes.c_ulong), ("green_mask", ctypes.c_ulong),
               ("blue_mask", ctypes.c_ulong)]


def capture_display(output: Path):
    library = ctypes.CDLL("libX11.so.6")
    library.XOpenDisplay.argtypes = [ctypes.c_char_p]
    library.XOpenDisplay.restype = ctypes.c_void_p
    library.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    library.XDefaultRootWindow.restype = ctypes.c_ulong
    library.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int,
                                ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                                ctypes.c_ulong, ctypes.c_int]
    library.XGetImage.restype = ctypes.POINTER(XImage)
    library.XDestroyImage.argtypes = [ctypes.POINTER(XImage)]
    library.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = library.XOpenDisplay(None)
    if not display:
        raise CanaryError("display unavailable for capture")
    image = None
    try:
        root = library.XDefaultRootWindow(display)
        image = library.XGetImage(display, root, 0, 0, 1600, 1200, ctypes.c_ulong(-1).value, 2)
        if not image:
            raise CanaryError("XGetImage failed")
        metadata = image.contents
        if (metadata.width, metadata.height, metadata.depth, metadata.bits_per_pixel,
            metadata.byte_order, metadata.red_mask, metadata.green_mask, metadata.blue_mask) != (
                1600, 1200, 24, 32, 0, 0xFF0000, 0xFF00, 0xFF):
            raise CanaryError("unrecognized measured RGB display layout")
        if metadata.bytes_per_line != 6400:
            raise CanaryError("unexpected measured display row stride")
        digest = hashlib.sha256()
        with output.open("xb") as file:
            header = b"P6\n1600 1200\n255\n"
            file.write(header)
            for y in range(1200):
                source = ctypes.string_at(metadata.data + y * metadata.bytes_per_line, 6400)
                row = bytearray(4800)
                row[0::3], row[1::3], row[2::3] = source[2::4], source[1::4], source[0::4]
                file.write(row)
                digest.update(row)
        return {"origin": "viewport-diagnostic-only", "complete_page": False,
                "width": 1600, "height": 1200, "pixel_sha256": digest.hexdigest(),
                "byte_order": "LSBFirst", "depth": 24, "bits_per_pixel": 32,
                "red_mask": 0xFF0000, "green_mask": 0xFF00, "blue_mask": 0xFF}
    finally:
        if image:
            library.XDestroyImage(image)
        library.XCloseDisplay(display)


def cgroup_metrics():
    result = {}
    for name in ("memory.current", "memory.peak", "memory.events", "pids.current", "pids.peak"):
        path = Path("/sys/fs/cgroup") / name
        result[name] = path.read_text()[:4096] if path.exists() else "UNAVAILABLE"
    return result


def process_metadata():
    result = []
    for path in sorted(Path("/proc").glob("[0-9]*/cmdline")):
        if len(result) >= 128:
            raise CanaryError("process metadata count exceeds PID budget")
        try:
            with path.open("rb") as file:
                argv = file.read(8193)
            if len(argv) > 8192:
                raise CanaryError("process argv over limit")
            if argv:
                with path.with_name("limits").open("rb") as file:
                    limits = file.read(8193)
                if len(limits) > 8192:
                    raise CanaryError("process limits metadata over limit")
                file_limit = next((line.decode("ascii", "strict") for line in limits.splitlines()
                                   if line.startswith(b"Max file size")), "UNAVAILABLE")
                result.append({"pid": int(path.parent.name),
                               "argv": argv.decode("utf-8", "replace").split("\0")[:-1],
                               "observed_file_size_limit": file_limit})
        except (FileNotFoundError, ProcessLookupError):
            pass
    return result


def main():
    started = time.monotonic()
    resource.setrlimit(resource.RLIMIT_FSIZE, (1024 ** 2, APPLICATION_FILE_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    # The supervisor's full diagnostic raster uses the larger capture
    # allowance; row-wise output retains its known 5.76 MiB ceiling.
    report = {"protocol": "original-pdf-startup-v1", "status": "FAIL",
              "app_launch_attempts": 0, "vendor_passes": 0,
              "input": "/input/digital.pdf", "image_compatibility": "NOT_RUN",
              "text_compatibility": "NOT_RUN", "vendor_dpr": "UNKNOWN",
              "loaded_qt_version": "UNKNOWN", "renderer_backend": "UNKNOWN",
              "declared_file_size_limits_bytes": {"supervisor_soft": 1024 ** 2,
                  "supervisor_hard": APPLICATION_FILE_LIMIT,
                  "application_soft_and_hard": APPLICATION_FILE_LIMIT,
                  "capture_soft": 6 * 1024 ** 2},
              "observed_supervisor_initial_file_size_limits_bytes": list(resource.getrlimit(resource.RLIMIT_FSIZE)),
              "observed_core_limits_bytes": list(resource.getrlimit(resource.RLIMIT_CORE)),
              "actions": [], "before_metrics": cgroup_metrics()}
    helpers = []
    helper_calls = 0
    application = None
    files = []
    def command(argv, **kwargs):
        nonlocal helper_calls
        helper_calls += 1
        if helper_calls > 400:
            raise CanaryError("controlled helper launch limit exceeded")
        action = {"action": "helper-command", "argv": argv}
        report["actions"].append(action)
        result = run_bounded(argv, **kwargs)
        action.update(status=result["status"], exit_code=result["exit_code"],
                      bytes_read=result["bytes_read"], prefix_truncated=result["prefix_truncated"])
        return result
    try:
        for name, argv in (("xvfb", ["Xvfb", ":99", "-screen", "0", "1600x1200x24",
                                     "-dpi", "96", "-nolisten", "tcp", "-noreset"]),
                           ("window-manager", ["openbox", "--sm-disable"])):
            log = (Path("/output") / (name + ".log")).open("xb")
            files.append(log)
            report["actions"].append({"action": "start-helper", "argv": argv})
            process = subprocess.Popen(argv, stdout=log, stderr=log, start_new_session=True)
            helpers.append(process)
            if name == "xvfb":
                deadline = time.monotonic() + 10
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise CanaryError("virtual display readiness deadline exceeded")
                    ready = command(["xdpyinfo"], deadline_seconds=min(2, remaining), output_limit=256 * 1024)
                    if ready["status"] == "PASS":
                        with Path("/output/xdpyinfo.txt").open("xb") as file:
                            file.write(ready["stdout"])
                        labels = ("dimensions:", "resolution:", "depth of root window:",
                                  "vendor string:", "vendor release number:", "number of screens:")
                        summary = [line for line in ready["stdout"].decode("utf-8", "strict").splitlines()
                                   if any(label in line for label in labels)]
                        report["display_metadata"] = {"size_bytes": len(ready["stdout"]),
                                                      "sha256": hashlib.sha256(ready["stdout"]).hexdigest(),
                                                      "summary": summary[:16]}
                        break
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise CanaryError("virtual display failed its measured readiness check")
                    time.sleep(0.1)
        argv = ["/opt/cajviewer/bin/start.sh", "/input/digital.pdf"]
        log = (Path("/output") / "application.log").open("xb")
        files.append(log)
        report["app_launch_attempts"] = 1
        report["actions"].append({"action": "official-desktop-launcher", "argv": argv,
                                  "file_size_limit_bytes": APPLICATION_FILE_LIMIT})
        application = subprocess.Popen(argv, stdout=log, stderr=log, start_new_session=True,
                                       preexec_fn=application_file_limit)
        deadline = time.monotonic() + 30
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                report["startup_reason"] = "matching-window-deadline"
                break
            observed = command(["xdotool", "search", "--onlyvisible", "--name", "digital[.]pdf"],
                               deadline_seconds=min(2, remaining), output_limit=4096)
            windows = observed["stdout"].decode("ascii", "strict").split()
            if observed["status"] == "PASS" and len(windows) == 1 and windows[0].isdigit():
                name = command(["xdotool", "getwindowname", windows[0]],
                               deadline_seconds=2, output_limit=4096)
                geometry = command(["xdotool", "getwindowgeometry", "--shell", windows[0]],
                                   deadline_seconds=2, output_limit=4096)
                if name["status"] != "PASS" or geometry["status"] != "PASS":
                    raise CanaryError("window measurement failed")
                report["observed_window"] = {"id": windows[0],
                                             "title": name["stdout"].decode("utf-8", "replace"),
                                             "geometry": geometry["stdout"].decode("ascii")}
                # Window title is startup evidence only, not page/render readiness.
                report["status"] = "STARTUP_OBSERVED"
                break
            if time.monotonic() >= deadline:
                report["startup_reason"] = "launcher-exited-or-matching-window-deadline"
                break
            time.sleep(0.1)
        report["processes_sampled_before_termination"] = process_metadata()
        report["launcher_exit_code_observed"] = application.poll()
        resource.setrlimit(resource.RLIMIT_FSIZE, (6 * 1024 ** 2, APPLICATION_FILE_LIMIT))
        report["diagnostic_capture"] = capture_display(Path("/output/startup.ppm"))
    except (OSError, CanaryError, UnicodeError) as error:
        report["status"] = "FAIL"
        report["error_type"] = type(error).__name__
    finally:
        report["cleanup"] = "PASS"
        def final_measurement(key, function):
            try:
                report[key] = function()
            except (OSError, CanaryError) as error:
                report["status"] = "FAIL"
                report.setdefault("finalization_errors", []).append({"stage": key, "error_type": type(error).__name__})
        final_measurement("before_termination_metrics", cgroup_metrics)
        for process in ([application] if application is not None else []) + helpers:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except OSError as error:
                report["cleanup"] = "FAIL"
                report["status"] = "FAIL"
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                report["cleanup"] = "FAIL"
                report["status"] = "FAIL"
        for file in files:
            file.close()
        final_measurement("after_helper_termination_metrics", cgroup_metrics)
        try:
            report["oom_kill_delta"] = oom_kill_delta(report["before_metrics"], report["after_helper_termination_metrics"])
            if report["oom_kill_delta"]:
                report["status"] = "FAIL"
        except (KeyError, CanaryError) as error:
            report["status"] = "FAIL"
            report["memory_audit_error_type"] = type(error).__name__
        final_measurement("processes_sampled_after_termination", process_metadata)
        report["elapsed_seconds"] = time.monotonic() - started
        report["controlled_helper_launches"] = helper_calls
        with Path("/output/session.json").open("x") as file:
            json.dump(report, file, indent=2)
            file.write("\n")
        with Path("/output/ready").open("x") as file:
            file.write("session-receipt-complete\n")
    print(json.dumps({key: report[key] for key in ("status", "app_launch_attempts", "vendor_passes")}), flush=True)
    # Keep bounded tmpfs artifacts mounted until the host collects them. This is
    # a collection lease, not application readiness; the app is already killed.
    lease = time.monotonic() + 60
    while time.monotonic() < lease:
        time.sleep(min(0.25, lease - time.monotonic()))
    return 0 if report["status"] == "STARTUP_OBSERVED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
