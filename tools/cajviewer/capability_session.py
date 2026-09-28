#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in live original-PDF capability session; the startup entry is unchanged."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import struct
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))

from capability_clipboard import Clipboard
from capability_io import ALLOWLIST, Meter, directory, exclusive, read_file
from capability_pages import PageGate, verify_page_frame
from capability_protocol import (CAPS, COLLECT_WAIT_RESERVE, DISPLAY, PAGE_KEYS, RECEIPT_LIMIT, Ledger,
                                 PublicationFailure, Refusal, discovery_outcome, exact_value, require, strict_json, validate_observation, validate_profile)
from capability_x11 import X11
from cajviewer_canary import oom_kill_delta, run_bounded
from cajviewer_session import process_metadata
import cajviewer_canary_fixtures as fixtures


def encoded(value):
    return (json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":")) + "\n").encode("ascii")


def receipt_bytes(report):
    payload = encoded(report)
    if len(payload) <= RECEIPT_LIMIT:
        return payload
    # Receipt refusal preserves known admissions and the primary error, and
    # explicitly omits raw observations/event detail rather than truncating it.
    return encoded({"protocol": report["protocol"], "status": "FAIL", "receipt_complete": False,
                    "reason": "receipt-size-limit", "first_failure": report.get("first_failure"),
                    "admitted": report.get("admitted"), "closing_errors": report.get("closing_errors"),
                    "cleanup": report.get("cleanup"), "application_launches": report.get("application_launches"),
                    "counts": report.get("counts"), "vendor_passes": 0, "omitted_bytes": len(payload)})


