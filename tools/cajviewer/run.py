#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run exactly two frozen original-PDF startup attempts, explicitly opt-in.

The image must already exist locally. This command never builds, pulls, retries
another environment, or opens a corpus. All receipts stay in a new external
directory. This is not the private image/text acquisition implementation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time
import types

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from cajviewer_canary import CanaryError, hash_regular, oom_kill_delta, read_pinned_metadata, run_bounded, verify_viewport_capture
import cajviewer_canary_fixtures as controls_recipe

SOURCE_FILES = {"scripts/cajviewer_canary.py", "scripts/cajviewer_canary_fixtures.py",
                "tools/cajviewer/run.py", "tools/cajviewer/cajviewer_session.py",
                "tools/cajviewer/prepare.py", "tools/cajviewer/inventory.py", "tools/cajviewer/Dockerfile"}
CONTROL_FILES = {"digital.pdf", "alternate-unicode.pdf", "image-only.pdf", "second-text.pdf", "controls.json"}
SCHEDULING_DEADLINE = None

# BEGIN PUBLIC SOURCE COMMON
# Frozen inline/host adapters embed these bytes, without another installed module.
PUBLIC_SOURCE_PINS = (
    ("cajviewer_canary", "process-helper", "/opt/canary/cajviewer_canary.py", 10992,
     "ba9abc3be6285cfb7d0af3840b88197fd5aa4ec22d7277ecee109ea0eae60c77"),
    ("inventory", "runtime-inventory", "/opt/canary/inventory.py", 2656,
     "cb1cec2a74be10413ce628cf5320180c1d2d950212f59b5750bbad434a24d556"),
)
PUBLIC_SOURCE_STAGES = ("read", "pin", "compile", "exec")
PUBLIC_SOURCE_REASONS = {"read": "PUBLIC_SOURCE_READ_FAILED", "pin": "PUBLIC_SOURCE_PIN_FAILED",
                         "compile": "PUBLIC_SOURCE_COMPILE_FAILED", "exec": "PUBLIC_SOURCE_EXEC_FAILED"}
PUBLIC_SOURCE_ERROR_TYPES = frozenset({
    "FileNotFoundError", "PermissionError", "IsADirectoryError", "NotADirectoryError", "OSError",
    "ValueError", "TypeError", "KeyError", "AttributeError", "SyntaxError", "UnicodeError",
    "UnicodeDecodeError", "ImportError", "ModuleNotFoundError", "MemoryError", "RuntimeError",
    "OverflowError", "KeyboardInterrupt", "SystemExit", "OTHER_ERROR_TYPE",
})


class SourceLoadError(ValueError):
    """Fixed safe source-accounting refusal; no exception text is retained."""


def _source_hex(value):
    return type(value) is str and len(value) == 64 and all(character in "0123456789abcdef" for character in value)


def _source_pins(pins):
    if type(pins) is not tuple or len(pins) != 2:
        raise SourceLoadError("source-load-pins")
    for pin, fixed in zip(pins, PUBLIC_SOURCE_PINS):
        if (type(pin) is not tuple or len(pin) != 5 or pin[:3] != fixed[:3]
                or any(type(value) is not str for value in pin[:3])
                or type(pin[3]) is not int or not 0 < pin[3] <= 65536 or not _source_hex(pin[4])):
            raise SourceLoadError("source-load-pins")
    return pins
# END PUBLIC SOURCE COMMON

