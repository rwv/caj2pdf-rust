# SPDX-License-Identifier: MIT
"""Validate actual closed upstream runtime metadata, without any runtime probe.

The upstream producer remains external. This reads only eight explicitly
pinned metadata/code blobs copied to the operational packet; no declared
inventory path, tool path, history path or vendor byte is opened here.
"""

from pathlib import PurePosixPath
import hashlib

from capability_protocol import HEX, RUNTIME_LIMITS, Refusal, exact_value, integer, require, strict_json
from run import validate_source_loads

RUNTIME_FILES = {"receipt": "receipt.json", "plan": "frozen-plan.json", "pending": "pending.json",
                 "token": "token.json", "inventory": "inventory.json", "producer": "producer.py",
                 "root_review": "root-review.json", "independent_review": "independent-review.json"}
CHECKS = ("public_audits", "dynamic_environment", "caps_uid_gid", "owned_cleanup",
          "captured_outputs", "source_loading", "runtime_comparison", "prior_evidence_preserved")
NESTED_COMMANDS = (["dpkg-query", "-W", "-f=${Package}\t${Version}\t${Architecture}\n"],
                   ["fc-list", "--format", "%{file}\t%{family}\t%{style}\n"])
TOOLS = {"python3", "Xvfb", "xdotool", "xclip", "xdpyinfo", "openbox", "fc-list", "dpkg-query"}


def byte_identity(value):
    require(type(value) is dict, "missing-runtime-identity")
    result = {key: value.get(key) for key in ("size_bytes", "sha256")}
    integer(result["size_bytes"], 0, 2 * 1024 ** 3)
    require(type(result["sha256"]) is str and HEX.fullmatch(result["sha256"]), "invalid-runtime-byte-pin")
    return result


def check_closing(value, schema, pins, image):
    require(value.get("schema") == schema and value.get("review_status") == "PASS"
            and value.get("operational_status") == "PASS" and value.get("image_id") == image,
            "runtime-independent-closing-not-pass")
    require(byte_identity(value.get("actual_receipt")) == pins["receipt"], "runtime-review-receipt-binding")
    bindings = value.get("bindings")
    require(type(bindings) is dict and set(bindings) == {"frozen_plan", "pending", "token", "inventory", "producer"},
            "runtime-review-bindings")
    for key, role in (("frozen_plan", "plan"), ("pending", "pending"), ("token", "token"),
                      ("inventory", "inventory"), ("producer", "producer")):
        require(byte_identity(bindings[key]) == pins[role], "runtime-review-binding-mismatch")
    require(type(value.get("checks")) is dict and all(value["checks"].get(key) == "PASS" for key in CHECKS),
            "runtime-independent-checks-not-pass")


