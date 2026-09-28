#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Refused-default host entry for the separately approved two-session protocol.

No input means NOT_RUN. No image build, pull, runtime discovery or GUI action
occurs before explicit pinned profile/runtime-view/source/approval admission.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))

from capability_io import (ALLOWLIST, Meter, consume, exclusive, read_file, stable)
from capability_clipboard import validate_copy
from capability_pages import PageGate, verify_page_frame
from capability_protocol import (CAPS, COLLECT_WAIT_RESERVE, HOST_TIME_RESERVE, COPY_KEYS, KINDS, PAGE_KEYS, RECEIPT_LIMIT, TRANSPORT_LIMIT,
                                 RUNTIME_LIMITS, Refusal, require)
from capability_protocol import PublicationFailure
from capability_protocol import strict_json, validate_profile
from capability_protocol import exact_value
from capability_protocol import discovery_outcome
from capability_runtime import RUNTIME_FILES, validate_runtime
from capability_session import encoded
from cajviewer_canary import run_bounded
import cajviewer_canary_fixtures as fixtures

SOURCE_PATHS = {name: "tools/cajviewer/" + name for name in (
    "capability_protocol.py", "capability_io.py", "capability_x11.py", "capability_clipboard.py",
    "capability_pages.py", "capability_session.py", "capability_collect.py", "capability_runtime.py", "run_capabilities.py", "run.py")}
SOURCE_PATHS.update({"cajviewer_canary.py": "scripts/cajviewer_canary.py",
                     "cajviewer_canary_fixtures.py": "scripts/cajviewer_canary_fixtures.py",
                     "cajviewer_session.py": "tools/cajviewer/cajviewer_session.py"})


class Host:
    def __init__(self, seconds, meter, command=run_bounded, clock=time.monotonic):
        self.command, self.meter, self.clock = command, meter, clock
        self.deadline = clock() + seconds
        self.actions, self.first_failure, self.closing_errors = [], None, []
        self.clients = 0

    def fail(self, stage, error, closing=False):
        record = {"stage": stage, "reason": str(error) if isinstance(error, Refusal) else "result-unavailable",
                  "error_type": type(error).__name__}
        if self.first_failure is None:
            self.first_failure = record
        if closing:
            self.closing_errors.append(record)

    def call(self, args, *, stage, limit=65536, sink=None, closing=False, seconds=60, observe=None, before_effect=None):
        require(self.clients < 32 and (closing or self.clock() < self.deadline), "host-helper-or-time-limit")
        self.clients += 1
        record = {"index": self.clients, "stage": stage, "argv": ["/usr/bin/docker", *args], "status": "UNKNOWN_AFTER_ADMISSION"}
        self.actions.append(record)
        try:
            if before_effect is not None:
                before_effect()
            result = self.command(record["argv"], deadline_seconds=10 if closing else min(seconds, self.deadline - self.clock()),
                                  output_limit=limit, stdout_sink=sink)
            if observe is not None:
                observe(result)
            record.update(status=result["status"], exit_code=result["exit_code"], bytes_read=result["bytes_read"],
                          prefix_truncated=result["prefix_truncated"], stdout_sha256=sink.digest.hexdigest() if sink is not None else hashlib.sha256(result["stdout"]).hexdigest(),
                          stderr_sha256=hashlib.sha256(result["stderr"]).hexdigest(),
                          hash_scope="complete-stream" if result["prefix_truncated"] is False and result["status"] in ("PASS", "FAIL") else "retained-prefix")
            require(result["prefix_truncated"] is False and result["status"] in ("PASS", "FAIL"), "host-helper-incomplete")
            require(result["status"] == "PASS" and exact_value(result["exit_code"], 0) and result["stderr"] == b"", "host-helper-failed")
            require(closing or self.clock() < self.deadline, "host-completion-after-deadline")
            return result
        except (Exception, KeyboardInterrupt) as error:
            self.fail(stage, error, closing)
            raise