class Desktop:
    """Finite actual xdotool dispatch and raw X11 observations.

    Frozen coordinates and keys must come from reviewed finite GUI discovery.
    An owned normal window, class and raw document/page controls are all
    mandatory; a matching title alone never passes the gate.
    """

    def __init__(self, profile, ledger, process_group, wire, command=run_bounded):
        self.profile, self.ledger, self.process_group = profile, ledger, process_group
        self.wire, self.command = wire, command
        self.observed = None
        self.serial = 0
        self.raw = []
        self.discovery_raw = []

    def helper(self, argv, *, stage, deadline_seconds=2, output_limit=4096, allow_absent=False):
        def run():
            result = self.command(argv, deadline_seconds=min(deadline_seconds, max(0.001, self.ledger.deadline - self.ledger.clock())),
                                  output_limit=output_limit, env=self.profile["environment"])
            absent = allow_absent and result["status"] == "FAIL" and exact_value(result["exit_code"], 1) and not result["stdout"]
            require((absent or result["status"] == "PASS" and exact_value(result["exit_code"], 0)) and not result["stderr"]
                    and result["prefix_truncated"] is False and exact_value(result["bytes_read"]["stdout"], len(result["stdout"])),
                    "desktop-helper-incomplete")
            return result
        return self.ledger.perform("query", stage, run)

    def owned(self, target="main"):
        if target == "main" and self.observed is not None:
            ids = [str(self.observed[0]).encode("ascii")]
        else:
            result = self.helper(["xdotool", "search", "--onlyvisible", "--maxdepth", "2", "--limit", "17", "--name", ".*"],
                                 stage="visible-owned-window-query", allow_absent=True)
            ids = result["stdout"].splitlines()
        require(len(ids) <= 16 and len(set(ids)) == len(ids) and
                all(value.isdigit() and 0 < int(value) < 2 ** 32 for value in ids), "window-candidate-limit-or-format")
        matches = []
        for value in ids:
            window = int(value)
            if not self.wire.viewable(window):
                continue
            _, bits, pid_raw = self.wire.property(window, self.wire.atom("_NET_WM_PID"), limit=4)
            if bits != 32 or len(pid_raw) != 4:
                continue
            pid = struct.unpack("<I", pid_raw)[0]
            try:
                if not pid or os.getpgid(pid) != self.process_group:
                    continue
            except (ProcessLookupError, PermissionError):
                continue
            _, bits, types = self.wire.property(window, self.wire.atom("_NET_WM_WINDOW_TYPE"), limit=64)
            expected_type = self.wire.atom("_NET_WM_WINDOW_TYPE_NORMAL" if target == "main" else "_NET_WM_WINDOW_TYPE_DIALOG")
            if bits != 32 or types != struct.pack("<I", expected_type):
                continue
            _, bits, cls = self.wire.property(window, self.wire.atom("WM_CLASS"))
            expected_class = self.profile["bindings"]["window_class"] if target == "main" else self.profile["bindings"]["open_dialog"]["class"]
            if bits != 8 or cls.decode("utf-8", "strict").rstrip("\0") != expected_class:
                continue
            if target == "open-dialog":
                _, bits, role = self.wire.property(window, self.wire.atom("WM_WINDOW_ROLE"))
                require(bits == 8 and role.decode("utf-8", "strict") == self.profile["bindings"]["open_dialog"]["role"],
                        "unapproved-dialog-role")
            matches.append((window, pid))
        require(len(matches) == 1, "owned-normal-or-open-dialog-unavailable")
        window, pid = matches[0]
        if target == "open-dialog":
            marker = self.profile["bindings"]["open_dialog"]["marker"]
            validate_observation(self.binding_at(window, marker), marker)
            return window, pid
        if self.observed is not None:
            require((window, pid) == self.observed, "owned-window-changed")
        self.observed = (window, pid)
        return window, pid

    def owner_allowed(self, owner):
        _, bits, data = self.wire.property(owner, self.wire.atom("_NET_WM_PID"), limit=4)
        if bits != 32 or len(data) != 4:
            return False
        pid = struct.unpack("<I", data)[0]
        try:
            return pid > 0 and os.getpgid(pid) == self.process_group
        except (ProcessLookupError, PermissionError):
            return False

    def read_binding(self, binding):
        window, _ = self.owned()
        payload = self.binding_at(window, binding)
        self.owned()
        return payload

    def binding_at(self, window, binding):
        if binding["kind"] == "property":
            _, bits, payload = self.wire.property(window, self.wire.atom(binding["atom"]), limit=4096)
            require(bits == 8, "desktop-property-not-text")
        else:
            payload = self.wire.image(binding["rect"])
        return payload

    def observe(self, bindings):
        owner = self.owned()
        result = {key: self.read_binding(binding) for key, binding in bindings.items()}
        require(self.owned() == owner, "observation-owner-raced")
        self.serial += 1
        result.update(serial=self.serial, owner=owner)
        self.raw.append({"serial": self.serial, "owner": list(owner), "raw": {
            key: {"encoding": "base64", "data": base64.b64encode(value).decode("ascii")}
            for key, value in result.items() if type(value) is bytes}})
        require(len(encoded(self.raw)) <= 512 * 1024, "raw-observation-limit")
        return result

    def dispatch(self, actions, *, discovery=False):
        for action in actions:
            window, _ = self.owned(action["target"])
            def perform():
                # Activate only the observed owned normal window. A modal
                # prompt cannot be acknowledged through a generic key branch.
                self.helper(["xdotool", "windowactivate", "--sync", str(window)], stage="focus-owned-window")
                _, bits, active = self.wire.property(self.wire.root, self.wire.atom("_NET_ACTIVE_WINDOW"), limit=4)
                require(bits == 32 and active == struct.pack("<I", window), "focus-not-owned-target")
                kind, value = action["kind"], action["value"]
                if kind == "key":
                    argv = ["xdotool", "key", "--clearmodifiers", value]
                elif kind == "type-path":
                    argv = ["xdotool", "type", "--clearmodifiers", "--", value]
                else:
                    x, y, width, height = self.wire.geometry(window)
                    require(x <= value[0] < x + width and y <= value[1] < y + height, "pointer-outside-owned-target")
                    argv = ["xdotool", "mousemove", "--sync", str(value[0]), str(value[1])]
                    if kind == "click":
                        argv += ["click", "1"]
                self.helper(argv, stage="gui-" + kind)
            self.ledger.perform("discovery" if discovery else "gui", "gui-" + action["kind"], perform)

    def discover(self, steps):
        for index, step in enumerate(steps):
            self.dispatch(step["actions"], discovery=True)
            owner = self.owned()
            raw = self.read_binding(step["binding"])
            require(self.owned() == owner, "discovery-owner-raced")
            outcome = discovery_outcome(raw, step)
            self.discovery_raw.append({"step": index, "owner": list(owner), "label": outcome["label"],
                                       "raw_base64": base64.b64encode(raw).decode("ascii")})
            self.dispatch(outcome["actions"], discovery=True)

    def capture(self):
        owner = self.owned()
        pixels = self.wire.image([0, 0, *DISPLAY])
        require(self.owned() == owner, "capture-owner-raced")
        return b"P6\n1600 1200\n255\n" + pixels

    def page_area(self, area):
        window, _ = self.owned()
        x, y, width, height = self.wire.geometry(window)
        left, top, page_width, page_height = area
        require(x <= left - 1 and y <= top - 1 and left + page_width + 1 <= x + width
                and top + page_height + 1 <= y + height, "physical-page-outside-owned-window")