def check_nested_helpers(receipt, inventory):
    """Bind the two complete original helper outputs to the parsed inventory.

    The producer retains UTF-8 stdout verbatim in observations. Its captures
    must hash those same bytes, including the trailing newline; no text repair
    or external runtime path is used by this consumer.
    """
    tools, observations = inventory.get("tools"), inventory.get("observations")
    require(type(tools) is dict and set(tools) == TOOLS and type(observations) is dict
            and set(observations) == {"packages", "fontconfig"}, "runtime-tools-or-observations-incomplete")
    for tool in tools.values():
        require(type(tool) is dict and type(tool.get("path")) is str and tool["path"].startswith("/")
                and len(tool["path"]) <= 4096, "runtime-tool-identity")
        identity = byte_identity(tool)
        require(0 < identity["size_bytes"] <= 64 * 1024 ** 2, "runtime-tool-size")
    nested = receipt.get("nested_helpers", {})
    require(nested.get("observation") == "COMPLETE_ENVELOPE"
            and exact_value([nested.get("attempted"), nested.get("spawned"), nested.get("spawn_unknown")], [2, 2, 0]),
            "runtime-nested-helper-counts")
    actions = nested.get("actions")
    require(type(actions) is list and len(actions) == 2, "runtime-nested-helper-membership")
    for index, (action, name) in enumerate(zip(actions, ("packages", "fontconfig")), 1):
        require(exact_value(action.get("ordinal"), index) and exact_value(action.get("argv"), NESTED_COMMANDS[index - 1])
                and action.get("status") == action.get("helper_status") == "PASS" and action.get("spawned") is True
                and exact_value(action.get("exit_code"), 0), "runtime-nested-helper-incomplete")
        require(type(observations[name]) is str, "runtime-observation-encoding")
        raw = observations[name].encode("utf-8", "strict")
        require(len(raw) <= 262144, "runtime-observation-limit")
        captures = action.get("captures", {})
        require(type(captures) is dict and set(captures) == {"stdout", "stderr"}, "runtime-nested-capture-membership")
        for stream, payload in (("stdout", raw), ("stderr", b"")):
            capture = captures[stream]
            require(capture.get("complete") is True and capture.get("hash_scope") == "retained-stream"
                    and byte_identity(capture) == {"size_bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()},
                    "runtime-nested-output-not-complete-or-bound")
        require(exact_value(action.get("bytes_read"), {"stdout": len(raw), "stderr": 0}), "runtime-nested-output-counts")


def validate_runtime(blobs, pins, profile):
    try:
        require(type(blobs) is dict and type(pins) is dict and set(blobs) == set(pins) == set(RUNTIME_FILES), "runtime-packet-membership")
        for role, payload in blobs.items():
            require(type(payload) is bytes and {"size_bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()} == pins[role],
                    "runtime-blob-pin-mismatch")
        return _validate_runtime(blobs, pins, profile)
    except (KeyError, TypeError, ValueError, IndexError, AttributeError):
        raise Refusal("malformed-runtime-packet") from None


def _validate_runtime(blobs, pins, profile):
    require(set(blobs) == set(pins) == set(RUNTIME_FILES), "runtime-packet-membership")
    values = {key: strict_json(raw, RUNTIME_LIMITS[key], max_nodes=50000 if key == "inventory" else 12000)
              for key, raw in blobs.items() if key != "producer"}
    r, plan, pending, token, inventory = [values[key] for key in ("receipt", "plan", "pending", "token", "inventory")]
    require(r.get("schema") == "cajviewer-public-runtime-inventory-preparation/1" and r.get("status") == "PASS"
            and r.get("scope") == "public-runtime-inventory-only", "runtime-view-not-pass")
    require(exact_value([r.get("inventory_attempts"), r.get("inventory_completed"), r.get("app_launches"), r.get("vendor_passes")],
                        [1, 1, 0, 0]), "runtime-attempt-counts")
    actions = r.get("actions")
    stages = ("name-absent", "create-one-inventory-container", "one-inventory-start-attach", "remove-owned-container", "name-final-absent")
    require(type(actions) is list and len(actions) == 5 and
            exact_value([r.get("docker_launch_attempts"), r.get("docker_successful_spawns")], [5, 5]), "runtime-action-counts")
    for index, action in enumerate(actions):
        require(exact_value(action.get("ordinal"), index + 1) and action.get("stage") == stages[index]
                and action.get("status") == action.get("helper_status") == "PASS" and action.get("spawned") is True
                and exact_value(action.get("exit_code"), 0), "runtime-action-incomplete")
        for stream in ("stdout", "stderr"):
            capture = action.get(stream, {})
            require(capture.get("complete") is True and capture.get("hash_scope") == "retained-stream"
                    and exact_value(action.get("bytes_read", {}).get(stream), byte_identity(capture)["size_bytes"]),
                    "runtime-prefix-output")
        require(action["stderr"]["size_bytes"] == 0, "runtime-helper-stderr")
    require(plan.get("schema") == r["schema"] and plan.get("status") == "FROZEN" and plan.get("image_id") == profile["image"],
            "runtime-frozen-plan-binding")
    for key, role in (("plan", "plan"), ("pending", "pending"), ("token", "token"), ("inventory", "inventory")):
        require(byte_identity(r.get(key)) == pins[role], "runtime-receipt-input-binding")
    require(byte_identity(plan.get("wrapper")) == pins["producer"], "runtime-producer-binding")
    require(byte_identity(pending.get("plan")) == pins["plan"] and token.get("decision") == "APPROVE"
            and token.get("root_review") is True and token.get("independent_review") is True,
            "runtime-consumed-token-binding")
    require(type(pending.get("pid")) is int and pending["pid"] > 0 and exact_value(token.get("pid"), pending["pid"])
            and byte_identity(token.get("plan")) == pins["plan"] and byte_identity(token.get("pending_preflight")) == pins["pending"]
            and token.get("parent_environment") == pending.get("parent_environment_identity")
            and token.get("child_environment") == pending.get("child_environment_identity"), "runtime-same-pid-token-binding")
    require(exact_value(r.get("comparison"), {"status": "PASS", "records_planned": 2731, "records_attempted": 2731,
            "records_passed": 2731, "records_failed": 0, "records_remaining": 0, "tools": "PASS",
            "packages_fonts": "PASS", "declared_byte_total": "PASS"}), "runtime-comparison-incomplete")
    loading = r.get("public_source_loading", {})
    verified = validate_source_loads(loading.get("records"))
    require(exact_value(loading, verified) and verified["records_completed"] == 2, "runtime-source-loading-incomplete")
    require(profile["runtime_sources"] == {row["path"].rsplit("/", 1)[-1]: row["actual_identity"]
                                           for row in verified["records"]}, "runtime-source-pin-mismatch")
    require(r.get("before_audit") == r.get("after_audit") == r.get("output_integrity") == "PASS"
            and r.get("container_creation") == "VERIFIED_CREATED" and r.get("cleanup") == "REMOVE_COMMAND_PASS"
            and r.get("final_container_absence") == "PASS", "runtime-closing-not-pass")
    records = [*plan["history"], *plan["public_dependencies"], plan["wrapper"], plan["inline_entry"],
               plan["protocol_doc"], *plan["tools"].values(), plan["public_inventory_transport"]["source"]]
    records += [{**record, "path": str(PurePosixPath(plan["source_root"]) / record["path"])} for record in plan["sources"]]
    require(1 <= len(records) <= 128 and len({row["path"] for row in records}) == len(records), "runtime-audit-declarations")
    audits = r.get("file_audits")
    require(type(audits) is list and len(audits) == 3 * len(records), "runtime-three-audits-required")
    index = 0
    for phase in ("public-preflight", "post-token-before-Docker", "final"):
        for expected in records:
            row = audits[index]
            require(row.get("phase") == phase and row.get("status") == "PASS" and row.get("path") == expected["path"]
                    and exact_value(row.get("ordinal"), index + 1)
                    and byte_identity(row.get("identity")) == byte_identity(expected), "runtime-audit-order-or-pin")
            index += 1
    environment = r.get("container_environment", {})
    require(exact_value(environment.get("uid_gid"), [1000, 1000]) and exact_value(environment.get("uid_gid_after"), [1000, 1000])
            and exact_value(environment.get("closing_audits"), {"caps": "PASS", "environment": "PASS", "user": "PASS"})
            and environment.get("closing_failures") == []
            and environment.get("environment_identity") == environment.get("environment_after_identity"), "runtime-environment-closing")
    byte_identity(environment.get("environment_identity"))
    for caps in r.get("container_caps", {}).values():
        require(exact_value([caps.get("memory_max_bytes"), caps.get("memory_swap_max_bytes"), caps.get("pids_max")],
                            [512 * 1024 ** 2, 0, 64]) and type(caps.get("cpu_period")) is int and caps["cpu_period"] > 0
                and exact_value(caps.get("cpu_quota"), 2 * caps["cpu_period"])
                and type(caps.get("memory_peak_bytes")) is int and 0 <= caps["memory_peak_bytes"] <= 512 * 1024 ** 2
                and type(caps.get("pids_peak")) is int and 0 <= caps["pids_peak"] <= 64
                and exact_value([caps["memory_events"].get("oom"), caps["memory_events"].get("oom_kill")], [0, 0]),
                "runtime-observed-caps")
    require(set(r.get("container_caps", {})) == {"caps_before", "caps_after"}, "missing-runtime-cap-closing")
    closing = r.get("closing_checks", [])
    require(type(closing) is list and len(closing) == 4 and all(row.get("status") == "PASS" for row in closing)
            and [row.get("kind") for row in closing] == ["dynamic-pin"] * 3 + ["host-environment"], "runtime-dynamic-closing")
    require(exact_value(r.get("output_audit_counts"), {"planned": 4, "attempted": 4, "passed": 4, "failed": 0, "remaining": 0})
            and len(r.get("output_audits", [])) == 4 and all(row.get("status") == "PASS" for row in r["output_audits"]),
            "runtime-captured-outputs-incomplete")
    outputs = r.get("retained_output_expectations")
    require(type(outputs) is list and len(outputs) == 4 and len({row["file"] for row in outputs}) == 4,
            "runtime-retained-output-membership")
    inventory_bound = 0
    for expected, observed in zip(outputs, r["output_audits"]):
        require(type(expected.get("file")) is str and len(expected["file"]) <= 80
                and PurePosixPath(expected["file"]).name == expected["file"] and expected["file"] not in ("", ".", "..")
                and type(expected.get("device")) is int and expected["device"] >= 0
                and type(expected.get("inode")) is int and expected["inode"] > 0 and exact_value(expected.get("mode"), 0o400)
                and observed.get("file") == expected["file"] and byte_identity(observed.get("identity")) == byte_identity(expected),
                "runtime-retained-output-identity-or-order")
        inventory_bound += byte_identity(expected) == pins["inventory"]
    require(inventory_bound == 1, "runtime-underlying-inventory-output-not-bound")
    require(inventory.get("status") == "PASS" and inventory.get("scope") == "opaque-runtime-inventory-only"
            and exact_value([inventory.get("app_launches"), inventory.get("vendor_passes")], [0, 0]), "runtime-inventory-framing")
    check_nested_helpers(r, inventory)
    entries = inventory.get("files", [])
    require(type(entries) is list and len(entries) == 2731
            and [row["path"] for row in entries] == sorted({row["path"] for row in entries}), "runtime-inventory-membership")
    total = 0
    for entry in entries:
        require(type(entry["path"]) is str and entry["path"].startswith("/") and len(entry["path"]) <= 4096
                and type(entry["mode"]) is int and 0 <= entry["mode"] <= 0o7777, "runtime-inventory-record")
        kind = entry.get("type")
        require(kind in ("directory", "symlink", "opaque-file"), "runtime-inventory-type")
        if kind == "opaque-file":
            total += byte_identity(entry)["size_bytes"]
        elif kind == "symlink":
            require(type(entry.get("target")) is str and len(entry["target"]) <= 4096, "runtime-inventory-link")
    require(exact_value(inventory.get("files_hashed_bytes"), total) and total <= 4 * 1024 ** 3, "runtime-inventory-byte-total")
    for key, schema in (("root_review", "cajviewer-root-v12-operational-closing/1"),
                        ("independent_review", "cajviewer-independent-v12-operational-closing/1")):
        check_closing(values[key], schema, pins, profile["image"])
    return {"status": "PASS", "scope": "closed-pinned-runtime-view-only", "image": profile["image"],
            "receipt": pins["receipt"], "root_review": pins["root_review"], "independent_review": pins["independent_review"],
            "new_capability_delivery": "SEPARATE_SOURCE_AND_CONTAINER_GUARDS_REQUIRED"}
