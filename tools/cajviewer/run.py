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
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time
import tokenize
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

# BEGIN INVENTORY HELPER COMMON
import base64
import math

INVENTORY_COMMANDS = (
    ("dpkg-query","-W","-f=${Package}\t${Version}\t${Architecture}\n"),
    ("fc-list","--format","%{file}\t%{family}\t%{style}\n"),
)
INVENTORY_LIMITS = {"stdout":262144,"stderr":65536}
INVENTORY_ENVELOPE_BYTES = 4 * 1024**2
INVENTORY_ERROR_TYPES = PUBLIC_SOURCE_ERROR_TYPES
INVENTORY_RESULT_FIELDS = ("helper_status","exit_code","spawned","captures","bytes_read")


class InventoryDiagnosticError(ValueError):
    pass


def _inventory_kind(error):
    name = type(error).__name__
    return name if name in INVENTORY_ERROR_TYPES else "OTHER_ERROR_TYPE"


def _inventory_encoded(value):
    return (json.dumps(value,sort_keys=True,separators=(",",":"),ensure_ascii=True) + "\n").encode()
# END INVENTORY HELPER COMMON

# BEGIN INVENTORY HELPER PRODUCER
def inventory_helper_call(run,report,argv,**kwargs):
    actions = report["actions"]
    ordinal = len(actions) + 1
    if (ordinal > 2 or list(argv) != list(INVENTORY_COMMANDS[ordinal - 1])
            or kwargs != {"deadline_seconds":10,"output_limit":262144}
            or any(type(value) is not int for value in kwargs.values())
            or report.get("terminal_helper") is not None
            or actions and actions[-1]["status"] != "PASS"):
        raise ValueError
    action = {"ordinal":ordinal,"argv":list(argv),"status":"PENDING","spawned":None,
              "helper_status":None,"exit_code":None,"bytes_read":None,"captures":None,
              "elapsed_seconds":None,"error_type":None}
    actions.append(action)
    reason,stage,stderr = "HELPER_RESULT_UNAVAILABLE","metadata-helper-run",None
    try:
        result = run(argv,**kwargs)
        if result is None:
            raise ValueError
        reason = "HELPER_RESULT_MALFORMED"
        if (type(result) is not dict or set(result) != {"status","exit_code","stdout","stderr",
                "bytes_read","prefix_truncated","elapsed_seconds"}
                or type(result["status"]) is not str or result["status"] not in ("PASS","FAIL","TIMEOUT","OUTPUT_LIMIT")
                or type(result["exit_code"]) is not int or not -255 <= result["exit_code"] <= 255
                or type(result["prefix_truncated"]) is not bool
                or result["prefix_truncated"] is not (result["status"] == "OUTPUT_LIMIT")
                or type(result["elapsed_seconds"]) not in (int,float)
                or not math.isfinite(result["elapsed_seconds"]) or not 0 <= result["elapsed_seconds"] <= 1200
                or type(result["bytes_read"]) is not dict or set(result["bytes_read"]) != set(INVENTORY_LIMITS)):
            raise ValueError
        for name,limit in INVENTORY_LIMITS.items():
            count,raw = result["bytes_read"][name],result[name]
            if (type(count) is not int or not 0 <= count <= limit + 1 or type(raw) is not bytes
                    or len(raw) > min(count,262144)
                    or result["status"] not in ("TIMEOUT","OUTPUT_LIMIT") and count > limit):
                raise ValueError
        captured = {name:result[name][:limit] for name,limit in INVENTORY_LIMITS.items()}
        partial = result["status"] in ("TIMEOUT","OUTPUT_LIMIT")
        captures = {name:{"size_bytes":len(raw),"sha256":hashlib.sha256(raw).hexdigest(),
                     "complete":not partial and len(raw) == result["bytes_read"][name],
                     "hash_scope":"retained-stream" if not partial and len(raw) == result["bytes_read"][name] else "captured-prefix"}
                    for name,raw in captured.items()}
        action.update(spawned=True,helper_status=result["status"],exit_code=result["exit_code"],
                      bytes_read=dict(result["bytes_read"]),captures=captures,elapsed_seconds=result["elapsed_seconds"])
        stderr = captured["stderr"]
        if result["status"] == "TIMEOUT":
            reason = "HELPER_TIMEOUT"
        elif result["status"] == "OUTPUT_LIMIT":
            reason = "HELPER_OUTPUT_LIMIT"
        elif result["exit_code"] != 0:
            reason = "HELPER_NONZERO_EXIT"
        elif result["status"] != "PASS":
            reason = "HELPER_RESULT_MALFORMED"
        elif not all(value["complete"] for value in captures.values()):
            reason,stage = "HELPER_CAPTURE_INCOMPLETE","metadata-helper-validation"
        elif stderr:
            reason,stage = "HELPER_STDERR_NOT_EMPTY","metadata-helper-validation"
        else:
            action["status"] = "PASS"
            return {**result,**captured}
        raise ValueError
    except BaseException as error:
        action.update(status="FAIL",error_type=_inventory_kind(error))
        if report.get("terminal_helper") is None:
            kept = None if stderr is None else stderr[:4096]
            complete = kept is not None and action["captures"]["stderr"]["complete"] and len(stderr) <= 4096
            report["terminal_helper"] = {"schema":"cajviewer-inventory-helper-diagnostic/1",
                "action_ordinal":ordinal,"stage":stage,"reason":reason,"error_type":action["error_type"],
                **{key:action[key] for key in INVENTORY_RESULT_FIELDS},
                "stderr":None if kept is None else {"encoding":"base64","data":base64.b64encode(kept).decode("ascii"),
                    "retained_bytes":len(kept),"sha256":hashlib.sha256(kept).hexdigest(),"complete":bool(complete),
                    "truncated":not complete,"hash_scope":"retained-stream" if complete else "captured-prefix"}}
        raise