class CheckedSink:
    def __init__(self, stream, meter):
        self.stream, self.meter, self.digest = stream, meter, hashlib.sha256()
        self.size = 0

    def write(self, payload):
        self.meter.write(self.stream, payload)
        self.digest.update(payload)
        self.size += len(payload)
        return len(payload)


def audit_inputs(root, controls, profile, meter):
    require(set(profile["sources"]) == set(SOURCE_PATHS), "source-delivery-set-mismatch")
    source_records = {}
    for name, path in SOURCE_PATHS.items():
        _, source_records[name] = read_file(root / path, meter, pin=profile["sources"][name], retain=False, metadata=True)
    control_records = {}
    for name, options in (("digital.pdf", {}), ("image-only.pdf", {"image_only": True})):
        payload, control_records[name] = read_file(controls / name, meter, pin=profile["controls"][name], limit=1024 ** 2, metadata=True)
        require(payload == fixtures.pdf(**options), "not-original-control-bytes")
    return {"sources": source_records, "controls": control_records}


def create_argv(name, profile, root, controls, profile_path, approval_path):
    args = ["create", "--pull", "never", "--name", name, "--hostname", "original-capability", "--network", "none",
            "--read-only", "--user", "1000:1000", "--init", "--cgroupns", "private", "--ipc", "private",
            "--memory", "1536m", "--memory-swap", "1536m", "--cpus", "2", "--pids-limit", "256",
            "--shm-size", "64m", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--ulimit", "core=0:0", "--ulimit", "fsize=67108864:67108864",
            "--tmpfs", "/home/canary:rw,nosuid,nodev,size=64m,uid=1000,gid=1000",
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=64m", "--tmpfs", "/runtime:rw,nosuid,nodev,size=8m,uid=1000,gid=1000",
            "--tmpfs", "/output:rw,nosuid,nodev,size=32m,uid=1000,gid=1000", "--workdir", "/home/canary"]
    for name, value in sorted(profile["environment"].items()):
        args += ["--env", name + "=" + value]
    for name, relative in sorted(SOURCE_PATHS.items()):
        require("," not in str(root / relative), "unsupported-mount-path")
        args += ["--mount", "type=bind,src=" + str(root / relative) + ",dst=/opt/capability/" + name + ",readonly"]
    for name in ("digital.pdf", "image-only.pdf"):
        require("," not in str(controls / name), "unsupported-mount-path")
        args += ["--mount", "type=bind,src=" + str(controls / name) + ",dst=/input/" + name + ",readonly"]
    for source, target in ((profile_path, "profile.json"), (approval_path, "approval.json")):
        require("," not in str(source), "unsupported-mount-path")
        args += ["--mount", "type=bind,src=" + str(source) + ",dst=/protocol/" + target + ",readonly"]
    # env -i prevents the image or Docker client environment silently adding
    # variables. Every effective session variable is the frozen explicit set.
    args += ["--entrypoint", "/usr/bin/env", profile["image"], "-i"]
    args += [name + "=" + value for name, value in sorted(profile["environment"].items())]
    args += ["/usr/bin/python3", "-B", "/opt/capability/capability_session.py", "--profile", "/protocol/profile.json",
             "--approval", "/protocol/approval.json"]
    return args


