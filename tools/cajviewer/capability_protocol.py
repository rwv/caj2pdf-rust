# SPDX-License-Identifier: MIT
"""Finite original-PDF capability contract; importing this performs no I/O.

An approved contract describes observations to make, never their outcome.
The desktop adapter supplies raw bytes to these independent validators.
"""

from __future__ import annotations

import hashlib
import json
import re
import threading
import time

MIB = 1024 ** 2
FILE_LIMIT = 6 * MIB
CONTENT_LIMIT = 32 * MIB
TRANSPORT_LIMIT = 40 * MIB
RECEIPT_LIMIT = 256 * 1024
AUX_LIMIT = 4 * MIB
COLLECT_WAIT_RESERVE = 60
HOST_TIME_RESERVE = 240
DISPLAY = (1600, 1200)
PAGE_KEYS = ("digital-1", "digital-2", "digital-3", "digital-4", "image-only-1")
COPY_KEYS = ("digital-1", "image-only-1")
CAPS = {"memory_bytes": 1536 * MIB, "memory_swap_bytes": 1536 * MIB,
        "cpus": "2", "pids": 256, "shm_bytes": 64 * MIB,
        "home_bytes": 64 * MIB, "tmp_bytes": 64 * MIB,
        "runtime_bytes": 8 * MIB, "output_bytes": CONTENT_LIMIT,
        "application_file_bytes": 64 * MIB, "core_bytes": 0}
KINDS = {"launcher": 1, "persistent-helper": 2, "gui": 48, "query": 1200,
         "clipboard": 32, "capture": 5, "discovery": 12, "log-drainer": 3, "closing": 32}
RUNTIME_LIMITS = {"receipt": MIB, "plan": MIB, "pending": MIB, "token": 65536,
                  "inventory": 4 * MIB, "producer": 65536, "root_review": RECEIPT_LIMIT,
                  "independent_review": RECEIPT_LIMIT}
HEX = re.compile(r"[0-9a-f]{64}\Z")


class Refusal(Exception):
    """Fixed, safe reason for failure, with no unbounded exception text."""


class PublicationFailure(Refusal):
    def __init__(self, report):
        super().__init__("terminal-report-publication-failed")
        self.report = report


def require(condition, reason):
    if not condition:
        raise Refusal(reason)


def fields(value, expected, reason="invalid-fields"):
    require(type(value) is dict and set(value) == set(expected), reason)


def integer(value, low, high, reason="invalid-integer"):
    require(type(value) is int and low <= value <= high, reason)
    return value


def identity(value):
    fields(value, ("size_bytes", "sha256"))
    integer(value["size_bytes"], 1, 2 * 1024 ** 3)
    require(type(value["sha256"]) is str and HEX.fullmatch(value["sha256"]), "invalid-pin")
    return value


def exact_value(value, expected):
    if type(value) is not type(expected):
        return False
    if type(expected) is dict:
        return set(value) == set(expected) and all(exact_value(value[k], child) for k, child in expected.items())
    if type(expected) is list:
        return len(value) == len(expected) and all(exact_value(a, b) for a, b in zip(value, expected))
    return value == expected


def strict_json(payload, limit=RECEIPT_LIMIT, *, max_nodes=12000):
    require(type(max_nodes) is int and 1 <= max_nodes <= 50000, "json-node-cap")
    require(type(payload) is bytes and len(payload) <= limit, "json-size-limit")
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate-json-key")
            result[key] = value
        return result
    try:
        value = json.loads(payload.decode("utf-8", "strict"), object_pairs_hook=pairs,
                           parse_constant=lambda _: (_ for _ in ()).throw(Refusal("nonfinite-json")))
    except (ValueError, UnicodeError, RecursionError):
        raise Refusal("malformed-json") from None
    # The byte limit also bounds strings; this explicit tree bound avoids a
    # deeply nested or enormous node contract despite small serialized size.
    pending, count = [(value, 0)], 0
    while pending:
        item, depth = pending.pop()
        count += 1
        require(count <= max_nodes and depth <= 12, "json-tree-limit")
        if type(item) is dict:
            pending.extend((child, depth + 1) for child in item.values())
        elif type(item) is list:
            pending.extend((child, depth + 1) for child in item)
    return value