class Workflow:
    """The exact same five pages and two copy transactions in each session."""

    def __init__(self, profile, ledger, desktop, clipboard, output, meter, fixture_module=fixtures):
        self.profile, self.ledger, self.desktop, self.clipboard = profile, ledger, desktop, clipboard
        self.output, self.meter, self.fixtures = Path(output), meter, fixture_module
        self.gate = PageGate(profile)
        self.pages, self.copies = [], []
        self.counts = {"pages_attempted": 0, "pages_completed": 0, "pages_failed": 0,
                       "pages_unverified": 0, "pages_remaining": 5,
                       "copy_attempted": 0, "copy_completed": 0, "copy_failed": 0,
                       "copy_unverified": 0, "copy_remaining": 2}

    def run(self, session_index):
        self.desktop.discover(self.profile["actions"]["discover"])
        self.gate.opened("digital.pdf", self.desktop.observe(self.profile["bindings"]["documents"]["digital.pdf"]))
        for key in PAGE_KEYS:
            if key == "image-only-1":
                self.desktop.dispatch(self.profile["actions"]["open-image"])
                self.gate.opened("image-only.pdf", self.desktop.observe(self.profile["bindings"]["documents"]["image-only.pdf"]))
            self.counts["pages_attempted"] += 1
            self.counts["pages_remaining"] -= 1
            try:
                self.desktop.dispatch(self.profile["actions"]["navigate"][key])
                self.desktop.dispatch(self.profile["actions"]["fit"])
                identity = self.gate.page(key, self.desktop.observe(self.profile["bindings"]["pages"][key]))
                self.desktop.page_area(self.profile["pages"][key]["rect"])
                raw = self.ledger.perform("capture", key, self.desktop.capture)
                # Preserve a complete raw failed observation too. No sixth
                # frame, retry, arbitrary crop, or rescale is admitted.
                exclusive(self.output / (key + ".ppm"), raw, self.meter)
                page = verify_page_frame(raw, key, self.profile["pages"][key], self.fixtures)
                page["identity"] = identity
                self.pages.append(page)
                self.counts["pages_completed"] += 1
            except (Exception, KeyboardInterrupt) as error:
                self.counts["pages_failed" if isinstance(error, Refusal) else "pages_unverified"] += 1
                raise
            if key in ("digital-1", "image-only-1"):
                self.counts["copy_attempted"] += 1
                self.counts["copy_remaining"] -= 1
                try:
                    payload, evidence = self.clipboard.copy(str(session_index) + ":" + key,
                        lambda: self.desktop.dispatch(self.profile["actions"]["copy"]))
                    exclusive(self.output / (key + ".clipboard"), payload, self.meter)
                    evidence["page_key"] = key
                    evidence["negative_control"] = "COMPLETE_EMPTY" if key == "image-only-1" and not payload else (
                        "TEXT_PRESENT_ORIGIN_UNVERIFIED" if key == "image-only-1" else "NOT_APPLICABLE")
                    self.copies.append(evidence)
                    self.counts["copy_completed"] += 1
                except (Exception, KeyboardInterrupt) as error:
                    self.counts["copy_failed" if isinstance(error, Refusal) else "copy_unverified"] += 1
                    raise
        return {"pages": self.pages, "copies": self.copies, "counts": self.counts}


def child_limits():
    resource.setrlimit(resource.RLIMIT_FSIZE, (CAPS["application_file_bytes"],) * 2)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


