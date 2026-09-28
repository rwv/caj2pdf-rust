#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""One frozen, original-PDF startup attempt inside the offline container.

This initial protocol observes startup only. It never accepts a dialog, chooses
an undocumented switch, opens bundled documents, or claims a complete page.
"""

from __future__ import annotations

import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import signal
import subprocess
import time

from cajviewer_canary import CanaryError, oom_kill_delta, run_bounded


RUNTIME_FILE_LIMIT = 64 * 1024 ** 2
MAX_WINDOW_CANDIDATES = 16
MAX_HELPER_CALLS = 400
QUERY_BYTES = 4096
SESSION_BYTES = 256 * 1024


class SessionFailure(CanaryError):
    """A fixed diagnostic location, never an arbitrary exception message."""

    def __init__(self, stage, reason, result=None, action_index=None):
        super().__init__(stage + ": " + reason.replace("-", " "))
        self.stage, self.reason = stage, reason
        self.result = result
        self.action_index = action_index if result is None else result.get("action_index")


def helper_capture(result):
    """Hash retained bytes; aborted helpers never claim a complete stream."""
    if (result["status"] not in ("PASS", "FAIL", "TIMEOUT", "OUTPUT_LIMIT")
            or type(result["exit_code"]) is not int or type(result["prefix_truncated"]) is not bool):
        raise SessionFailure("helper-result", "malformed-capture")
    complete = result["status"] in ("PASS", "FAIL") and not result["prefix_truncated"]
    capture = {}
    for stream in ("stdout", "stderr"):
        payload = result[stream]
        read_bytes = result["bytes_read"][stream]
        if (not isinstance(payload, bytes) or type(read_bytes) is not int
                or read_bytes < len(payload)):
            raise SessionFailure("helper-result", "malformed-capture")
        stream_complete = complete and read_bytes == len(payload)
        capture[stream] = {"captured_bytes": len(payload),
                           "sha256": hashlib.sha256(payload).hexdigest(),
                           "complete": stream_complete,
                           "truncated": read_bytes > len(payload) or result["prefix_truncated"],
                           "hash_scope": "complete-stream" if stream_complete else "captured-prefix"}
    return capture


def terminal_failure(error, stage):
    diagnostic = {"stage": error.stage if isinstance(error, SessionFailure) else stage,
                  "reason": error.reason if isinstance(error, SessionFailure) else "operation-failed",
                  "action_index": error.action_index if isinstance(error, SessionFailure) else None}
    if isinstance(error, SessionFailure) and error.result is not None:
        result = error.result
        diagnostic["helper"] = {"status": result["status"], "exit_code": result["exit_code"],
                                "bytes_read": result["bytes_read"], "capture": helper_capture(result)}
        if "argv" in result:
            diagnostic["helper"]["argv"] = result["argv"]
        # Helper stdout is not embedded in this diagnostic. Measured window
        # observations retain their separate declared scope in this receipt.
        stderr = result["stderr"][:QUERY_BYTES]
        diagnostic["stderr"] = {"encoding": "base64", "data": base64.b64encode(stderr).decode("ascii"),
                                "retained_bytes": len(stderr), "sha256": hashlib.sha256(stderr).hexdigest(),
                                "complete": diagnostic["helper"]["capture"]["stderr"]["complete"]
                                            and len(stderr) == len(result["stderr"]),
                                "truncated": len(stderr) < len(result["stderr"])
                                             or diagnostic["helper"]["capture"]["stderr"]["truncated"]}
    elif diagnostic["action_index"] is not None:
        diagnostic["helper"] = {"status": "UNAVAILABLE", "exit_code": None,
                                "bytes_read": None, "capture": None}
    return diagnostic


def session_payload(report):
    """Serialize completely or emit an explicit small FAIL refusal receipt."""
    def encode(value):
        return (json.dumps(value, ensure_ascii=True, separators=(",", ":")) + "\n").encode("ascii")
    report.setdefault("receipt_complete", True)
    payload = encode(report)
    if len(payload) <= SESSION_BYTES:
        return payload
    # The host still receives the primary failure and mandatory cleanup/OOM
    # evidence. The omitted ledger is declared, not silently truncated.
    refusal = {key: report[key] for key in ("protocol", "input", "app_launch_attempts",
               "vendor_passes", "cleanup", "controlled_helper_launches", "elapsed_seconds")}
    refusal.update(status="FAIL", receipt_complete=False,
                   receipt_refusal={"reason": "serialized-size-limit", "limit_bytes": SESSION_BYTES,
                                    "unserialized_size_bytes": len(payload), "ledger_omitted": True,
                                    "actions_omitted": len(report["actions"])})
    refusal["terminal_failure"] = {"stage": "receipt", "reason": "serialized-size-limit", "action_index": None}
    for key in ("terminal_failure", "error_type", "oom_kill_delta", "memory_audit_error_type",
                "finalization_errors"):
        if key in report:
            refusal[key] = report[key]
    for key in ("before_metrics", "after_helper_termination_metrics"):
        refusal[key] = {"memory.events": report.get(key, {}).get("memory.events", "UNAVAILABLE")}
    payload = encode(refusal)
    if len(payload) > SESSION_BYTES:
        raise CanaryError("bounded refusal receipt exceeds session limit")
    report.clear()
    report.update(refusal)
    return payload


def observe_owned_window(command, process_group: int, deadline: float):
    """Observe startup only using visible X11 IDs and their reported owner.

    xdotool's documented getwindowpid reads _NET_WM_PID, which some clients
    omit. An absent/stale/foreign owner cannot prove startup. Titles never
    identify the requested document, and no window is activated or changed.
    """
    def query(argv, stage):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SessionFailure(stage, "observation-deadline")
        result = command(argv, stage=stage, deadline_seconds=min(2, remaining), output_limit=QUERY_BYTES)
        if (result["status"] not in ("PASS", "FAIL") or result["prefix_truncated"]
                or len(result["stdout"]) > 4096 or len(result["stderr"]) > 4096):
            reason = {"TIMEOUT": "timeout", "OUTPUT_LIMIT": "output-limit"}.get(result["status"], "malformed-helper-result")
            raise SessionFailure(stage, reason, result)
        if time.monotonic() >= deadline:
            raise SessionFailure(stage, "observation-deadline", result)
        if result["status"] == "PASS" and (result["exit_code"] != 0 or result["stderr"]):
            raise SessionFailure(stage, "inconsistent-success", result)
        if result["status"] == "FAIL" and result["exit_code"] != 1:
            raise SessionFailure(stage, "unexpected-exit", result)
        if result["status"] == "FAIL" and stage != "window-owner" and result["stderr"]:
            raise SessionFailure(stage, "helper-failed", result)
        return result

    def decimal_id(data, maximum):
        if not data or len(data) > 10 or not data.isdigit():
            return None
        number = int(data)
        return number if 0 < number <= maximum else None

    def owner(window):
        result = query(["xdotool", "getwindowpid", window], "window-owner")
        if result["status"] != "PASS":
            return None
        pid = decimal_id(result["stdout"].strip(), 2 ** 31 - 1)
        if pid is None:
            return None
        try:
            group = os.getpgid(pid)
        except (ProcessLookupError, PermissionError):
            return None
        except OSError:
            raise SessionFailure("window-owner", "owner-check-failed", result) from None
        return pid if group == process_group else None

    result = query(["xdotool", "search", "--onlyvisible", "--maxdepth", "2",
                    "--limit", str(MAX_WINDOW_CANDIDATES + 1), "--name", ".*"], "visible-window-search")
    if result["status"] != "PASS":
        if result["exit_code"] == 1 and not result["stdout"] and not result["stderr"]:
            return None
        raise SessionFailure("visible-window-search", "helper-failed", result)
    windows = result["stdout"].splitlines()
    if len(windows) > MAX_WINDOW_CANDIDATES:
        raise SessionFailure("visible-window-search", "candidate-count-limit", result)
    identifiers = [decimal_id(window, 2 ** 32 - 1) for window in windows]
    if None in identifiers or len(set(identifiers)) != len(windows):
        raise SessionFailure("visible-window-search", "malformed-window-ids", result)
    for raw_window in windows:
        window = raw_window.decode("ascii", "strict")
        pid = owner(window)
        if pid is None:
            continue
        name = query(["xdotool", "getwindowname", window], "window-title")
        if name["status"] != "PASS":
            continue
        try:
            title = name["stdout"].decode("utf-8", "strict")
        except UnicodeError:
            raise SessionFailure("window-title", "malformed-title", name) from None
        if "\0" in title:
            raise SessionFailure("window-title", "malformed-title", name)
        geometry = query(["xdotool", "getwindowgeometry", "--shell", window], "window-geometry")
        if geometry["status"] != "PASS":
            continue
        try:
            geometry_text = geometry["stdout"].decode("ascii", "strict")
        except UnicodeError:
            raise SessionFailure("window-geometry", "malformed-geometry", geometry) from None
        fields = {}
        for line in geometry_text.splitlines():
            key, separator, value = line.partition("=")
            if (not separator or key in fields or len(value) > 11
                    or re.fullmatch(r"-?[0-9]+", value) is None):
                raise SessionFailure("window-geometry", "malformed-geometry", geometry)
            fields[key] = int(value)
        if (set(fields) != {"WINDOW", "X", "Y", "WIDTH", "HEIGHT", "SCREEN"}
                or fields["WINDOW"] != int(window) or fields["SCREEN"] != 0
                or not 0 < fields["WIDTH"] <= 65535 or not 0 < fields["HEIGHT"] <= 65535
                or not -(2 ** 31) <= fields["X"] < 2 ** 31
                or not -(2 ** 31) <= fields["Y"] < 2 ** 31):
            raise SessionFailure("window-geometry", "geometry-outside-profile", geometry)
        if owner(window) != pid:
            continue
        return {"id": window, "pid": pid, "process_group": process_group,
                "scope": "startup-owned-visible-window-only", "document_identity": "UNVERIFIED",
                "title": title, "geometry": geometry_text}
    return None


def runtime_file_limit():
    """Set the single-threaded application/display child's limit before exec.

    Supervisor/query children keep the smaller soft limit. Writable mounts
    and diagnostic collection retain their independent aggregate/file caps.
    """
    resource.setrlimit(resource.RLIMIT_FSIZE, (RUNTIME_FILE_LIMIT,) * 2)


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
    resource.setrlimit(resource.RLIMIT_FSIZE, (1024 ** 2, RUNTIME_FILE_LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    # The supervisor's full diagnostic raster uses the larger capture
    # allowance; row-wise output retains its known 5.76 MiB ceiling.
    report = {"protocol": "original-pdf-startup-v1", "status": "FAIL",
              "app_launch_attempts": 0, "vendor_passes": 0,
              "input": "/input/digital.pdf", "image_compatibility": "NOT_RUN",
              "text_compatibility": "NOT_RUN", "vendor_dpr": "UNKNOWN",
              "loaded_qt_version": "UNKNOWN", "renderer_backend": "UNKNOWN",
              "observed_supervisor_qtwebengine_disable_sandbox": os.environ.get("QTWEBENGINE_DISABLE_SANDBOX"),
              "declared_file_size_limits_bytes": {"supervisor_soft": 1024 ** 2,
                  "supervisor_hard": RUNTIME_FILE_LIMIT,
                  "application_soft_and_hard": RUNTIME_FILE_LIMIT,
                  "xvfb_soft_and_hard": RUNTIME_FILE_LIMIT,
                  "capture_soft": 6 * 1024 ** 2},
              "observed_supervisor_initial_file_size_limits_bytes": list(resource.getrlimit(resource.RLIMIT_FSIZE)),
              "observed_core_limits_bytes": list(resource.getrlimit(resource.RLIMIT_CORE)),
              "actions": [], "before_metrics": cgroup_metrics()}
    helpers = []
    helper_calls = 0
    application = None
    files = []
    stage = "display-start"
    def command(argv, *, stage, **kwargs):
        nonlocal helper_calls
        if helper_calls >= MAX_HELPER_CALLS:
            raise SessionFailure(stage, "helper-call-limit")
        helper_calls += 1
        action = {"action": "helper-command", "stage": stage, "argv": argv}
        report["actions"].append(action)
        action_index = len(report["actions"])
        try:
            result = run_bounded(argv, **kwargs)
        except (Exception, KeyboardInterrupt):
            action.update(status="FAIL", reason="helper-result-unavailable")
            raise SessionFailure(stage, "helper-result-unavailable", action_index=action_index) from None
        try:
            result["action_index"] = action_index
            result["argv"] = argv
            action.update(status=result["status"], exit_code=result["exit_code"],
                          bytes_read=result["bytes_read"], prefix_truncated=result["prefix_truncated"],
                          capture=helper_capture(result))
        except (KeyError, TypeError, SessionFailure):
            action.update(status="FAIL", reason="malformed-helper-result")
            raise SessionFailure(stage, "malformed-helper-result", action_index=action_index) from None
        return result
    try:
        for name, argv in (("xvfb", ["Xvfb", ":99", "-screen", "0", "1600x1200x24",
                                     "-dpi", "96", "-nolisten", "tcp", "-noreset"]),
                           ("window-manager", ["openbox", "--sm-disable"])):
            stage = "display-start" if name == "xvfb" else "window-manager-start"
            log = (Path("/output") / (name + ".log")).open("xb")
            files.append(log)
            report["actions"].append({"action": "start-helper", "argv": argv,
                "file_size_limit_bytes": RUNTIME_FILE_LIMIT if name == "xvfb" else 1024 ** 2})
            process = subprocess.Popen(argv, stdout=log, stderr=log, start_new_session=True,
                                       preexec_fn=runtime_file_limit if name == "xvfb" else None)
            helpers.append(process)
            if name == "xvfb":
                stage = "display-readiness"
                deadline = time.monotonic() + 10
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise SessionFailure(stage, "readiness-deadline")
                    ready = command(["xdpyinfo"], stage=stage, deadline_seconds=min(2, remaining), output_limit=256 * 1024)
                    if ready["status"] in ("TIMEOUT", "OUTPUT_LIMIT"):
                        raise SessionFailure(stage, "timeout" if ready["status"] == "TIMEOUT" else "output-limit", ready)
                    if ((ready["status"] == "PASS" and (ready["exit_code"] != 0 or ready["stderr"]))
                            or (ready["status"] == "FAIL" and ready["exit_code"] != 1)
                            or ready["status"] not in ("PASS", "FAIL")):
                        raise SessionFailure(stage, "unexpected-readiness-result", ready)
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
                        raise SessionFailure(stage, "readiness-failed", ready)
                    time.sleep(0.1)
        stage = "application-start"
        argv = ["/opt/cajviewer/bin/start.sh", "/input/digital.pdf"]
        log = (Path("/output") / "application.log").open("xb")
        files.append(log)
        report["app_launch_attempts"] = 1
        report["actions"].append({"action": "official-desktop-launcher", "argv": argv,
                                  "file_size_limit_bytes": RUNTIME_FILE_LIMIT})
        application = subprocess.Popen(argv, stdout=log, stderr=log, start_new_session=True,
                                       preexec_fn=runtime_file_limit)
        deadline = time.monotonic() + 30
        stage = "owned-window-observation"
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                report["startup_reason"] = "owned-visible-window-deadline"
                report["terminal_failure"] = terminal_failure(SessionFailure(stage, "observation-deadline"), stage)
                break
            observed = observe_owned_window(command, application.pid, deadline)
            if observed is not None:
                report["observed_window"] = observed
                report["status"] = "STARTUP_OBSERVED"
                break
            if time.monotonic() >= deadline:
                report["startup_reason"] = "launcher-exited-or-owned-visible-window-deadline"
                report["terminal_failure"] = terminal_failure(SessionFailure(stage, "observation-deadline"), stage)
                break
            time.sleep(0.1)
        stage = "process-snapshot"
        report["processes_sampled_before_termination"] = process_metadata()
        report["launcher_exit_code_observed"] = application.poll()
        resource.setrlimit(resource.RLIMIT_FSIZE, (6 * 1024 ** 2, RUNTIME_FILE_LIMIT))
        stage = "viewport-capture"
        report["diagnostic_capture"] = capture_display(Path("/output/startup.ppm"))
    except (Exception, KeyboardInterrupt) as error:
        report["status"] = "FAIL"
        report["error_type"] = type(error).__name__
        report.setdefault("terminal_failure", terminal_failure(error, stage))
    finally:
        report["cleanup"] = "PASS"
        def final_measurement(key, function):
            try:
                report[key] = function()
            except (Exception, KeyboardInterrupt) as error:
                report["status"] = "FAIL"
                report.setdefault("finalization_errors", []).append({"stage": key, "error_type": type(error).__name__})
                report.setdefault("terminal_failure", {"stage": "final-measurement", "reason": "measurement-failed", "action_index": None})
        final_measurement("before_termination_metrics", cgroup_metrics)
        for process in ([application] if application is not None else []) + helpers:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except (OSError, KeyboardInterrupt):
                report["cleanup"] = "FAIL"
                report["status"] = "FAIL"
                report.setdefault("terminal_failure", {"stage": "cleanup", "reason": "process-group-kill-failed", "action_index": None})
            try:
                process.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired, KeyboardInterrupt):
                report["cleanup"] = "FAIL"
                report["status"] = "FAIL"
                report.setdefault("terminal_failure", {"stage": "cleanup", "reason": "process-reap-failed", "action_index": None})
        for file in files:
            try:
                file.close()
            except (OSError, KeyboardInterrupt):
                report["cleanup"] = "FAIL"
                report["status"] = "FAIL"
                report.setdefault("terminal_failure", {"stage": "cleanup", "reason": "log-close-failed", "action_index": None})
        final_measurement("after_helper_termination_metrics", cgroup_metrics)
        try:
            report["oom_kill_delta"] = oom_kill_delta(report["before_metrics"], report["after_helper_termination_metrics"])
            if report["oom_kill_delta"]:
                report["status"] = "FAIL"
                report.setdefault("terminal_failure", {"stage": "memory-audit", "reason": "oom-kill-observed", "action_index": None})
        except (KeyError, CanaryError) as error:
            report["status"] = "FAIL"
            report["memory_audit_error_type"] = type(error).__name__
            report.setdefault("terminal_failure", {"stage": "memory-audit", "reason": "memory-audit-unavailable", "action_index": None})
        final_measurement("processes_sampled_after_termination", process_metadata)
        report["elapsed_seconds"] = time.monotonic() - started
        report["controlled_helper_launches"] = helper_calls
        payload = session_payload(report)
        with Path("/output/session.json").open("xb") as file:
            file.write(payload)
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