def inventory_envelope_bytes(report,maximum=INVENTORY_ENVELOPE_BYTES):
    if type(maximum) is not int or not 256 <= maximum <= INVENTORY_ENVELOPE_BYTES:
        raise InventoryDiagnosticError("inventory-envelope-limit")
    payload = _inventory_encoded(report)
    if len(payload) > maximum:
        report.update(status="FAIL",envelope_refusal="envelope-size-refusal",inventory_json=None,inventory_bytes_omitted=True)
        payload = _inventory_encoded(report)
    if len(payload) > maximum:
        raise InventoryDiagnosticError("inventory-envelope-size")
    return payload
# END INVENTORY HELPER PRODUCER

# BEGIN INVENTORY HELPER VALIDATOR
def _inventory_hex(value):
    return type(value) is str and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def validate_inventory_actions(actions):
    """Validate the two fixed action records, without trusting producer guards."""
    keys = {"ordinal", "argv", "status", "spawned", "helper_status", "exit_code", "bytes_read",
            "captures", "elapsed_seconds", "error_type"}
    if type(actions) is not list or len(actions) > 2:
        raise InventoryDiagnosticError("inventory-action-count")
    safe = []
    for ordinal, action in enumerate(actions, 1):
        if (type(action) is not dict or set(action) != keys or type(action["ordinal"]) is not int
                or action["ordinal"] != ordinal or type(action["argv"]) is not list
                or any(type(value) is not str for value in action["argv"])
                or action["argv"] != list(INVENTORY_COMMANDS[ordinal - 1])
                or type(action["status"]) is not str or action["status"] not in ("PASS", "FAIL")
                or ordinal > 1 and safe[-1]["status"] != "PASS"):
            raise InventoryDiagnosticError("inventory-action-framing")
        if action["helper_status"] is None:
            if (action["status"] != "FAIL" or any(action[name] is not None for name in
                    ("spawned", "exit_code", "bytes_read", "captures", "elapsed_seconds"))):
                raise InventoryDiagnosticError("inventory-unavailable-result")
        else:
            status, counts, captures = action["helper_status"], action["bytes_read"], action["captures"]
            if (type(status) is not str or status not in ("PASS", "FAIL", "TIMEOUT", "OUTPUT_LIMIT")
                    or action["spawned"] is not True or type(action["exit_code"]) is not int
                    or not -255 <= action["exit_code"] <= 255
                    or type(action["elapsed_seconds"]) not in (int, float)
                    or not math.isfinite(action["elapsed_seconds"]) or not 0 <= action["elapsed_seconds"] <= 1200
                    or type(counts) is not dict or set(counts) != set(INVENTORY_LIMITS)
                    or type(captures) is not dict or set(captures) != set(INVENTORY_LIMITS)):
                raise InventoryDiagnosticError("inventory-result-framing")
            for name, limit in INVENTORY_LIMITS.items():
                value, count = captures[name], counts[name]
                if (type(count) is not int or not 0 <= count <= limit + 1
                        or status not in ("TIMEOUT", "OUTPUT_LIMIT") and count > limit
                        or type(value) is not dict or set(value) != {"size_bytes", "sha256", "complete", "hash_scope"}
                        or type(value["size_bytes"]) is not int or not 0 <= value["size_bytes"] <= min(count, limit)
                        or not _inventory_hex(value["sha256"]) or type(value["complete"]) is not bool
                        or value["complete"] is not (status not in ("TIMEOUT", "OUTPUT_LIMIT") and value["size_bytes"] == count)
                        or type(value["hash_scope"]) is not str
                        or value["hash_scope"] != ("retained-stream" if value["complete"] else "captured-prefix")
                        or value["size_bytes"] == 0 and value["sha256"] != hashlib.sha256(b"").hexdigest()):
                    raise InventoryDiagnosticError("inventory-capture-framing")
            passed = (status == "PASS" and action["exit_code"] == 0
                      and all(value["complete"] for value in captures.values()) and counts["stderr"] == 0)
            if (action["status"] == "PASS") is not passed:
                raise InventoryDiagnosticError("inventory-contradictory-result")
        if (action["status"] == "PASS" and action["error_type"] is not None
                or action["status"] == "FAIL" and (type(action["error_type"]) is not str
                                                   or action["error_type"] not in INVENTORY_ERROR_TYPES)):
            raise InventoryDiagnosticError("inventory-error-type")
        safe.append(json.loads(json.dumps(action)))
    return safe