class LogPump:
    """Bound each live pipe before retention, leaving receipt space available."""

    def __init__(self, process, path, limit, on_failure=None):
        self.process, self.path, self.limit = process, Path(path), limit
        self.requested = self.calls = self.returned = self.retained = 0
        self.error = None
        self.complete = False
        self.on_failure = on_failure
        self.thread = threading.Thread(target=self._drain, name="capability-log", daemon=True)
        self.thread.start()

    def _drain(self):
        try:
            with self.path.open("xb") as stream:
                while True:
                    count = min(65536, self.limit + 1 - self.returned)
                    self.requested += count
                    self.calls += 1
                    data = self.process.stdout.read(count)
                    if not data:
                        self.complete = True
                        break
                    self.returned += len(data)
                    require(self.returned <= self.limit, "live-log-size-limit")
                    Meter().write(stream, data)
                    self.retained += len(data)
        except (Exception, KeyboardInterrupt) as error:
            self.error = str(error) if isinstance(error, Refusal) else "live-log-result-unavailable"
            if self.on_failure is not None:
                self.on_failure(error)
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def close(self):
        self.thread.join(timeout=5)
        require(not self.thread.is_alive() and self.error is None and self.complete, "live-log-incomplete")
        self.process.stdout.close()

    def summary(self):
        return {"requested_bytes": self.requested, "read_calls": self.calls,
                "returned_bytes": self.returned, "retained_bytes": self.retained,
                "complete": self.complete, "error": self.error,
                "hash_scope": "complete-stream" if self.complete and self.error is None else "retained-prefix"}


def process_identity(pid):
    try:
        with open("/proc/" + str(pid) + "/stat", "rb") as file:
            raw = file.read(4097)
        require(len(raw) <= 4096, "process-stat-size-limit")
        end = raw.rfind(b")")
        values = raw[end + 2:].split()
        require(end > 0 and len(values) >= 20 and values[19].isdigit(), "process-stat-malformed")
        return int(values[19])
    except (FileNotFoundError, ProcessLookupError):
        return None


def process_identities():
    """Bounded identities for the private container, including reparented PIDs."""
    names = [name for name in os.listdir("/proc") if name.isdigit()]
    require(len(names) <= CAPS["pids"], "process-snapshot-limit")
    result = {}
    for name in names:
        born = process_identity(int(name))
        if born is not None:
            result[int(name)] = born
    return result


def close_owned_tree(baseline):
    """No process outside the fresh container is inspected or signalled."""
    for _ in range(3):
        current = process_identities()
        require(all(pid not in current or current[pid] == born for pid, born in baseline.items()), "baseline-process-identity-changed")
        extra = {pid: born for pid, born in current.items() if pid not in baseline}
        if not extra:
            return {"status": "PASS", "scope": "owned-process-tree-excluding-baseline-lease", "remaining": 0}
        for pid, born in extra.items():
            # Repeat identity immediately before signalling to avoid PID reuse.
            if process_identity(pid) == born:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        time.sleep(0.05)
    raise Refusal("owned-process-tree-not-proved-closed")


def collector_baseline(meter, seconds):
    """Prove the original collection lease before any display/app admission."""
    until = time.monotonic() + 10
    while time.monotonic() < until:
        try:
            payload, _ = read_file("/runtime/collector-admission.json", meter, limit=1024)
            record = strict_json(payload, 1024)
            require(set(record) == {"protocol", "pid", "birth", "uid_gid", "seconds"}
                    and record["protocol"] == "original-capability-collector/1"
                    and type(record["pid"]) is int and record["pid"] > 1
                    and type(record["birth"]) is int and record["birth"] > 0
                    and exact_value(record["uid_gid"], [1000, 1000]) and exact_value(record["seconds"], seconds + COLLECT_WAIT_RESERVE),
                    "collector-admission-mismatch")
            snapshot = process_identities()
            require(snapshot.get(record["pid"]) == record["birth"], "collector-identity-mismatch")
            with open("/proc/" + str(record["pid"]) + "/cmdline", "rb") as file:
                argv = file.read(4097)
            require(argv == ("/usr/bin/python3\0-B\0/opt/capability/capability_collect.py\0--wait-seconds\0" + str(seconds + COLLECT_WAIT_RESERVE) + "\0").encode(),
                    "collector-not-original-lease-command")
            return snapshot
        except FileNotFoundError:
            time.sleep(0.1)
    raise Refusal("collector-admission-deadline")