def validate_container(raw, container, name, profile, create_args):
    observed = strict_json(raw, 65536)
    require(type(observed) is list and len(observed) == 1, "container-inspect-shape")
    value = observed[0]
    require(value["Id"] == container and value["Name"] == "/" + name and value["Image"] == profile["image"],
            "container-image-or-identity-mismatch")
    host, config = value["HostConfig"], value["Config"]
    expected = {"NetworkMode": "none", "ReadonlyRootfs": True, "Memory": CAPS["memory_bytes"],
                "MemorySwap": CAPS["memory_swap_bytes"], "NanoCpus": 2000000000, "PidsLimit": 256,
                "ShmSize": CAPS["shm_bytes"], "CgroupnsMode": "private", "IpcMode": "private", "Init": True,
                "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges"]}
    require(all(exact_value(host.get(key), wanted) for key, wanted in expected.items()), "observed-container-caps-mismatch")
    require(config["User"] == "1000:1000" and config["Hostname"] == "original-capability"
            and config["Entrypoint"] == ["/usr/bin/env"] and config["WorkingDir"] == "/home/canary", "container-entry-mismatch")
    position = create_args.index(profile["image"])
    require(config["Cmd"] == create_args[position + 1:], "container-command-mismatch")
    tmpfs = {create_args[index + 1].split(":", 1)[0]: create_args[index + 1].split(":", 1)[1]
             for index, argument in enumerate(create_args) if argument == "--tmpfs"}
    require(host["Tmpfs"] == tmpfs and not host.get("Privileged") and not host.get("CapAdd")
            and not host.get("Devices") and not host.get("DeviceRequests") and not host.get("VolumesFrom"),
            "container-writable-mount-mismatch")
    expected_mounts = {}
    for index, argument in enumerate(create_args):
        if argument == "--mount":
            parts = dict(part.split("=", 1) for part in create_args[index + 1].split(",") if "=" in part)
            expected_mounts[parts["dst"]] = parts["src"]
    actual_mounts = {}
    for mount in value["Mounts"]:
        require(mount["Type"] in ("bind", "tmpfs"), "unexpected-container-mount")
        if mount["Type"] == "bind":
            require(mount["Destination"] not in actual_mounts and mount["RW"] is False, "duplicate-or-writable-bind")
            actual_mounts[mount["Destination"]] = mount["Source"]
    require(actual_mounts == expected_mounts, "container-source-delivery-mismatch")
    require(len(host["Ulimits"]) == 2 and exact_value({entry["Name"]: [entry["Soft"], entry["Hard"]] for entry in host["Ulimits"]},
            {"core": [0, 0], "fsize": [67108864, 67108864]}), "container-file-core-limit-mismatch")
    return {"status": "PASS", "scope": "observed-created-container-config-only", "id": container,
            "image": value["Image"], "readonly_delivery": actual_mounts}


def validate_launch_accounting(report, profile, index):
    """A complete receipt can establish zero before launch, including FAIL.

    A merely reported zero cannot erase a host-admitted unknown slot. Match the
    typed count to the entire finite admission/event ledger before adopting it.
    This check establishes accounting only, not page/copy or cleanup success.
    """
    require(type(report) is dict and report.get("protocol") == profile["protocol"]
            and report.get("receipt_complete") is True and exact_value(report.get("session_index"), index),
            "incomplete-launch-accounting-receipt")
    count, admitted, events = report.get("application_launches"), report.get("admitted"), report.get("events")
    require(type(count) is int and count in (0, 1) and type(admitted) is dict and set(admitted) == set(KINDS)
            and all(type(admitted[key]) is int and 0 <= admitted[key] <= cap for key, cap in KINDS.items())
            and type(events) is list and len(events) == sum(admitted.values()), "invalid-launch-accounting-ledger")
    observed = dict.fromkeys(KINDS, 0)
    for ordinal, event in enumerate(events):
        require(type(event) is dict and set(event) == {"index", "kind", "name", "status"}
                and exact_value(event["index"], ordinal) and type(event["kind"]) is str and event["kind"] in KINDS
                and type(event["name"]) is str and len(event["name"]) <= 80
                and event["status"] in ("COMPLETE", "FAIL", "UNKNOWN_AFTER_ADMISSION", "SENT_NO_REPLY"),
                "invalid-launch-accounting-event")
        observed[event["kind"]] += 1
        if event["kind"] == "launcher":
            require(event["name"] == "official-launcher" and event["status"] != "SENT_NO_REPLY", "invalid-launcher-event")
    require(exact_value(observed, admitted) and admitted["launcher"] == count, "launch-count-event-mismatch")
    return count