def validate_inventory_diagnostic(diagnostic, actions):
    """Bind metadata; larger/incomplete prefix membership trusts the producer."""
    actions = validate_inventory_actions(actions)
    failed = bool(actions and actions[-1]["status"] == "FAIL")
    if diagnostic is None:
        if failed:
            raise InventoryDiagnosticError("inventory-diagnostic-missing")
        return None
    keys = {"schema", "action_ordinal", "stage", "reason", "error_type", "helper_status", "exit_code",
            "spawned", "captures", "bytes_read", "stderr"}
    if (not failed or type(diagnostic) is not dict or set(diagnostic) != keys
            or diagnostic["schema"] != "cajviewer-inventory-helper-diagnostic/1"
            or type(diagnostic["action_ordinal"]) is not int or diagnostic["action_ordinal"] != len(actions)):
        raise InventoryDiagnosticError("inventory-diagnostic-framing")
    action = actions[-1]
    for name in ("error_type", *INVENTORY_RESULT_FIELDS):
        if (type(diagnostic[name]) is not type(action[name])
                or json.dumps(diagnostic[name], sort_keys=True) != json.dumps(action[name], sort_keys=True)):
            raise InventoryDiagnosticError("inventory-diagnostic-binding")
    status = action["helper_status"]
    if status is None:
        reasons, stage = ("HELPER_RESULT_UNAVAILABLE", "HELPER_RESULT_MALFORMED"), "metadata-helper-run"
    elif status == "TIMEOUT":
        reasons, stage = ("HELPER_TIMEOUT",), "metadata-helper-run"
    elif status == "OUTPUT_LIMIT":
        reasons, stage = ("HELPER_OUTPUT_LIMIT",), "metadata-helper-run"
    elif action["exit_code"] != 0:
        reasons, stage = ("HELPER_NONZERO_EXIT",), "metadata-helper-run"
    elif status != "PASS":
        reasons, stage = ("HELPER_RESULT_MALFORMED",), "metadata-helper-run"
    elif not all(value["complete"] for value in action["captures"].values()):
        reasons, stage = ("HELPER_CAPTURE_INCOMPLETE",), "metadata-helper-validation"
    else:
        reasons, stage = ("HELPER_STDERR_NOT_EMPTY",), "metadata-helper-validation"
    if type(diagnostic["reason"]) is not str or diagnostic["reason"] not in reasons or diagnostic["stage"] != stage:
        raise InventoryDiagnosticError("inventory-diagnostic-reason")
    stderr = diagnostic["stderr"]
    if status is None:
        if stderr is not None:
            raise InventoryDiagnosticError("inventory-unavailable-stderr")
    else:
        capture = action["captures"]["stderr"]
        complete = capture["complete"] and capture["size_bytes"] <= 4096
        if (type(stderr) is not dict or set(stderr) != {"encoding", "data", "retained_bytes", "sha256",
                    "complete", "truncated", "hash_scope"}
                or stderr["encoding"] != "base64" or type(stderr["data"]) is not str or len(stderr["data"]) > 5464
                or type(stderr["retained_bytes"]) is not int or stderr["retained_bytes"] != min(capture["size_bytes"], 4096)
                or not _inventory_hex(stderr["sha256"]) or type(stderr["complete"]) is not bool
                or stderr["complete"] is not complete or type(stderr["truncated"]) is not bool
                or stderr["truncated"] is not (not complete)
                or stderr["hash_scope"] != ("retained-stream" if complete else "captured-prefix")):
            raise InventoryDiagnosticError("inventory-diagnostic-stderr")
        try:
            raw = base64.b64decode(stderr["data"].encode("ascii"), validate=True)
        except (ValueError, UnicodeError):
            raise InventoryDiagnosticError("inventory-diagnostic-base64") from None
        if (len(raw) != stderr["retained_bytes"] or base64.b64encode(raw).decode("ascii") != stderr["data"]
                or hashlib.sha256(raw).hexdigest() != stderr["sha256"]
                or complete and stderr["sha256"] != capture["sha256"]):
            raise InventoryDiagnosticError("inventory-diagnostic-identity")
    return json.loads(json.dumps(diagnostic))