def cgroup_value(name, meter):
    # Kernel files can advertise size zero. Read these fixed observations to
    # EOF under a 4096-byte ceiling instead of treating stat size as content.
    parent = directory("/sys/fs/cgroup")
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    finally:
        os.close(parent)
    data = bytearray()
    with os.fdopen(fd, "rb") as stream:
        while True:
            part = meter.read(stream, min(1024, 4097 - len(data)))
            if not part:
                break
            data.extend(part)
            require(len(data) <= 4096, "cgroup-observation-limit")
    return bytes(data).decode("ascii", "strict")


def cgroup_snapshot(meter):
    return {name: cgroup_value(name, meter) for name in
            ("memory.current", "memory.peak", "memory.events", "pids.current", "pids.peak")}


def session_limits(meter):
    """Read only the four fixed, bounded cgroup cap files; never probe tools."""
    values = {}
    for name in ("memory.max", "memory.swap.max", "pids.max", "cpu.max"):
        values[name] = cgroup_value(name, meter).split()
    require(all(len(values[key]) == 1 and values[key][0].isdigit()
                for key in ("memory.max", "memory.swap.max", "pids.max"))
            and len(values["cpu.max"]) == 2 and all(part.isdigit() for part in values["cpu.max"]), "unavailable-session-caps")
    quota, period = map(int, values["cpu.max"])
    require(period > 0 and quota == 2 * period and int(values["memory.max"][0]) == CAPS["memory_bytes"]
            and int(values["memory.swap.max"][0]) == 0 and int(values["pids.max"][0]) == CAPS["pids"], "session-observed-caps-mismatch")
    require(resource.getrlimit(resource.RLIMIT_FSIZE) == (CAPS["application_file_bytes"],) * 2
            and resource.getrlimit(resource.RLIMIT_CORE) == (0, 0), "session-file-core-limits-mismatch")
    return {"status": "PASS", "memory_max_bytes": int(values["memory.max"][0]), "memory_swap_max_bytes": 0,
            "pids_max": int(values["pids.max"][0]), "cpu_quota": quota, "cpu_period": period,
            "file_limits": list(resource.getrlimit(resource.RLIMIT_FSIZE)), "core_limits": [0, 0]}


def session_environment(profile):
    raw = encoded(dict(os.environ))
    require(dict(os.environ) == profile["environment"] and os.getuid() == os.getgid() == 1000, "session-environment-or-user-mismatch")
    return {"status": "PASS", "uid_gid": [1000, 1000], "identity": {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}}