# BEGIN PUBLIC SOURCE LOADER
def load_public_sources(read, records, *, pins=PUBLIC_SOURCE_PINS, module_registry=None):
    """Load two pinned MIT modules; stop and re-raise the first failure.

    The reader must return complete bounded regular-file bytes through EOF,
    or raise. See cajviewer-source-loading.md for the safe record contract.
    """
    pins = _source_pins(pins)
    if type(records) is not list or records or module_registry is not None and type(module_registry) is not dict:
        raise SourceLoadError("source-load-initial-state")
    registry = sys.modules if module_registry is None else module_registry
    modules = {}
    for ordinal, (name, role, path, size, sha) in enumerate(pins, 1):
        row = {"ordinal": ordinal, "role": role, "path": path,
               "expected_identity": {"size_bytes": size, "sha256": sha}, "actual_identity": None,
               "stages": {"read": "PENDING", "pin": "NOT_RUN", "compile": "NOT_RUN", "exec": "NOT_RUN"},
               "status": "PENDING", "failure_stage": None, "reason": None, "error_type": None}
        records.append(row)
        stage = "read"
        try:
            raw = read(path, 65536)
            if type(raw) is not bytes or len(raw) > 65536:
                raise SourceLoadError("source-load-read-contract")
            row["actual_identity"] = {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
            row["stages"][stage] = "PASS"
            stage = "pin"
            row["stages"][stage] = "PENDING"
            if row["actual_identity"] != row["expected_identity"]:
                raise ValueError("source-load-pin")
            row["stages"][stage] = "PASS"
            stage = "compile"
            row["stages"][stage] = "PENDING"
            code = compile(raw, path, "exec")
            row["stages"][stage] = "PASS"
            stage = "exec"
            row["stages"][stage] = "PENDING"
            module = types.ModuleType(name)
            module.__file__ = path
            registry[name] = module
            exec(code, module.__dict__)
            row["stages"][stage] = "PASS"
            row["status"] = "PASS"
            modules[name] = module
        except BaseException as error:
            kind = type(error).__name__
            if stage == "read":
                row["actual_identity"] = None
            row.update(status="FAIL", failure_stage=stage, reason=PUBLIC_SOURCE_REASONS[stage],
                       error_type=kind if kind in PUBLIC_SOURCE_ERROR_TYPES else "OTHER_ERROR_TYPE")
            row["stages"][stage] = "FAIL"
            raise
    return modules
# END PUBLIC SOURCE LOADER

# BEGIN PUBLIC SOURCE VALIDATOR
def validate_source_loads(records, *, pins=PUBLIC_SOURCE_PINS):
    """Validate independent safe accounting, including a complete FAIL ledger.

    This does not approve the outer child, inventory, environment or caps. The
    caller must parse bounded duplicate-safe complete metadata first. Exact pin
    equality proves byte identity only, not application or document behavior.
    """
    pins = _source_pins(pins)
    if type(records) is not list or len(records) > 2:
        raise SourceLoadError("source-load-accounting")
    safe = []
    keys = {"ordinal", "role", "path", "expected_identity", "actual_identity", "stages",
            "status", "failure_stage", "reason", "error_type"}
    for ordinal, row in enumerate(records, 1):
        _, role, path, size, sha = pins[ordinal - 1]
        if (type(row) is not dict or set(row) != keys or type(row["ordinal"]) is not int
                or row["ordinal"] != ordinal or type(row["role"]) is not str or row["role"] != role
                or type(row["path"]) is not str or row["path"] != path
                or type(row["expected_identity"]) is not dict
                or set(row["expected_identity"]) != {"size_bytes", "sha256"}
                or type(row["expected_identity"]["size_bytes"]) is not int
                or not _source_hex(row["expected_identity"]["sha256"])
                or row["expected_identity"] != {"size_bytes": size, "sha256": sha}
                or type(row["stages"]) is not dict or set(row["stages"]) != set(PUBLIC_SOURCE_STAGES)
                or ordinal > 1 and safe[-1]["status"] != "PASS"):
            raise SourceLoadError("source-load-accounting")
        actual = row["actual_identity"]
        if actual is not None and (type(actual) is not dict or set(actual) != {"size_bytes", "sha256"}
                or type(actual["size_bytes"]) is not int or not 0 <= actual["size_bytes"] <= 65536
                or not _source_hex(actual["sha256"])):
            raise SourceLoadError("source-load-accounting")
        states = [row["stages"][name] for name in PUBLIC_SOURCE_STAGES]
        if (states[0] == "NOT_RUN" or any(type(value) is not str
                or value not in ("NOT_RUN", "PENDING", "PASS", "FAIL") for value in states)):
            raise SourceLoadError("source-load-accounting")
        first = next((index for index, value in enumerate(states) if value != "PASS"), 4)
        if first < 4 and any(value != "NOT_RUN" for value in states[first + 1:]):
            raise SourceLoadError("source-load-accounting")
        if states[0] == "PASS" and actual is None or states[1] == "PASS" and actual != row["expected_identity"]:
            raise SourceLoadError("source-load-accounting")
        if states[0] != "PASS" and actual is not None:
            raise SourceLoadError("source-load-accounting")
        if first == 4:
            expected = ("PASS", None, None, None)
        elif states[first] == "FAIL":
            stage = PUBLIC_SOURCE_STAGES[first]
            if type(row["error_type"]) is not str or row["error_type"] not in PUBLIC_SOURCE_ERROR_TYPES:
                raise SourceLoadError("source-load-accounting")
            expected = ("FAIL", stage, PUBLIC_SOURCE_REASONS[stage], row["error_type"])
        else:
            expected = ("PENDING", None, None, None)
        if (type(row["status"]) is not str
                or (row["status"], row["failure_stage"], row["reason"], row["error_type"]) != expected):
            raise SourceLoadError("source-load-accounting")
        safe.append({**row, "expected_identity": dict(row["expected_identity"]),
                     "actual_identity": None if actual is None else dict(actual), "stages": dict(row["stages"])})
    return {"observation": "COMPLETE_ORDERED_RECORDS", "declared_sources": 2,
            "records_attempted": len(safe), "records_completed": sum(row["status"] == "PASS" for row in safe),
            "records_failed": sum(row["status"] == "FAIL" for row in safe), "records_remaining": 2 - len(safe),
            "stages": {name: {"attempted": sum(row["stages"][name] != "NOT_RUN" for row in safe),
                              "completed": sum(row["stages"][name] == "PASS" for row in safe),
                              "failed": sum(row["stages"][name] == "FAIL" for row in safe)} for name in PUBLIC_SOURCE_STAGES},
            "records": safe}


def source_loading_observation(envelope, *, complete, pins=PUBLIC_SOURCE_PINS):
    """Retain known source stages before an outer FAIL refusal.

    Prefix captures or invalid records remain UNKNOWN and cannot yield a
    successful source count. No arbitrary envelope fields enter the result.
    """
    unknown = {"observation": "UNKNOWN", "declared_sources": 2, "records": None,
               "reason": "SOURCE_CAPTURE_INCOMPLETE"}
    if complete is not True:
        return unknown
    if (type(envelope) is not dict or type(envelope.get("status")) is not str
            or envelope["status"] not in ("PASS", "FAIL")):
        return {**unknown, "reason": "SOURCE_ACCOUNTING_MALFORMED"}
    try:
        observed = validate_source_loads(envelope.get("source_loads"), pins=pins)
        if envelope["status"] == "PASS" and observed["records_completed"] != 2:
            return {**unknown, "reason": "SOURCE_ACCOUNTING_MALFORMED"}
        return observed
    except SourceLoadError:
        return {**unknown, "reason": "SOURCE_ACCOUNTING_MALFORMED"}
# END PUBLIC SOURCE VALIDATOR


def public_source_fragment(source, part):
    """Extract only a named block from a separately hash-pinned runner source.

    The source must be a complete bounded read. This function does not read,
    import or execute it; the phase must audit the whole source independently.
    """
    if type(source) is not bytes or len(source) > 65536 or part not in ("LOADER", "VALIDATOR"):
        raise SourceLoadError("source-load-fragment")
    fragments = []
    for name in ("COMMON", part):
        start = ("# BEGIN PUBLIC SOURCE " + name + "\n").encode("ascii")
        end = ("# END PUBLIC SOURCE " + name + "\n").encode("ascii")
        if source.count(start) != 1 or source.count(end) != 1:
            raise SourceLoadError("source-load-fragment")
        left, right = source.index(start), source.index(end)
        if right <= left or fragments and left <= source.index(b"# END PUBLIC SOURCE COMMON\n"):
            raise SourceLoadError("source-load-fragment")
        fragments.append(source[left:right + len(end)])
    return b"\n".join(fragments)

# Docker cp cannot read this tmpfs mount. Run an original, finite archive
# transport in the container's mount namespace using its pinned Python tool.
# The sole directory is held open; only seven known regular files are eligible.
COLLECT_PROGRAM = """import os, stat, sys, tarfile
allowed = {'session.json', 'ready', 'startup.ppm', 'application.log',
           'xvfb.log', 'window-manager.log', 'xdpyinfo.txt'}
root = os.open(sys.argv[1], os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
try:
    with tarfile.open(fileobj=sys.stdout.buffer, mode='w|') as archive:
        directory = tarfile.TarInfo('output')
        directory.type, directory.mode = tarfile.DIRTYPE, 0o700
        archive.addfile(directory)
        total = count = 0
        with os.scandir(root) as entries:
            for entry in entries:
                count += 1
                if count > 7 or entry.name not in allowed:
                    raise RuntimeError('unexpected diagnostic file')
                descriptor = os.open(entry.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=root)
                with os.fdopen(descriptor, 'rb') as file:
                    before = os.fstat(file.fileno())
                    total += before.st_size
                    if not stat.S_ISREG(before.st_mode) or before.st_size > 6 * 1024**2 or total > 32 * 1024**2:
                        raise RuntimeError('diagnostic file contract exceeded')
                    item = tarfile.TarInfo('output/' + entry.name)
                    item.size, item.mode = before.st_size, 0o400
                    archive.addfile(item, file)
                    after = os.fstat(file.fileno())
                    identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
                    if identity(before) != identity(after):
                        raise RuntimeError('diagnostic file changed during transport')
finally:
    os.close(root)
"""


class PhaseDeadline(CanaryError):
    """Stop further app scheduling; retain independent closing budgets."""


def check_phase():
    if SCHEDULING_DEADLINE is not None and time.monotonic() >= SCHEDULING_DEADLINE:
        raise PhaseDeadline("global startup scheduling deadline exceeded")


def command(argv, *, deadline=10, output_limit=65536, stdout_sink=None):
    check_phase()
    result = run_bounded(argv, deadline_seconds=deadline, output_limit=output_limit,
                         stdout_sink=stdout_sink)
    if result["status"] != "PASS":
        raise CanaryError(f"helper {result['status']}")
    return result


def audit_files(records, root):
    for record in records:
        path = Path(record["path"])
        if path.is_absolute() or ".." in path.parts:
            raise CanaryError("unsafe pinned relative path")
        hash_regular(root / path, expected_size=record["size_bytes"],
                     expected_sha256=record["sha256"])


def extract_output(archive_path: Path, output: Path):
    output.mkdir(mode=0o700)
    total = count = 0
    with tarfile.open(archive_path, mode="r:") as archive:
        for member in archive:
            count += 1
            total += member.size
            parts = Path(member.name).parts
            if (count > 32 or total > 32 * 1024 ** 2 or member.size > 6 * 1024 ** 2
                    or member.name.startswith("/") or ".." in parts
                    or not parts or parts[0] != "output"
                    or not (member.isfile() or member.isdir())):
                raise CanaryError("container output archive exceeds the declared contract")
            archive.extract(member, path=output, filter="data")
    return {"members": count, "size_bytes": total}


def proven_absent(result, name):
    return (result is not None and result["status"] == "FAIL" and result["exit_code"] == 1
            and result["stdout"].strip() == b"[]"
            and ("No such container: " + name).encode("ascii") in result["stderr"])


def validate_session(session, capture_dir):
    if (not isinstance(session, dict)
            or session.get("protocol") != "original-pdf-startup-v1" or session.get("input") != "/input/digital.pdf"
            or type(session.get("app_launch_attempts")) is not int or session["app_launch_attempts"] not in (0, 1)
            or session.get("vendor_passes") != 0 or session.get("cleanup") != "PASS"
            or session.get("status") not in ("FAIL", "STARTUP_OBSERVED")
            or (session["status"] == "STARTUP_OBSERVED" and session["app_launch_attempts"] != 1)):
        raise CanaryError("invalid original startup session contract")
    if (not isinstance(session.get("before_metrics"), dict)
            or not isinstance(session.get("after_helper_termination_metrics"), dict)):
        raise CanaryError("session cgroup audit missing or malformed")
    if oom_kill_delta(session["before_metrics"], session["after_helper_termination_metrics"]):
        raise CanaryError("application cgroup observed an OOM-killed process")
    if session.get("receipt_complete", True) is not True:
        return {"status": "NOT_RUN", "scope": "viewport-artifact-integrity-only",
                "reason": "session-receipt-refused", "vendor_passes": 0}
    if "diagnostic_capture" not in session:
        return {"status": "NOT_RUN", "scope": "viewport-artifact-integrity-only",
                "reason": "capture-missing", "vendor_passes": 0}
    capture = session["diagnostic_capture"]
    if (not isinstance(capture, dict) or capture.get("origin") != "viewport-diagnostic-only"
            or capture.get("complete_page") is not False
            or tuple(capture.get(key) for key in ("width", "height", "depth", "bits_per_pixel")) != (1600, 1200, 24, 32)):
        raise CanaryError("diagnostic capture contract mismatch")
    return {"status": "PASS", **verify_viewport_capture(capture_dir / "startup.ppm", pixel_sha256=capture["pixel_sha256"])}


def attempt(image: str, controls: Path, directory: Path, name: str) -> dict:
    started = time.monotonic()
    directory.mkdir(mode=0o700)
    report = {"name": name, "status": "FAIL", "vendor_passes": 0,
              "app_launch_attempts": 0, "cleanup": "NOT_RUN", "docker_calls": 0,
              "diagnostic_integrity": {"status": "NOT_RUN", "reason": "session-not-collected", "vendor_passes": 0},
              "docker_actions": []}
    owned = False
    def closing(stage, args, deadline):
        report["docker_calls"] += 1
        action = {"stage": stage, "argv": ["docker", *args]}
        report["docker_actions"].append(action)
        try:
            result = run_bounded(action["argv"], deadline_seconds=deadline)
            action.update(status=result["status"], exit_code=result["exit_code"], bytes_read=result["bytes_read"])
            return result
        except (OSError, subprocess.TimeoutExpired) as error:
            action.update(status="FAIL", error_type=type(error).__name__)
            report["status"] = "FAIL"
            return None
    def docker(args, *, deadline=10, output_limit=65536, stdout_sink=None):
        check_phase()
        report["docker_calls"] += 1
        if report["docker_calls"] > 80:
            raise CanaryError("Docker client call limit exceeded")
        action = {"argv": ["docker", *args]}
        report["docker_actions"].append(action)
        result = run_bounded(action["argv"], deadline_seconds=deadline,
                             output_limit=output_limit, stdout_sink=stdout_sink)
        action.update(status=result["status"], exit_code=result["exit_code"],
                      bytes_read=result["bytes_read"], prefix_truncated=result["prefix_truncated"])
        if result["stderr"]:
            with (directory / f"helper-{report['docker_calls']}.stderr").open("xb") as file:
                file.write(result["stderr"])
        if result["status"] != "PASS":
            raise CanaryError(f"Docker helper {result['status']}")
        return result
    try:
        # No destructive action may target a pre-existing container.
        before = closing("before-container-audit", ["container", "inspect", name], 5)
        if before is None:
            raise CanaryError("container preflight helper failed")
        if not proven_absent(before, name):
            raise CanaryError("container absence not proven")
        owned = True  # Cleanup is registered before create, even if its client fails.
        argv = ["create", "--name", name, "--platform=linux/amd64", "--pull=never",
                "--network=none", "--read-only", "--init", "--user=1000:1000", "--cgroupns=private",
                "--memory=1536m", "--memory-swap=1536m", "--pids-limit=256", "--cpus=2",
                "--shm-size=64m", "--cap-drop=ALL", "--security-opt=no-new-privileges",
                "--tmpfs=/home/canary:rw,nosuid,nodev,size=64m,uid=1000,gid=1000",
                "--tmpfs=/tmp:rw,nosuid,nodev,size=64m,mode=1777",
                "--tmpfs=/runtime:rw,nosuid,nodev,size=8m,uid=1000,gid=1000,mode=0700",
                "--tmpfs=/output:rw,nosuid,nodev,size=32m,uid=1000,gid=1000",
                "--mount", f"type=bind,src={controls},dst=/input,readonly", image]
        report["create_argv"] = ["docker", *argv]
        docker(argv)
        docker(["start", name])
        report["app_launch_attempts"] = "PENDING_SESSION_RECEIPT"
        deadline = time.monotonic() + 60
        for _ in range(60):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise CanaryError("startup container deadline exceeded")
            state = docker(["exec", name, "python3", "-c",
                            "from pathlib import Path; print('READY' if Path('/output/ready').is_file() else 'WAIT')"],
                           deadline=min(3, remaining), output_limit=4096)
            if state["stdout"] == b"READY\n":
                break
            if state["stdout"] != b"WAIT\n":
                raise CanaryError("unexpected collection readiness state")
            time.sleep(min(1, max(0, deadline - time.monotonic())))
        else:
            raise CanaryError("startup polling count exceeded")
        report["container_inspect"] = json.loads(docker(["inspect", name])["stdout"])
        status = report["container_inspect"][0]["State"]
        report["container_state"] = status
        if not status["Running"]:
            raise CanaryError("tmpfs collection lease ended before capture")
        archive = directory / "output.tar"
        with archive.open("xb") as file:
            result = docker(["exec", name, "python3", "-c", COLLECT_PROGRAM, "/output"], deadline=15,
                            output_limit=40 * 1024 ** 2, stdout_sink=file)
        report["output_tar_bytes_read"] = result["bytes_read"]["stdout"]
        report["output_archive"] = extract_output(archive, directory / "capture")
        session_path = directory / "capture" / "output" / "session.json"
        session_payload, session_identity = read_pinned_metadata(session_path, max_bytes=256 * 1024)
        session = json.loads(session_payload)
        report["session_identity"] = session_identity
        if not isinstance(session, dict):
            raise CanaryError("session receipt must be an object")
        report["session_status"] = session["status"]
        report["app_launch_attempts"] = session["app_launch_attempts"]
        if session["status"] == "FAIL":
            report["primary_failure"] = {"origin": "session", "error_type": session.get("error_type"),
                                         "terminal_failure": session.get("terminal_failure"),
                                         "receipt_refusal": session.get("receipt_refusal")}
        report["diagnostic_integrity"] = validate_session(session, session_path.parent)
        if (session["status"] == "STARTUP_OBSERVED" and not status["OOMKilled"]
                and report["diagnostic_integrity"]["status"] == "PASS"):
            report["status"] = "STARTUP_OBSERVED"
    except (OSError, CanaryError, tarfile.TarError, ValueError, KeyError) as error:
        report["error_type"] = type(error).__name__
        report["phase_deadline_exceeded"] = isinstance(error, PhaseDeadline)
        if report["app_launch_attempts"] == "PENDING_SESSION_RECEIPT":
            report["app_launch_attempts"] = "UNKNOWN_AFTER_START"
    finally:
        if owned:
            logs = closing("collect-container-logs", ["logs", name], 5)
            if logs is not None:
                try:
                    with (directory / "container.log").open("xb") as file:
                        file.write(logs["stdout"] + logs["stderr"])
                    if logs["status"] != "PASS":
                        report["status"] = "FAIL"
                except OSError as error:
                    report["status"] = "FAIL"
                    report["log_write_error_type"] = type(error).__name__
            removed = closing("remove-owned-container", ["rm", "--force", name], 15)
            after = closing("after-container-audit", ["container", "inspect", name], 5)
            report["cleanup"] = ("PASS" if removed is not None and removed["status"] == "PASS"
                                  and proven_absent(after, name) else "FAIL")
            if report["cleanup"] != "PASS":
                report["status"] = "FAIL"
        report["elapsed_seconds"] = time.monotonic() - started
        if SCHEDULING_DEADLINE is not None and time.monotonic() >= SCHEDULING_DEADLINE:
            report["status"] = "FAIL"
            report["phase_deadline_exceeded"] = True
        with (directory / "attempt.json").open("x") as file:
            json.dump(report, file, indent=2)
            file.write("\n")
    return report


def execute(protocol: Path, output: Path):
    check_phase()
    payload, identity = read_pinned_metadata(protocol)
    plan = json.loads(payload)
    if plan["protocol"] != "original-pdf-startup-v1" or len(plan["session_names"]) != 2:
        raise CanaryError("wrong finite startup protocol")
    image = plan["image_id"]
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image):
        raise CanaryError("an exact prebuilt image ID is required")
    names = plan["session_names"]
    if len(set(names)) != 2 or any(not re.fullmatch(r"cajviewer-124-[a-z0-9-]{1,50}", name) for name in names):
        raise CanaryError("invalid predeclared session names")
    controls = Path(plan["controls_dir"]).resolve(strict=True)
    if ({record["path"] for record in plan["source_files"]} != SOURCE_FILES
            or len(plan["source_files"]) != len(SOURCE_FILES)
            or {record["path"] for record in plan["control_files"]} != CONTROL_FILES
            or len(plan["control_files"]) != len(CONTROL_FILES)
            or {path.name for path in controls.iterdir()} != CONTROL_FILES):
        raise CanaryError("missing, duplicate, or unexpected frozen source/control files")
    audit_files(plan["source_files"], ROOT)
    audit_files(plan["control_files"], controls)
    for name, options in (("digital.pdf", {}), ("alternate-unicode.pdf", {"alternate": True}),
                          ("image-only.pdf", {"image_only": True}), ("second-text.pdf", {"second": True})):
        original = controls_recipe.pdf(**options)
        hash_regular(controls / name, expected_size=len(original),
                     expected_sha256=hashlib.sha256(original).hexdigest())
    observed_image = json.loads(command(["docker", "image", "inspect", image])["stdout"])[0]
    if observed_image["Id"] != image or observed_image["Architecture"] != "amd64" or observed_image["Os"] != "linux":
        raise CanaryError("runtime image identity/platform mismatch")
    record = plan["runtime_inventory"]
    if record["image_id"] != image:
        raise CanaryError("runtime inventory belongs to a different image")
    inventory_payload, inventory_identity = read_pinned_metadata(Path(record["path"]), max_bytes=4 * 1024 ** 2)
    if (inventory_identity["sha256"] != record["sha256"]
            or inventory_identity["size_bytes"] != record["size_bytes"]):
        raise CanaryError("runtime inventory identity mismatch")
    inventory = json.loads(inventory_payload)
    inventory_by_path = {entry["path"]: entry for entry in inventory["files"]}
    for original, installed in (("scripts/cajviewer_canary.py", "/opt/canary/cajviewer_canary.py"),
                                ("tools/cajviewer/cajviewer_session.py", "/opt/canary/cajviewer_session.py")):
        entry = inventory_by_path[installed]
        hash_regular(ROOT / original, expected_size=entry["size_bytes"], expected_sha256=entry["sha256"])
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = {"protocol_identity": identity, "vendor_passes": 0,
              "scope": "original-pdf-startup-only", "status": "FAIL", "attempts": [],
              "phase_metadata_helpers": [{"argv": ["docker", "image", "inspect", image], "status": "PASS"}]}
    try:
        for ordinal, name in enumerate(names, 1):
            check_phase()
            result = attempt(image, controls, output / f"session-{ordinal}", name)
            report["attempts"].append(result)
            if result["cleanup"] != "PASS" or result.get("phase_deadline_exceeded"):
                break
        if len(report["attempts"]) == 2 and all(result["status"] == "STARTUP_OBSERVED" for result in report["attempts"]):
            report["status"] = "STARTUP_OBSERVED"
    finally:
        try:
            audit_files(plan["source_files"], ROOT)
            audit_files(plan["control_files"], controls)
            hash_regular(protocol, expected_size=identity["size_bytes"], expected_sha256=identity["sha256"])
            hash_regular(Path(record["path"]), expected_size=record["size_bytes"], expected_sha256=record["sha256"])
            report["final_file_audit"] = "PASS"
        except (OSError, CanaryError) as error:
            report["status"] = "FAIL"
            report["final_file_audit"] = "FAIL"
            report["audit_error_type"] = type(error).__name__
        with (output / "run.json").open("x") as file:
            json.dump(report, file, indent=2)
            file.write("\n")
    return report


def main(argv=None):
    global SCHEDULING_DEADLINE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", type=Path)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args(argv)
    if args.protocol is None and args.output_dir is None:
        print(json.dumps({"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0}))
        return 0
    if args.protocol is None or args.output_dir is None:
        parser.error("--protocol and --output-dir must be supplied together")
    SCHEDULING_DEADLINE = time.monotonic() + 360
    try:
        report = execute(args.protocol, args.output_dir)
    except (OSError, CanaryError, ValueError, KeyError, RecursionError) as error:
        print(json.dumps({"status": "FAIL", "error_type": type(error).__name__, "vendor_passes": 0}))
        return 1
    finally:
        SCHEDULING_DEADLINE = None
    print(json.dumps({key: report[key] for key in ("status", "scope", "vendor_passes")}))
    return 0 if report["status"] == "STARTUP_OBSERVED" else 1


if __name__ == "__main__":
    raise SystemExit(main())