def inventory_helper_observation(envelope, *, complete):
    unknown = {"observation": "UNKNOWN", "attempted": None, "spawned": None, "actions": None,
               "terminal_helper": None, "reason": "INVENTORY_CAPTURE_INCOMPLETE"}
    if complete is not True:
        return unknown
    if type(envelope) is not dict or envelope.get("status") not in ("PASS", "FAIL"):
        return {**unknown, "reason": "INVENTORY_ACCOUNTING_MALFORMED"}
    try:
        actions = validate_inventory_actions(envelope.get("actions"))
        diagnostic = validate_inventory_diagnostic(envelope.get("terminal_helper"), actions)
        if envelope["status"] == "PASS" and (len(actions) != 2 or any(row["status"] != "PASS" for row in actions)):
            raise InventoryDiagnosticError("inventory-impossible-pass")
        unknown_count = sum(row["spawned"] is None for row in actions)
        observed_count = sum(row["spawned"] is True for row in actions)
        return {"observation": "COMPLETE_ORDERED_RECORDS", "attempted": len(actions),
                "spawned": None if unknown_count else observed_count, "observed_spawned": observed_count,
                "spawn_unknown": unknown_count, "actions": actions,
                "terminal_helper": diagnostic}
    except InventoryDiagnosticError:
        return {**unknown, "reason": "INVENTORY_ACCOUNTING_MALFORMED"}
# END INVENTORY HELPER VALIDATOR