def validate_collected(root, records, profile, meter):
    payload, _ = read_file(root / "session.json", meter, pin=records["session.json"], limit=RECEIPT_LIMIT)
    report = strict_json(payload)
    require(report.get("protocol") == profile["protocol"] and report.get("receipt_complete") is True,
            "incomplete-session-receipt")
    require(report.get("application_launches") == 1 and type(report.get("application_launches")) is int,
            "invalid-session-launch-accounting")
    require(type(report.get("session_index")) is int and report["session_index"] in (1, 2), "invalid-session-index")
    validate_launch_accounting(report, profile, report["session_index"])
    require(report.get("status") == "CAPABILITIES_OBSERVED_ACTIVE_SETTINGS_UNVERIFIED"
            and exact_value(report.get("vendor_passes"), 0) and report.get("first_failure") is None and not report.get("closing_errors")
            and report.get("process_cleanup", {}).get("status") == "PASS", "session-observation-failed")
    require(exact_value(report.get("counts"), {"pages_attempted":5,"pages_completed":5,"pages_failed":0,"pages_unverified":0,
            "pages_remaining":0,"copy_attempted":2,"copy_completed":2,"copy_failed":0,"copy_unverified":0,"copy_remaining":0}),
            "session-work-accounting-incomplete")
    require(exact_value(report.get("environment_before"), report.get("environment_after"))
            and report.get("environment_before", {}).get("status") == "PASS"
            and exact_value(report.get("environment_before", {}).get("uid_gid"), [1000, 1000])
            and exact_value(report.get("limits_before"), report.get("limits_after"))
            and report.get("limits_before", {}).get("status") == "PASS" and exact_value(report.get("oom_kill_delta"), 0),
            "session-closing-caps-environment-incomplete")
    env = encoded(profile["environment"])
    require(exact_value(report["environment_before"]["identity"], {"size_bytes":len(env),"sha256":hashlib.sha256(env).hexdigest()}),
            "session-observed-environment-pin-mismatch")
    limits = report["limits_before"]
    require(exact_value([limits.get("memory_max_bytes"),limits.get("memory_swap_max_bytes"),limits.get("pids_max"),
                        limits.get("file_limits"),limits.get("core_limits")],
                       [CAPS["memory_bytes"],0,CAPS["pids"],[CAPS["application_file_bytes"]]*2,[0,0]])
            and type(limits.get("cpu_period")) is int and limits["cpu_period"] > 0
            and exact_value(limits.get("cpu_quota"),2*limits["cpu_period"]), "session-observed-cap-fields-mismatch")
    require(set(records) == set(ALLOWLIST), "successful-session-artifact-set")
    ready, _ = read_file(root / "ready", meter, pin=records["ready"], limit=64)
    require(ready == b"capability-receipt-complete\n", "incomplete-session-ready")
    raw, _ = read_file(root / "observations.json", meter, pin=records["observations.json"], limit=512 * 1024)
    observed = strict_json(raw, 512 * 1024)
    require(type(observed) is dict and set(observed) == {"workflow", "discovery"}, "raw-observation-membership")
    observations = observed["workflow"]
    require(type(observations) is list and len(observations) == 7, "raw-observation-count")
    discoveries = observed["discovery"]
    require(type(discoveries) is list and len(discoveries) == len(profile["actions"]["discover"]), "raw-discovery-count")
    for index, (record, step) in enumerate(zip(discoveries, profile["actions"]["discover"])):
        require(type(record) is dict and set(record) == {"step", "owner", "label", "raw_base64"}
                and exact_value(record["step"], index) and exact_value(record["owner"], observations[0]["owner"]), "raw-discovery-owner-or-order")
        try:
            payload = base64.b64decode(record["raw_base64"], validate=True)
        except (ValueError, TypeError):
            raise Refusal("invalid-discovery-base64") from None
        require(record["label"] == discovery_outcome(payload, step)["label"], "raw-discovery-branch-mismatch")
    def decode(observation):
        require(type(observation) is dict and set(observation) == {"serial", "owner", "raw"}, "invalid-raw-observation")
        result = {"serial": observation["serial"], "owner": tuple(observation["owner"])}
        for key, value in observation["raw"].items():
            require(type(value) is dict and set(value) == {"encoding", "data"} and value["encoding"] == "base64",
                    "invalid-raw-encoding")
            try:
                result[key] = base64.b64decode(value["data"], validate=True)
            except (ValueError, TypeError):
                raise Refusal("invalid-raw-base64") from None
        return result
    gate, index = PageGate(profile), 0
    gate.opened("digital.pdf", decode(observations[index]))
    index += 1
    pages = []
    for key in PAGE_KEYS:
        if key == "image-only-1":
            gate.opened("image-only.pdf", decode(observations[index]))
            index += 1
        gate.page(key, decode(observations[index]))
        index += 1
        frame, _ = read_file(root / (key + ".ppm"), meter, pin=records[key + ".ppm"])
        pages.append(verify_page_frame(frame, key, profile["pages"][key], fixtures))
    require([p["page_key"] for p in report["pages"]] == list(PAGE_KEYS)
            and [c["page_key"] for c in report["copies"]] == list(COPY_KEYS), "workflow-count-or-order")
    for copy in report["copies"]:
        data, _ = read_file(root / (copy["page_key"] + ".clipboard"), meter,
                           pin=records[copy["page_key"] + ".clipboard"], limit=profile["clipboard"]["limit_bytes"])
        require(copy["transfer"]["complete"] is True and copy["transfer"]["size_bytes"] == len(data)
                and copy["transfer"]["sha256"] == hashlib.sha256(data).hexdigest(), "clipboard-artifact-mismatch")
        validate_copy(copy, data, profile["clipboard"], str(report["session_index"]) + ":" + copy["page_key"])
    return {"report": report, "pages": pages, "records": records, "discovery_labels": [row["label"] for row in discoveries]}