def rect(value):
    require(type(value) is list and len(value) == 4, "invalid-rectangle")
    x, y, width, height = value
    integer(x, 0, DISPLAY[0] - 1)
    integer(y, 0, DISPLAY[1] - 1)
    integer(width, 1, DISPLAY[0])
    integer(height, 1, DISPLAY[1])
    require(x + width <= DISPLAY[0] and y + height <= DISPLAY[1], "rectangle-outside-display")
    return tuple(value)


def validate_action(value):
    fields(value, ("kind", "value", "target"))
    require(value["target"] in ("main", "open-dialog"), "unapproved-gui-target")
    kind, argument = value["kind"], value["value"]
    if kind == "key":
        require(type(argument) is str and re.fullmatch(r"[A-Za-z0-9_+]{1,64}", argument), "invalid-key")
    elif kind in ("click", "move"):
        require(type(argument) is list and len(argument) == 2, "invalid-pointer")
        integer(argument[0], 0, DISPLAY[0] - 1)
        integer(argument[1], 0, DISPLAY[1] - 1)
    elif kind == "type-path":
        require(argument in ("/input/digital.pdf", "/input/image-only.pdf"), "unapproved-open-path")
    else:
        raise Refusal("unapproved-action")


def validate_binding(value, tokens=()):
    """One raw X11 property binding or an exact, independently pinned ROI.

    Bitmap bytes are acquisition evidence, not OCR. Their semantic association
    must be reviewed before freezing the profile; no title-only fallback exists.
    """
    require(type(value) is dict and value.get("kind") in ("property", "bitmap"), "unavailable-binding")
    if value["kind"] == "property":
        fields(value, ("kind", "atom", "encoding", "template"))
        require(value["encoding"] in ("utf-8", "ascii"), "unsupported-property-encoding")
        require(type(value["atom"]) is str and re.fullmatch(r"[A-Za-z0-9_]{1,64}", value["atom"]), "invalid-atom")
        # A literal complete match, not arbitrary caller regular expressions.
    else:
        fields(value, ("kind", "rect", "cell", "glyphs", "template"))
        area = rect(value["rect"])
        require(type(value["cell"]) is list and len(value["cell"]) == 2, "invalid-glyph-cell")
        width = integer(value["cell"][0], 1, 32)
        height = integer(value["cell"][1], 1, 64)
        require(area[2] % width == 0 and area[3] == height and area[2] // width <= 256, "bitmap-text-layout")
        require(type(value["glyphs"]) is dict and 1 <= len(value["glyphs"]) <= 128, "glyph-count-limit")
        pins = set()
        for character, pin in value["glyphs"].items():
            require(type(character) is str and len(character) == 1 and ord(character) >= 32, "invalid-glyph-character")
            identity(pin)
            require(pin["size_bytes"] == width * height * 3 and pin["sha256"] not in pins, "ambiguous-glyph-contract")
            pins.add(pin["sha256"])
    template = value["template"]
    require(type(template) is str and 0 < len(template) <= 256, "invalid-observation-template")
    require(all("{" + token + "}" in template for token in tokens), "missing-source-semantic-token")
    stripped = template
    for token in ("document", "page", "count", "width", "height"):
        stripped = stripped.replace("{" + token + "}", "")
    require("{" not in stripped and "}" not in stripped, "unapproved-template-token")


def validate_profile(payload):
    p = strict_json(payload)
    fields(p, ("protocol", "status", "review", "image", "runtime_view", "runtime_sources", "sources", "controls", "host_tools", "host_environment",
               "environment", "caps", "seconds", "bindings", "actions", "pages", "settings", "clipboard"))
    require(p["protocol"] == "original-pdf-capabilities-v1" and p["status"] == "FROZEN", "profile-not-frozen")
    fields(p["review"], ("root", "independent"))
    require(all(type(v) is str and HEX.fullmatch(v) for v in p["review"].values()), "missing-review-pins")
    require(type(p["image"]) is str and re.fullmatch(r"sha256:[0-9a-f]{64}", p["image"]), "image-not-pinned")
    fields(p["runtime_view"], ("receipt", "plan", "pending", "token", "inventory", "producer", "root_review", "independent_review"))
    for role, pin in p["runtime_view"].items():
        identity(pin)
        require(pin["size_bytes"] <= RUNTIME_LIMITS[role], "runtime-metadata-role-limit")
    identity(p["host_environment"])
    fields(p["host_tools"], ("docker", "python"))
    for role in ("docker", "python"):
        fields(p["host_tools"][role], ("path", "identity"))
        require(p["host_tools"][role]["path"] == ("/usr/bin/docker" if role == "docker" else "/usr/bin/python3.13"), "unapproved-host-tool")
        identity(p["host_tools"][role]["identity"])
    require(exact_value(p["caps"], CAPS), "caps-amendment-required")
    integer(p["seconds"], 30, 600)
    for key in ("sources", "runtime_sources", "controls"):
        require(type(p[key]) is dict and 1 <= len(p[key]) <= 16, "invalid-pin-set")
        for name, pin in p[key].items():
            require(type(name) is str and re.fullmatch(r"[A-Za-z0-9_.-]{1,80}", name), "invalid-pin-name")
            identity(pin)
    require(set(p["controls"]) == {"digital.pdf", "image-only.pdf"}, "control-set-mismatch")
    require(type(p["environment"]) is dict and 1 <= len(p["environment"]) <= 24, "invalid-environment")
    for key, value in p["environment"].items():
        require(re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", key) and type(value) is str
                and len(value) <= 256 and "\0" not in value, "invalid-environment")
    require(p["environment"].get("DISPLAY") == ":99" and p["environment"].get("HOME") == "/home/canary"
            and p["environment"].get("XDG_RUNTIME_DIR") == "/runtime", "environment-outside-isolation")
    fields(p["bindings"], ("window_class", "open_dialog", "documents", "pages", "about"))
    require(type(p["bindings"]["window_class"]) is str and 0 < len(p["bindings"]["window_class"]) <= 128,
            "unavailable-window-class")
    fields(p["bindings"]["open_dialog"], ("class", "role", "marker"))
    require(all(type(p["bindings"]["open_dialog"][k]) is str and
                0 < len(p["bindings"]["open_dialog"][k]) <= 128 for k in ("class", "role")),
            "unavailable-open-dialog-identity")
    validate_binding(p["bindings"]["open_dialog"]["marker"])
    fields(p["bindings"]["documents"], ("digital.pdf", "image-only.pdf"))
    fields(p["bindings"]["pages"], PAGE_KEYS)
    for category, bindings in (("documents", p["bindings"]["documents"]), ("pages", p["bindings"]["pages"])):
        for binding in bindings.values():
            fields(binding, ("document", "count", "extent") if category == "documents" else ("document", "page", "count", "extent"))
            for key, observation in binding.items():
                validate_binding(observation, {"document": ("document",), "page": ("page",),
                                              "count": ("count",), "extent": ("width", "height")}[key])
    # About can be unavailable; it never becomes an inferred active build.
    require(p["bindings"]["about"] is None or type(p["bindings"]["about"]) is dict, "invalid-about-binding")
    if p["bindings"]["about"] is not None:
        validate_binding(p["bindings"]["about"])
    fields(p["actions"], ("discover", "open-image", "copy", "navigate", "fit", "about"))
    fields(p["actions"]["navigate"], PAGE_KEYS)
    groups = [p["actions"][k] for k in ("open-image", "copy", "fit", "about")]
    groups += list(p["actions"]["navigate"].values())
    for group in groups:
        require(type(group) is list and len(group) <= 8, "action-count-limit")
        for action in group:
            validate_action(action)
    discovery = p["actions"]["discover"]
    require(type(discovery) is list and len(discovery) <= 6 and p["actions"]["copy"] and p["actions"]["open-image"],
            "unavailable-control-contract")
    discovery_count, discovery_bytes = 0, 0
    for step in discovery:
        fields(step, ("actions", "binding", "outcomes"))
        require(type(step["actions"]) is list and len(step["actions"]) <= 2, "discovery-action-limit")
        for action in step["actions"]:
            validate_action(action)
        validate_binding(step["binding"])
        require(type(step["outcomes"]) is list and 1 <= len(step["outcomes"]) <= 2, "discovery-branch-limit")
        labels, texts = set(), set()
        for outcome in step["outcomes"]:
            fields(outcome, ("label", "text", "actions"))
            require(type(outcome["label"]) is str and re.fullmatch(r"[A-Za-z0-9_-]{1,32}", outcome["label"])
                    and outcome["label"] not in labels and type(outcome["text"]) is str
                    and 0 < len(outcome["text"]) <= 256 and "{" not in outcome["text"] and "}" not in outcome["text"]
                    and outcome["text"] not in texts, "ambiguous-discovery-outcome")
            labels.add(outcome["label"]); texts.add(outcome["text"])
            require(type(outcome["actions"]) is list and len(outcome["actions"]) <= 2, "discovery-selection-limit")
            for action in outcome["actions"]:
                validate_action(action)
        require(step["binding"]["template"] in texts, "discovery-binding-outcome-mismatch")
        discovery_count += len(step["actions"]) + max(len(outcome["actions"]) for outcome in step["outcomes"])
        binding = step["binding"]
        discovery_bytes += 4096 if binding["kind"] == "property" else binding["rect"][2] * binding["rect"][3] * 3
    require(discovery_count <= KINDS["discovery"], "whole-discovery-action-budget")
    gui_count = (sum(len(group) for group in p["actions"]["navigate"].values()) + 5 * len(p["actions"]["fit"])
                 + 2 * len(p["actions"]["copy"]) + len(p["actions"]["open-image"]) + len(p["actions"]["about"]))
    require(gui_count <= KINDS["gui"], "whole-workflow-gui-budget")
    raw_upper = sum(4096 if b["kind"] == "property" else b["rect"][2] * b["rect"][3] * 3
                    for group in list(p["bindings"]["documents"].values()) + list(p["bindings"]["pages"].values())
                    for b in group.values())
    require(raw_upper + discovery_bytes <= 320 * 1024, "whole-workflow-observation-budget")
    fields(p["pages"], PAGE_KEYS)
    for key, page in p["pages"].items():
        fields(page, ("rect", "grid", "points", "rotation", "origin", "outside_rgb"))
        _, _, width, height = rect(page["rect"])
        require(exact_value(page["grid"], [width, height]) and page["origin"] == "complete-page-capture", "unsupported-acquisition-contract")
        expected_points = ["2052", "1538"] if key == "digital-4" else ["256.5", "192.25"]
        require(page["points"] == expected_points and exact_value(page["rotation"], 90 if key == "digital-3" else 0), "physical-page-contract")
        require((width, height) == ((769, 1026) if key == "digital-3" else (1026, 769)), "original-grid-contract")
        require(type(page["outside_rgb"]) is list and len(page["outside_rgb"]) == 3
                and all(type(v) is int and 0 <= v <= 255 for v in page["outside_rgb"])
                and page["outside_rgb"] != [255, 255, 255], "unavailable-page-boundary-contract")
    fields(p["settings"], ("display", "layout", "color", "font", "print"))
    require(exact_value(p["settings"]["display"], [1600, 1200, 24, 96]), "display-contract-mismatch")
    require(all(type(p["settings"][k]) is str and 0 < len(p["settings"][k]) <= 256
                for k in ("layout", "color", "font", "print")), "unavailable-settings-contract")
    fields(p["clipboard"], ("target", "encoding", "limit_bytes", "seconds", "manager_revision"))
    require(p["clipboard"]["target"] in ("UTF8_STRING", "STRING"), "unsupported-text-target")
    require(p["clipboard"]["encoding"] == ("utf-8" if p["clipboard"]["target"] == "UTF8_STRING" else "latin-1"), "target-encoding-mismatch")
    integer(p["clipboard"]["limit_bytes"], 1, 1024 * 1024)
    integer(p["clipboard"]["seconds"], 1, 10)
    require(p["clipboard"]["manager_revision"] == "TIMESTAMP", "unavailable-clipboard-revision")
    return p


class Ledger:
    """Admission precedes every effect. Closing has a separate reserved budget."""

    def __init__(self, seconds, clock=time.monotonic):
        self.clock = clock
        self.started = clock()
        self.deadline = self.started + seconds
        self.events = []
        self.counts = {kind: 0 for kind in KINDS}
        self.first_failure = None
        self.closing_errors = []
        self.lock = threading.Lock()

    def admit(self, kind, name):
        require(kind in KINDS and type(name) is str and len(name) <= 80, "invalid-admission")
        require(kind == "closing" or self.clock() < self.deadline, "phase-deadline")
        require(self.counts[kind] < KINDS[kind], "admission-limit-" + kind)
        self.counts[kind] += 1
        event = {"index": len(self.events), "kind": kind, "name": name, "status": "UNKNOWN_AFTER_ADMISSION"}
        self.events.append(event)
        return event

    def perform(self, kind, name, function):
        event = self.admit(kind, name)
        try:
            result = function()
            event["status"] = "COMPLETE"
            return result
        except (Exception, KeyboardInterrupt) as error:
            event["status"] = "FAIL" if isinstance(error, Refusal) else "UNKNOWN_AFTER_ADMISSION"
            self.fail(name, error, closing=kind == "closing")
            raise

    def fail(self, stage, error, closing=False):
        reason = str(error) if isinstance(error, Refusal) else "operation-result-unavailable"
        item = {"stage": stage, "reason": reason, "error_type": type(error).__name__}
        with self.lock:
            if closing:
                self.closing_errors.append(item)
            if self.first_failure is None:
                self.first_failure = item

    def summary(self):
        return {"admitted": dict(self.counts), "events": self.events,
                "first_failure": self.first_failure, "closing_errors": self.closing_errors}


def validate_observation(raw, binding, values=None):
    require(type(raw) is bytes, "observation-not-bytes")
    if binding["kind"] == "property":
        require(len(raw) <= 4096, "property-size-limit")
        try:
            text = raw.decode(binding["encoding"], "strict")
        except UnicodeError:
            raise Refusal("property-encoding-failure") from None
    else:
        _, _, width, height = binding["rect"]
        cell_width, cell_height = binding["cell"]
        require(len(raw) == width * height * 3, "bitmap-observation-length")
        by_pin = {pin["sha256"]: character for character, pin in binding["glyphs"].items()}
        decoded = []
        for column in range(width // cell_width):
            glyph = b"".join(raw[(row * width + column * cell_width) * 3:
                                 (row * width + (column + 1) * cell_width) * 3] for row in range(cell_height))
            character = by_pin.get(hashlib.sha256(glyph).hexdigest())
            require(character is not None, "unknown-bitmap-glyph")
            decoded.append(character)
        text = "".join(decoded)
    expected = binding["template"]
    for key, value in (values or {}).items():
        expected = expected.replace("{" + key + "}", str(value))
    require("{" not in expected and text == expected, "desktop-observation-mismatch")
    return {"size_bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest(), "decoded_text": text,
            "decoding": "exact-public-desktop-binding"}


def discovery_outcome(raw, step):
    matches = []
    for outcome in step["outcomes"]:
        try:
            validate_observation(raw, {**step["binding"], "template": outcome["text"]})
            matches.append(outcome)
        except Refusal:
            pass
    require(len(matches) == 1, "discovery-outcome-unavailable")
    return matches[0]


def text_evidence(payload, encoding):
    try:
        text = payload.decode(encoding, "strict")
    except UnicodeError:
        return {"encoding": encoding, "decode": "FAIL", "codepoints": None, "unicode_origin": "UNVERIFIED"}
    expected = "RUST 0123456789 中口一RUST 123987 中口一"
    # Report raw order and coverage without repairing whitespace or content.
    ordered = hashlib.sha256()
    for character in text:
        ordered.update(ord(character).to_bytes(4, "big"))
    return {"encoding": encoding, "decode": "PASS", "text_prefix": text[:256],
            "codepoint_prefix": [ord(c) for c in text[:256]], "prefix_complete": len(text) <= 256,
            "codepoint_count": len(text), "ordered_utf32be_sha256": ordered.hexdigest(),
            "full_raw_payload": "retained-exact-clipboard-artifact", "coverage": {c: text.count(c) for c in dict.fromkeys(expected)},
            "exact_expected": text == expected, "unicode_origin": "UNVERIFIED"}