# BEGIN INVENTORY HELPER HOST
def parse_inventory_envelope(raw):
    """Parse only bounded complete JSON, rejecting duplicates and deep nesting."""
    if type(raw) is not bytes or not raw or len(raw) > INVENTORY_ENVELOPE_BYTES:
        raise InventoryDiagnosticError("inventory-envelope-bounds")
    depth, quoted, escaped = 0, False, False
    for byte in raw:
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            if depth > 32:
                raise InventoryDiagnosticError("inventory-envelope-depth")
        elif byte in (93, 125):
            depth -= 1
            if depth < 0:
                raise InventoryDiagnosticError("inventory-envelope-framing")
    def pairs(items):
        value = {}
        for key, item in items:
            if key in value:
                raise InventoryDiagnosticError("inventory-envelope-duplicate")
            value[key] = item
        return value
    def constant(value):
        raise InventoryDiagnosticError("inventory-envelope-number")
    try:
        value = json.loads(raw.decode("utf-8", "strict"), object_pairs_hook=pairs, parse_constant=constant)
    except (UnicodeError, ValueError, RecursionError):
        raise InventoryDiagnosticError("inventory-envelope-framing") from None
    if (type(value) is not dict or value.get("schema") != "cajviewer-inventory-accounting/1"
            or type(value.get("status")) is not str or value["status"] not in ("PASS", "FAIL")
            or any(type(value.get(key)) is not int or value[key] != 0 for key in ("app_launches", "vendor_passes"))):
        raise InventoryDiagnosticError("inventory-envelope-framing")
    return value