def run_session(profile, output, session_index, *, process_factory=subprocess.Popen, wire_factory=X11):
    ledger, meter = Ledger(profile["seconds"]), Meter()
    report = {"protocol": profile["protocol"], "status": "FAIL", "receipt_complete": True,
              "application_launches": 0, "vendor_passes": 0, "image_compatibility": "NOT_RUN",
              "text_compatibility": "NOT_RUN", "cleanup": "UNKNOWN", "session_index": session_index,
              "active_build": "UNAVAILABLE", "loaded_qt_version": "UNAVAILABLE",
              "renderer_backend": "UNAVAILABLE", "vendor_dpr": "UNAVAILABLE",
              "settings_declared": profile["settings"], "pages": [], "copies": [],
              "counts": {"pages_attempted": 0, "pages_completed": 0, "pages_failed": 0, "pages_unverified": 0,
                         "pages_remaining": 5, "copy_attempted": 0, "copy_completed": 0, "copy_failed": 0,
                         "copy_unverified": 0, "copy_remaining": 2}}
    processes, logs, wire, workflow = [], [], None, None
    output = Path(output)
    stage, baseline = "initial-audit", None
    try:
        require(dict(os.environ) == profile["environment"], "session-environment-mismatch")
        require(os.getuid() == 1000 and os.getgid() == 1000, "session-user-mismatch")
        require(set(output.iterdir()) == set(), "session-output-not-empty")
        report["environment_before"] = session_environment(profile)
        report["limits_before"] = session_limits(meter)
        report["before_metrics"] = cgroup_snapshot(meter)
        baseline = collector_baseline(meter, profile["seconds"])
        for name, argv in (("xvfb", ["Xvfb", ":99", "-screen", "0", "1600x1200x24", "-dpi", "96", "-nolisten", "tcp", "-noreset"]),
                           ("window-manager", ["openbox", "--sm-disable"])):
            stage = name + "-start"
            process = ledger.perform("persistent-helper", stage,
                lambda argv=argv: process_factory(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=profile["environment"],
                                                         start_new_session=True, preexec_fn=child_limits))
            processes.append(process)
            logs.append(ledger.perform("log-drainer", name + "-log", lambda process=process, name=name:
                LogPump(process, output / (name + ".log"), ALLOWLIST[name + ".log"],
                        on_failure=lambda error: ledger.fail("live-log-" + name, error))))
            if name == "xvfb":
                ready = False
                for _ in range(20):
                    result = ledger.perform("query", "display-readiness", lambda: run_bounded(
                        ["xdpyinfo"], deadline_seconds=0.5, output_limit=65536, env=profile["environment"]))
                    if result["status"] == "PASS" and result["exit_code"] == 0 and not result["stderr"]:
                        ready = True
                        break
                    require(result["status"] == "FAIL" and not result["prefix_truncated"], "display-readiness-incomplete")
                require(ready, "display-readiness-unavailable")
        stage = "display-connection"
        wire = ledger.perform("query", stage, lambda: wire_factory(
            admit=lambda opcode: ledger.admit("query", "x11-" + str(opcode)), deadline=ledger.deadline))
        exclusive(output / "display.json", encoded(wire.display), meter)
        stage = "official-launcher"
        # Admission consumes the sole launch slot even if Popen escapes.
        def launch():
            report["application_launches"] = 1
            return process_factory(["/opt/cajviewer/bin/start.sh", "/input/digital.pdf"], stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, env=profile["environment"], start_new_session=True, preexec_fn=child_limits)
        process = ledger.perform("launcher", stage, launch)
        processes.append(process)
        logs.append(ledger.perform("log-drainer", "application-log", lambda:
            LogPump(process, output / "application.log", ALLOWLIST["application.log"],
                    on_failure=lambda error: ledger.fail("live-log-application", error))))
        desktop = Desktop(profile, ledger, process.pid, wire)
        ready = False
        for _ in range(12):
            try:
                desktop.owned()
                ready = True
                break
            except Refusal as error:
                require(str(error) == "owned-normal-or-open-dialog-unavailable", "window-readiness-failed")
                time.sleep(0.1)
        require(ready, "owned-window-readiness-limit")
        clipboard = Clipboard(wire, ledger, profile["clipboard"], owner_allowed=desktop.owner_allowed)
        workflow = Workflow(profile, ledger, desktop, clipboard, output, meter)
        stage = "capability-workflow"
        report.update(workflow.run(session_index))
        # Observed About bytes remain separate from inferred toolkit/backend.
        if profile["bindings"]["about"] is not None:
            desktop.dispatch(profile["actions"]["about"])
            raw = desktop.read_binding(profile["bindings"]["about"])
            report["about"] = validate_observation(raw, profile["bindings"]["about"])
        else:
            report["about"] = {"status": "UNAVAILABLE", "reason": "no-reviewed-about-binding"}
        exclusive(output / "observations.json", encoded({"workflow": desktop.raw, "discovery": desktop.discovery_raw}), meter)
        report["status"] = "CAPABILITIES_OBSERVED_ACTIVE_SETTINGS_UNVERIFIED"
    except (Exception, KeyboardInterrupt) as error:
        ledger.fail(stage, error)
    finally:
        if workflow is not None:
            report.update(pages=workflow.pages, copies=workflow.copies, counts=workflow.counts)
        for index, process in enumerate(reversed(processes)):
            def stop(process=process):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=5)
            try:
                ledger.perform("closing", "reap-group-" + str(index), stop)
            except (Exception, KeyboardInterrupt):
                continue
        for log in logs:
            try:
                ledger.perform("closing", "close-log", log.close)
            except (Exception, KeyboardInterrupt):
                continue
        if wire is not None:
            report["x11_accounting"] = wire.accounting()
            try:
                ledger.perform("closing", "close-display-connection", wire.close)
            except (Exception, KeyboardInterrupt):
                pass
        if baseline is not None:
            try:
                report["process_cleanup"] = ledger.perform("closing", "owned-tree-final-absence",
                    lambda: close_owned_tree(baseline))
            except (Exception, KeyboardInterrupt):
                report["process_cleanup"] = {"status": "UNKNOWN_OR_FAIL", "remaining": None}
        for name, function in (("after_metrics", lambda: cgroup_snapshot(meter)), ("processes_after_termination", process_metadata),
                               ("environment_after", lambda: session_environment(profile)), ("limits_after", lambda: session_limits(meter))):
            try:
                report[name] = ledger.perform("closing", name, function)
            except (Exception, KeyboardInterrupt):
                report[name] = "UNAVAILABLE"
        try:
            require(exact_value(report["environment_before"], report["environment_after"])
                    and exact_value(report["limits_before"], report["limits_after"]), "session-closing-identity-mismatch")
            for key, cap in (("memory.peak", CAPS["memory_bytes"]), ("pids.peak", CAPS["pids"])):
                value = report["after_metrics"][key].strip()
                require(value.isdigit() and int(value) <= cap, "session-measured-peak-unavailable-or-over-cap")
        except (Exception, KeyboardInterrupt) as error:
            ledger.fail("session-cap-environment-closing", error, closing=True)
        try:
            report["oom_kill_delta"] = oom_kill_delta(report["before_metrics"], report["after_metrics"])
            require(report["oom_kill_delta"] == 0, "oom-kill-observed")
        except (Exception, KeyboardInterrupt) as error:
            ledger.fail("oom-audit", error, closing=True)
        report.update(ledger.summary())
        report["logs"] = [log.summary() for log in logs]
        report["cleanup"] = "AWAITING_HOST_CONTAINER_REMOVAL" if not ledger.closing_errors else "FAIL"
        report["elapsed_seconds"] = ledger.clock() - ledger.started
        report["file_io"] = meter.summary()
        if ledger.first_failure is not None:
            report["status"] = "FAIL"
        # Closing publication is attempted even after primary/cleanup failures.
        # Unknown publication is caught by the host; it never means zero starts.
        try:
            exclusive(output / "session.json", receipt_bytes(report), meter)
            exclusive(output / "ready", b"capability-receipt-complete\n", meter)
        except (Exception, KeyboardInterrupt) as error:
            ledger.fail("session-terminal-publication", error, closing=True)
            report.update(ledger.summary())
            report.update(status="FAIL", file_io=meter.summary())
            raise PublicationFailure(report) from None
    return report


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print(encoded({"status": "NOT_RUN", "application_launches": 0, "vendor_passes": 0}).decode(), end="")
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--approval", type=Path)
    parser.add_argument("--session", type=int, choices=(1, 2))
    args = parser.parse_args(argv)
    if args.profile is None and args.approval is None and args.session is None:
        print(encoded({"status": "NOT_RUN", "application_launches": 0, "vendor_passes": 0}).decode(), end="")
        return 0
    try:
        require(args.profile is not None and args.approval is not None and args.session is not None, "incomplete-session-admission")
        meter = Meter()
        payload, pin = read_file(args.profile, meter, limit=RECEIPT_LIMIT)
        approval, _ = read_file(args.approval, meter, limit=65536)
        a = strict_json(approval)
        require(exact_value(a, {"status": "APPROVED", "profile": pin, "sessions": 2}), "session-not-approved")
        profile = validate_profile(payload)
        report = run_session(profile, Path("/output"), args.session)
        print(encoded({"status": report["status"], "application_launches": report["application_launches"], "vendor_passes": 0}).decode(), end="")
        # Owned container lease only; all app/display groups are already reaped.
        until = time.monotonic() + 60
        while time.monotonic() < until:
            time.sleep(min(0.25, until - time.monotonic()))
        return int(report["status"] == "FAIL")
    except (Exception, KeyboardInterrupt) as error:
        retained = error.report if isinstance(error, PublicationFailure) else {}
        print(encoded({"status": "REFUSED_OR_FAIL", "application_launches": retained.get("application_launches"),
                       "admitted": retained.get("admitted"), "counts": retained.get("counts"),
                       "first_failure": retained.get("first_failure"), "closing_errors": retained.get("closing_errors", []),
                       "vendor_passes": 0, "reason": str(error) if isinstance(error, Refusal) else "result-unavailable"}).decode(), end="")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