def attempt(host, index, profile, root, controls, profile_path, approval_path, output):
    name = "original-capability-" + uuid.uuid4().hex
    receipt = {"index": index, "status": "FAIL", "container": name, "container_id": None,
               "launcher_slot": "NOT_ADMITTED", "application_launches": 0, "cleanup": "UNKNOWN", "closing_errors": []}
    container = None
    try:
        # Unique absent name is checked before creation; only our returned
        # verified ID may be removed. An escaped create is never assumed absent.
        absent = host.call(["container", "ls", "--all", "--no-trunc", "--filter", "name=^/" + name + "$", "--format", "{{.ID}}"],
                           stage="name-absence")
        require(not absent["stdout"], "reserved-container-name-exists")
        argv = create_argv(name, profile, root, controls, profile_path, approval_path) + ["--session", str(index)]
        def observed_creation(result):
            nonlocal container
            if (result.get("status") == "PASS" and exact_value(result.get("exit_code"), 0) and result.get("stderr") == b""
                    and result.get("prefix_truncated") is False
                    and type(result.get("stdout")) is bytes and re.fullmatch(rb"[0-9a-f]{64}\n?", result["stdout"])
                    and exact_value(result.get("bytes_read"), {"stdout": len(result["stdout"]), "stderr": 0})):
                container = receipt["container_id"] = result["stdout"].rstrip(b"\n").decode("ascii")
        result = host.call(argv, stage="create-owned-container", observe=observed_creation)
        require(container is not None, "created-container-id-unavailable")
        observed = host.call(["inspect", "--format", "{{.Id}} {{.Name}}", container], stage="created-identity")
        require(observed["stdout"] == (container + " /" + name + "\n").encode("ascii"), "created-identity-mismatch")
        inspected = host.call(["inspect", container], stage="created-isolation-audit")
        receipt["created_audit"] = validate_container(inspected["stdout"], container, name, profile, argv)
        def before_start():
            receipt["launcher_slot"] = "ADMITTED_UNKNOWN_AFTER_START"
            receipt["application_launches"] = None
        host.call(["start", container], stage="start-owned-container", before_effect=before_start)
        # Bounded collection invocation waits for the terminal marker inside
        # the same owned container; no application readiness is inferred.
        wait = ["exec", container, "/usr/bin/python3", "-B", "/opt/capability/capability_collect.py",
                "--wait-seconds", str(profile["seconds"] + COLLECT_WAIT_RESERVE)]
        host.call(wait, stage="terminal-marker-wait", seconds=profile["seconds"] + COLLECT_WAIT_RESERVE + 5)
        transport_path = output / "transport.bin"
        with transport_path.open("xb") as stream:
            sink = CheckedSink(stream, host.meter)
            host.call(["exec", container, "/usr/bin/python3", "-B", "/opt/capability/capability_collect.py", "--produce"],
                      stage="exact-artifact-collection", limit=TRANSPORT_LIMIT, sink=sink)
            stream.flush()
            os.fsync(stream.fileno())
            os.fchmod(stream.fileno(), 0o400)
        artifacts = output / "artifacts"
        artifacts.mkdir(mode=0o700)
        with transport_path.open("rb") as stream:
            before = os.fstat(stream.fileno())
            require(before.st_size <= TRANSPORT_LIMIT, "transport-file-limit")
            records = consume(stream, artifacts, host.meter)
            require(stable(before) == stable(os.fstat(stream.fileno())), "transport-file-changed")
        receipt["records"] = records
        # Retain a failing complete session before refusing the semantic gate.
        raw, _ = read_file(artifacts / "session.json", host.meter, pin=records["session.json"], limit=RECEIPT_LIMIT)
        session = strict_json(raw)
        receipt["reported_application_launches"] = session.get("application_launches")
        receipt["reported_first_failure"] = session.get("first_failure")
        try:
            count = validate_launch_accounting(session, profile, index)
        except Refusal as error:
            receipt["launch_accounting"] = {"status": "UNVERIFIED", "reason": str(error)}
            raise
        receipt["application_launches"] = count
        receipt["launch_accounting"] = {"status": "VERIFIED_COUNT_AND_LEDGER", "application_launches": count}
        receipt["launcher_slot"] = "OBSERVED_COUNT_AND_LEDGER"
        receipt["validated"] = validate_collected(artifacts, records, profile, host.meter)
        receipt["status"] = "CAPABILITY_OBSERVATIONS_COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        host.fail("session-" + str(index), error)
        receipt["first_failure"] = host.first_failure
    finally:
        if container is not None:
            try:
                inspected = host.call(["inspect", container], stage="final-isolation-audit", closing=True)
                receipt["final_audit"] = validate_container(inspected["stdout"], container, name, profile, argv)
            except (Exception, KeyboardInterrupt) as error:
                host.fail("final-isolation-audit", error, closing=True)
                receipt["closing_errors"].append("final-isolation-audit")
            for stage, args in (("remove-owned-id", ["rm", "--force", container]),
                                ("final-owned-id-absence", ["container", "ls", "--all", "--no-trunc", "--filter", "id=" + container,
                                                           "--format", "{{.ID}}"] )):
                try:
                    result = host.call(args, stage=stage, closing=True)
                    if stage.endswith("absence"):
                        require(not result["stdout"], "owned-container-still-present")
                        receipt["cleanup"] = "PASS"
                except (Exception, KeyboardInterrupt) as error:
                    host.fail(stage, error, closing=True)
                    receipt["closing_errors"].append(stage)
            if receipt["closing_errors"]:
                receipt["cleanup"] = "FAIL"
                receipt["status"] = "FAIL"
        else:
            receipt["cleanup"] = "UNVERIFIED_NO_OWNED_ID"
    return receipt