def require_inventory_success(envelope, expected_environment_identity, *, pins=PUBLIC_SOURCE_PINS):
    """Require existing two-helper/source/cap/ENV/UID integrity before PASS."""
    observed = inventory_helper_observation(envelope, complete=True)
    loading = source_loading_observation(envelope, complete=True, pins=pins)
    if (envelope["status"] != "PASS" or envelope.get("scope") != "opaque-runtime-inventory-only"
            or observed["observation"] != "COMPLETE_ORDERED_RECORDS" or observed["attempted"] != 2
            or any(row["status"] != "PASS" for row in observed["actions"])
            or loading["observation"] != "COMPLETE_ORDERED_RECORDS" or loading["records_completed"] != 2
            or envelope.get("closing_audits") != {"caps": "PASS", "environment": "PASS", "user": "PASS"}
            or envelope.get("closing_failures") != [] or envelope.get("inventory_bytes_omitted", False) is not False
            or type(envelope.get("inventory_json")) is not str
            or any(type(envelope.get(key)) is not int or envelope[key] != 0 for key in ("app_launches", "vendor_passes"))):
        raise InventoryDiagnosticError("required-inventory-envelope")
    for key in ("environment_identity", "environment_after_identity"):
        value = envelope.get(key)
        if (type(value) is not dict or set(value) != {"size_bytes", "sha256"}
                or type(value["size_bytes"]) is not int or not 0 < value["size_bytes"] <= 262144
                or not _inventory_hex(value["sha256"]) or value != expected_environment_identity):
            raise InventoryDiagnosticError("required-inventory-environment")
    for key in ("uid_gid", "uid_gid_after"):
        value = envelope.get(key)
        if type(value) is not list or len(value) != 2 or any(type(item) is not int or item != 1000 for item in value):
            raise InventoryDiagnosticError("required-inventory-user")
    keys = {"memory_max_bytes", "memory_swap_max_bytes", "memory_peak_bytes", "pids_max", "pids_peak",
            "cpu_quota", "cpu_period", "memory_events"}
    for key in ("caps_before", "caps_after"):
        caps = envelope.get(key)
        if (type(caps) is not dict or set(caps) != keys or any(type(caps[name]) is not int for name in keys - {"memory_events"})
                or caps["memory_max_bytes"] != 536870912 or caps["memory_swap_max_bytes"] != 0
                or caps["pids_max"] != 64 or not 0 <= caps["memory_peak_bytes"] <= 536870912
                or not 0 <= caps["pids_peak"] <= 64 or not 0 < caps["cpu_period"] <= 10**9
                or caps["cpu_quota"] != 2 * caps["cpu_period"] or type(caps["memory_events"]) is not dict
                or len(caps["memory_events"]) > 64
                or any(type(name) is not str or len(name) > 64 or type(count) is not int or not 0 <= count < 2**63
                       for name, count in caps["memory_events"].items())
                or any(caps["memory_events"].get(name) != 0 for name in ("oom", "oom_kill"))):
            raise InventoryDiagnosticError("required-container-cap-facts")
    raw = envelope["inventory_json"].encode("utf-8", "strict")
    if (len(raw) > INVENTORY_ENVELOPE_BYTES or type(envelope.get("inventory_identity")) is not dict
            or envelope["inventory_identity"] != {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
            or type(envelope["inventory_identity"].get("size_bytes")) is not int):
        raise InventoryDiagnosticError("underlying-inventory-identity")


def observe_inventory_result(result, report, expected_environment_identity, *, pins=PUBLIC_SOURCE_PINS):
    """Adopt accounting before outer refusal; enclosing host owns cleanup."""
    report["nested_helpers"] = inventory_helper_observation(None, complete=False)
    report["public_source_loading"] = source_loading_observation(None, complete=False, pins=pins)
    if (type(result) is not dict or result.get("status") not in ("PASS", "FAIL")
            or result.get("prefix_truncated") is not False or type(result.get("stdout")) is not bytes
            or type(result.get("bytes_read")) is not dict
            or type(result["bytes_read"].get("stdout")) is not int
            or result["bytes_read"]["stdout"] != len(result["stdout"])):
        raise InventoryDiagnosticError("incomplete-inventory-envelope")
    try:
        envelope = parse_inventory_envelope(result["stdout"])
    except InventoryDiagnosticError:
        raise InventoryDiagnosticError("inventory-envelope-framing") from None
    report["nested_helpers"] = inventory_helper_observation(envelope, complete=True)
    report["public_source_loading"] = source_loading_observation(envelope, complete=True, pins=pins)
    report["container_caps"] = {key: envelope.get(key) for key in ("caps_before", "caps_after")}
    report["container_environment"] = {key: envelope.get(key) for key in
        ("environment_identity", "environment_after_identity", "uid_gid", "uid_gid_after", "closing_audits", "closing_failures")}
    if (report["nested_helpers"]["observation"] == "UNKNOWN"
            or report["public_source_loading"]["observation"] == "UNKNOWN"):
        raise InventoryDiagnosticError("inventory-accounting-malformed")
    if (result["status"] != "PASS" or type(result.get("exit_code")) is not int or result["exit_code"] != 0
            or type(result.get("stderr")) is not bytes or result["stderr"]
            or type(result["bytes_read"].get("stderr")) is not int or result["bytes_read"]["stderr"] != 0):
        raise InventoryDiagnosticError("required-inventory-helper")
    require_inventory_success(envelope, expected_environment_identity, pins=pins)
    return envelope
# END INVENTORY HELPER HOST


def inventory_helper_fragment(source, part):
    """Include the exact public COMMON dependency; HOST also gets its validator."""
    if type(source) is not bytes or len(source) > 65536 or part not in ("PRODUCER", "VALIDATOR", "HOST"):
        raise InventoryDiagnosticError("inventory-fragment")
    fragments, previous_end = [], -1
    for name in (("COMMON", "VALIDATOR", "HOST") if part == "HOST" else ("COMMON", part)):
        start = ("# BEGIN INVENTORY HELPER " + name + "\n").encode()
        end = ("# END INVENTORY HELPER " + name + "\n").encode()
        if source.count(start) != 1 or source.count(end) != 1:
            raise InventoryDiagnosticError("inventory-fragment")
        left, right = source.index(start), source.index(end)
        if right <= left or left <= previous_end:
            raise InventoryDiagnosticError("inventory-fragment")
        fragments.append(source[left:right + len(end)])
        previous_end = right + len(end) - 1
    dependency = public_source_fragment(source, "VALIDATOR" if part == "HOST" else "LOADER")
    if part != "HOST":
        dependency = dependency.split(b"# BEGIN PUBLIC SOURCE LOADER\n", 1)[0]
    fragments.insert(0, dependency)
    return b"\n".join(fragments)


# Complete owned entry; no operational profile. Source-loader bytes stay exact.
INVENTORY_ENTRY_PRELUDE = r'''# SPDX-License-Identifier: MIT
import hashlib,json,os,shutil,stat,sys,types
from pathlib import Path
report={"schema":"cajviewer-inventory-accounting/1","status":"FAIL",
 "scope":"opaque-runtime-inventory-only","app_launches":0,"vendor_passes":0,
 "actions":[],"source_loads":[],"terminal_helper":None,"inventory_json":None,
 "inventory_identity":None,"caps_before":None,"caps_after":None,
 "environment_identity":None,"environment_after_identity":None,"uid_gid":None,"uid_gid_after":None,
 "closing_audits":{"caps":"NOT_RUN","environment":"NOT_RUN","user":"NOT_RUN"},"closing_failures":[]}
expected_env=environment=None
'''

INVENTORY_ENTRY_BODY = r'''
def identity(raw):
 return {"size_bytes":len(raw),"sha256":hashlib.sha256(raw).hexdigest()}
def environment_snapshot():
 value=dict(os.environ)
 if len(value)>64 or any(len(key)>128 or len(item)>4096 for key,item in value.items()):
  raise ValueError
 return value
def env_identity(value):
 return identity(_inventory_encoded(value))
def closing(kind,error=None,unknown=False):
 report["status"]="FAIL"
 report["closing_audits"][kind]="UNKNOWN" if unknown else "FAIL"
 report["closing_failures"].append({"kind":kind,"reason":"baseline-unavailable" if unknown else "closing-observation-failed",
  "error_type":None if error is None else _inventory_kind(error)})
def read_fixed(path,maximum):
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
 with os.fdopen(fd,"rb",buffering=0) as stream:
  if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
   raise ValueError
  chunks,size=[],0
  while True:
   requested=min(65536,maximum+1-size)
   counts=report.setdefault("fixed_read_counts",{"calls":0,"requested_bytes":0,"returned_bytes":0})
   counts["calls"]+=1
   counts["requested_bytes"]+=requested
   raw=stream.read(requested)
   counts["returned_bytes"]+=len(raw)
   if not raw:
    return b"".join(chunks)
   size+=len(raw)
   if size>maximum:
    raise ValueError
   chunks.append(raw)
def caps():
 root=Path("/sys/fs/cgroup")
 values={name:read_fixed(str(root/name),4096).decode("ascii","strict").strip() for name in
  ("memory.max","memory.swap.max","memory.peak","memory.events","pids.max","pids.peak","cpu.max")}
 events={}
 for line in values["memory.events"].splitlines():
  name,value=line.split()
  if name in events or not value.isdecimal():
   raise ValueError
  events[name]=int(value)
 quota,period=values["cpu.max"].split()
 if (values["memory.max"]!="536870912" or values["memory.swap.max"]!="0" or values["pids.max"]!="64"
  or not quota.isdecimal() or not period.isdecimal() or int(period)==0 or int(quota)!=2*int(period)
  or not {"oom","oom_kill"}<=set(events) or not values["memory.peak"].isdecimal()
  or not values["pids.peak"].isdecimal() or int(values["memory.peak"])>536870912 or int(values["pids.peak"])>64):
  raise ValueError
 return {"memory_max_bytes":536870912,"memory_swap_max_bytes":0,"memory_peak_bytes":int(values["memory.peak"]),
  "pids_max":64,"pids_peak":int(values["pids.peak"]),"cpu_quota":int(quota),"cpu_period":int(period),"memory_events":events}
try:
 expected_env=json.loads(sys.argv[1])
 tools=json.loads(sys.argv[2])
 environment=environment_snapshot()
 report["environment_identity"]=env_identity(environment)
 report["uid_gid"]=[os.getuid(),os.getgid()]
 if environment!=expected_env or report["uid_gid"]!=[1000,1000]:
  raise ValueError
 report["caps_before"]=caps()
 if any(report["caps_before"]["memory_events"][key]!=0 for key in ("oom","oom_kill")):
  raise ValueError
 load_public_sources(read_fixed,report["source_loads"])
 inventory=sys.modules["inventory"]
 original_run,original_hash=inventory.run_bounded,inventory.hash_regular
 tool_pins={}
 def observed_hash(path,**kwargs):
  report["opaque_hash_attempts"]=report.get("opaque_hash_attempts",0)+1
  expected=next((value for value in tools.values() if value["path"]==str(path)),None)
  if expected is not None:
   kwargs.update(expected_size=expected["size_bytes"],expected_sha256=expected["sha256"],max_bytes=expected["size_bytes"])
  result=original_hash(path,**kwargs)
  report["completed_opaque_hash_bytes"]=report.get("completed_opaque_hash_bytes",0)+result["size_bytes"]
  if str(path) in {value["path"] for value in tools.values()}:
   tool_pins[str(path)]=result
  return result
 def observed_run(argv,**kwargs):
  ordinal=len(report["actions"])
  if ordinal>=2 or argv!=list(INVENTORY_COMMANDS[ordinal]) or kwargs!={"deadline_seconds":10,"output_limit":262144}:
   raise ValueError
  tool=tools[argv[0]]
  selected=shutil.which(argv[0])
  if (selected is None or str(Path(selected).resolve(strict=True))!=tool["path"]
   or tool_pins.get(tool["path"])!={key:tool[key] for key in ("size_bytes","sha256")}):
   raise ValueError
  return inventory_helper_call(original_run,report,argv,**kwargs)
 inventory.hash_regular,inventory.run_bounded=observed_hash,observed_run
 value=inventory.inventory()
 if len(report["actions"])!=2 or any(action["status"]!="PASS" for action in report["actions"]):
  raise ValueError
 text=json.dumps(value,sort_keys=True)
 report.update(inventory_identity=identity(text.encode("utf-8")),inventory_json=text,status="PASS")
except BaseException as error:
 report["error_type"]=(report["source_loads"][-1]["error_type"] if report["source_loads"]
  and report["source_loads"][-1]["status"]=="FAIL" else _inventory_kind(error))
finally:
 for kind in ("caps","environment","user"):
  try:
   if kind=="caps":
    after=report["caps_after"]=caps()
    before=report["caps_before"]
    if any(after["memory_events"][key]!=0 for key in ("oom","oom_kill")):
     raise ValueError
    unknown=before is None
    valid=unknown or all(after["memory_events"][key]==before["memory_events"][key] for key in ("oom","oom_kill"))
   elif kind=="environment":
    after_env=environment_snapshot()
    report["environment_after_identity"]=env_identity(after_env)
    unknown=environment is None or expected_env is None
    valid=unknown or after_env==environment==expected_env
   else:
    after=report["uid_gid_after"]=[os.getuid(),os.getgid()]
    if after!=[1000,1000]:
     raise ValueError
    unknown=report["uid_gid"] is None
    valid=unknown or after==report["uid_gid"]
   if not valid:
    raise ValueError
   if unknown:
    closing(kind,unknown=True)
   else:
    report["closing_audits"][kind]="PASS"
  except BaseException as error:
   closing(kind,error)
 payload=inventory_envelope_bytes(report)
 sys.stdout.buffer.write(payload)
 sys.stdout.buffer.flush()
sys.exit(0 if report["status"]=="PASS" else 1)
'''


def _compact_inventory_entry_piece(source, unit):
    """Compact fixed owned pieces, preserving string tokens and other code."""
    protected, comments = set(), {}
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
        if token.type == tokenize.STRING:
            protected.update(range(token.start[0] + 1, token.end[0] + 1))
        elif token.type == tokenize.COMMENT and "SPDX-License-Identifier" not in token.string:
            comments[token.start[0]] = token.start[1]
    lines = []
    for row, line in enumerate(source.splitlines(keepends=True), 1):
        if row in protected:
            lines.append(line)
            continue
        if row in comments:
            line = line[:comments[row]] + "\n"
        stripped = line.lstrip(" ")
        if stripped.strip():
            lines.append(" " * ((len(line) - len(stripped)) // unit) + stripped)
    return "".join(lines)


def inventory_entry_source(source):
    """Assemble fixed bytes only; caller must separately pin source and result."""
    loader = public_source_fragment(source, "LOADER")
    producer = inventory_helper_fragment(source, "PRODUCER")
    producer = producer[producer.index(b"# BEGIN INVENTORY HELPER COMMON\n"):]
    pieces = (INVENTORY_ENTRY_PRELUDE.encode(), loader,
              _compact_inventory_entry_piece(producer.decode("utf-8", "strict"), 4).encode(),
              INVENTORY_ENTRY_BODY.encode())
    result = b"\n".join(pieces)
    if len(result) > 16384:
        raise InventoryDiagnosticError("inventory-inline-size")
    return result

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
    if argv is None:
        argv = sys.argv[1:]
    if not argv:
        print(json.dumps({"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0}))
        return 0
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