def execute(profile_path, profile_pin, runtime_path, approval_path, root, controls, output, *, command=run_bounded):
    meter = Meter(256 * 1024 ** 2)
    payload, profile_record = read_file(profile_path, meter, pin=profile_pin, limit=RECEIPT_LIMIT, metadata=True)
    actual = {key: profile_record[key] for key in ("size_bytes", "sha256")}
    profile = validate_profile(payload)
    blobs, runtime_records = {}, {}
    for role, name in RUNTIME_FILES.items():
        blobs[role], runtime_records[role] = read_file(runtime_path / name, meter, pin=profile["runtime_view"][role],
                                                     limit=RUNTIME_LIMITS[role], metadata=True)
    runtime_gate = validate_runtime(blobs, profile["runtime_view"], profile)
    del blobs
    approval_raw, approval_record = read_file(approval_path, meter, limit=65536, metadata=True)
    require(exact_value(strict_json(approval_raw), {"status": "APPROVED", "profile": actual, "sessions": 2}), "missing-exact-approval")
    tool_records = {role: read_file(tool["path"], meter, pin=tool["identity"], limit=64 * 1024 ** 2,
                                   retain=False, metadata=True)[1] for role, tool in profile["host_tools"].items()}
    require(sys.executable == profile["host_tools"]["python"]["path"], "unapproved-host-interpreter")
    parent_environment = encoded(dict(os.environ))
    require({"size_bytes": len(parent_environment), "sha256": hashlib.sha256(parent_environment).hexdigest()} == profile["host_environment"],
            "host-environment-mismatch")
    before = audit_inputs(root, controls, profile, meter)
    # New exclusive host namespace is created only after all read-only gates.
    output.mkdir(mode=0o700, parents=False)
    host = Host(2 * profile["seconds"] + HOST_TIME_RESERVE, meter, command)
    report = {"protocol": profile["protocol"], "status": "FAIL", "vendor_passes": 0,
              "application_observations": "NOT_RUN", "historical_launch_outcomes": 12,
              "maximum_new_launches": 2, "maximum_cumulative_launches": 14, "sessions": [], "before_audit": before,
              "active_settings": "UNVERIFIED", "image_compatibility": "NOT_RUN", "text_compatibility": "NOT_RUN"}
    report["runtime_gate"] = runtime_gate
    try:
        for index in (1, 2):
            namespace = output / ("session-" + str(index))
            namespace.mkdir(mode=0o700)
            result = attempt(host, index, profile, root, controls, profile_path, approval_path, namespace)
            report["sessions"].append(result)
            if result["status"] != "CAPABILITY_OBSERVATIONS_COMPLETE" or result["cleanup"] != "PASS":
                break
        if len(report["sessions"]) == 2 and all(s["status"] == "CAPABILITY_OBSERVATIONS_COMPLETE" for s in report["sessions"]):
            first, second = [s["validated"] for s in report["sessions"]]
            require([p["decoded"] for p in first["pages"]] == [p["decoded"] for p in second["pages"]], "two-session-grid-disagreement")
            require([c["transfer"] for c in first["report"]["copies"]] == [c["transfer"] for c in second["report"]["copies"]],
                    "two-session-copy-disagreement")
            require(first["discovery_labels"] == second["discovery_labels"], "two-session-discovery-method-disagreement")
            require(exact_value([s["validated"]["report"]["session_index"] for s in report["sessions"]], [1, 2])
                    and report["sessions"][0]["container"] != report["sessions"][1]["container"], "sessions-not-distinct")
            report["status"] = "TWO_SESSION_CAPABILITIES_OBSERVED_ACTIVE_SETTINGS_UNVERIFIED"
            report["application_observations"] = "OBSERVED_ONLY"
    except (Exception, KeyboardInterrupt) as error:
        host.fail("two-session-comparison", error)
    finally:
        for stage, function, expected in (("source-control-final-audit", lambda: audit_inputs(root, controls, profile, meter), before),
                                ("profile-final-audit", lambda: read_file(profile_path, meter, pin=actual, limit=RECEIPT_LIMIT, metadata=True)[1], profile_record),
                                ("runtime-final-audit", lambda: {role: read_file(runtime_path / name, meter,
                                    pin=profile["runtime_view"][role], limit=RUNTIME_LIMITS[role], retain=False, metadata=True)[1] for role, name in RUNTIME_FILES.items()}, runtime_records),
                                ("approval-final-audit", lambda: read_file(approval_path, meter, limit=65536, metadata=True)[1], approval_record),
                                ("environment-final-audit", lambda: encoded(dict(os.environ)) == parent_environment, True),
                                ("host-tool-final-audit", lambda: {role: read_file(tool["path"], meter, pin=tool["identity"],
                                    limit=64 * 1024 ** 2, retain=False, metadata=True)[1] for role, tool in profile["host_tools"].items()}, tool_records)):
            try:
                report[stage] = function()
                require(exact_value(report[stage], expected), "closing-binding-mismatch")
            except (Exception, KeyboardInterrupt) as error:
                host.fail(stage, error, closing=True)
        report.update(first_failure=host.first_failure, closing_errors=host.closing_errors,
                      actions=host.actions, host_helper_admissions=host.clients, file_io=meter.summary(),
                      sessions_attempted=len(report["sessions"]), sessions_remaining=2 - len(report["sessions"]))
        launches = [session["application_launches"] for session in report["sessions"]]
        report["new_application_launches"] = sum(launches) if all(type(v) is int for v in launches) else None
        report["cumulative_application_launches"] = 12 + report["new_application_launches"] if report["new_application_launches"] is not None else None
        if host.first_failure is not None:
            report["status"] = "FAIL"
        try:
            raw = encoded(report)
            require(len(raw) <= 1024 ** 2, "host-report-limit")
            exclusive(output / "capability-report.json", raw, meter)
        except (Exception, KeyboardInterrupt) as error:
            host.fail("terminal-report-publication", error, closing=True)
            report.update(status="FAIL", first_failure=host.first_failure, closing_errors=host.closing_errors)
            raise PublicationFailure(report) from None
    return report


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print(encoded({"status": "NOT_RUN", "application_launches": 0, "vendor_passes": 0,
                       "historical_launch_outcomes": 12, "maximum_cumulative_launches": 14}).decode(), end="")
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("profile", "runtime-packet", "approval", "source-root", "controls-root", "output"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--profile-sha256")
    parser.add_argument("--profile-size", type=int)
    args = parser.parse_args(argv)
    if all(value is None for value in vars(args).values()):
        print(encoded({"status": "NOT_RUN", "application_launches": 0, "vendor_passes": 0,
                       "historical_launch_outcomes": 12, "maximum_cumulative_launches": 14}).decode(), end="")
        return 0
    try:
        require(all(value is not None for value in vars(args).values()), "incomplete-explicit-admission")
        report = execute(args.profile, {"size_bytes": args.profile_size, "sha256": args.profile_sha256},
                         args.runtime_packet, args.approval, args.source_root.absolute(), args.controls_root.absolute(), args.output.absolute())
        print(encoded({"status": report["status"], "vendor_passes": 0,
                       "new_application_launches": report["new_application_launches"]}).decode(), end="")
        return int(report["status"] == "FAIL")
    except (Exception, KeyboardInterrupt) as error:
        retained = error.report if isinstance(error, PublicationFailure) else {}
        print(encoded({"status": "REFUSED_OR_FAIL", "new_application_launches": retained.get("new_application_launches"),
                       "host_helper_admissions": retained.get("host_helper_admissions"),
                       "sessions_attempted": retained.get("sessions_attempted"), "first_failure": retained.get("first_failure"),
                       "closing_errors": retained.get("closing_errors", []), "vendor_passes": 0,
                       "reason": str(error) if isinstance(error, Refusal) else "result-unavailable"}).decode(), end="")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
