# SPDX-License-Identifier: MIT
"""Live bounded CLIPBOARD transactions using X11 and ICCCM selection events.

No xclip subprocess or caller-supplied ownership summary stands in for a live
transfer. TIMESTAMP is a selection-acquisition timestamp, not a text hash.
"""

from __future__ import annotations

import hashlib
import struct

from capability_protocol import Refusal, exact_value, integer, require, text_evidence


def newer(value, baseline):
    """Compare CARD32 server timestamps, including wrap, within half a cycle."""
    return 0 < (value - baseline) % (2 ** 32) < 2 ** 31


def validate_copy(evidence, payload, contract, nonce):
    """Independent host check of retained raw selection event and text bytes."""
    require(type(evidence) is dict and type(payload) is bytes, "missing-copy-evidence")
    sentinel = ("ORIGINAL-CAJ-CAP-SENTINEL:" + nonce).encode("ascii")
    for key in ("sentinel_owner", "owner", "selection", "target_atom", "timestamp_atom", "baseline_revision", "revision"):
        integer(evidence.get(key), 1, 2 ** 32 - 1, "invalid-copy-transaction-number")
    require(evidence["owner"] != evidence["sentinel_owner"] and newer(evidence["revision"], evidence["baseline_revision"]),
            "copy-owner-or-revision-not-fresh")
    try:
        raw_event = bytes.fromhex(evidence["selection_clear_hex"])
    except (ValueError, TypeError, KeyError):
        raise Refusal("missing-raw-selection-clear") from None
    require(len(raw_event) == 32 and raw_event[0] == 29
            and struct.unpack_from("<III", raw_event, 4) ==
            (evidence["revision"], evidence["sentinel_owner"], evidence["selection"]), "copy-raw-selection-event-mismatch")
    require(evidence.get("sentinel") == {"size_bytes": len(sentinel), "sha256": hashlib.sha256(sentinel).hexdigest()}
            and exact_value(evidence.get("selection_clear_timestamp"), evidence["revision"])
            and exact_value(evidence.get("after"), {"owner": evidence["owner"], "revision": evidence["revision"]}),
            "copy-transaction-binding-mismatch")
    targets = evidence.get("targets")
    require(type(targets) is list and 1 <= len(targets) <= 64 and all(type(value) is int and 0 < value < 2 ** 32 for value in targets)
            and len(set(targets)) == len(targets) and evidence["target_atom"] in targets
            and evidence["timestamp_atom"] in targets, "copy-target-list-incomplete")
    transfer = evidence.get("transfer", {})
    require(transfer.get("complete") is True and type(transfer.get("incremental")) is bool
            and type(transfer.get("pieces")) is int and 1 <= transfer["pieces"] <= 129
            and (transfer["pieces"] >= 2 if transfer["incremental"] else transfer["pieces"] == 1)
            and exact_value(transfer.get("size_bytes"), len(payload))
            and transfer.get("sha256") == hashlib.sha256(payload).hexdigest()
            and len(payload) <= contract["limit_bytes"] and payload != sentinel, "copy-transfer-incomplete-or-stale")
    require(evidence.get("origin") == "ordinary-copy" and evidence.get("target") == contract["target"]
            and evidence.get("status") == ("COMPLETE" if payload else "COMPLETE_EMPTY")
            and exact_value(evidence.get("text"), text_evidence(payload, contract["encoding"])), "copy-text-evidence-mismatch")
    return {"status": evidence["status"], "raw": {"size_bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()},
            "text": evidence["text"], "freshness": "RAW_SELECTION_EVENT_AND_COMPLETE_TRANSFER_BOUND"}


class Clipboard:
    def __init__(self, wire, ledger, contract, *, owner_allowed):
        self.wire, self.ledger, self.contract = wire, ledger, contract
        self.owner_allowed = owner_allowed
        self.window = wire.window()
        self.selection = wire.atom("CLIPBOARD")
        self.result_property = wire.atom("CAJ_CAP_RESULT")
        self.target = wire.atom(contract["target"])
        self.incr = wire.atom("INCR")
        self.targets = wire.atom("TARGETS")
        self.timestamp_target = wire.atom("TIMESTAMP")
        self.sentinel = None
        self.installed_timestamp = None
        self.deadline = None
        self.requests_served = 0

    def _serve(self, event):
        require(self.requests_served < 16, "sentinel-request-limit")
        self.requests_served += 1
        timestamp, owner, requestor, selection, target, property_atom = struct.unpack_from("<IIIIII", event, 4)
        require(owner == self.window and selection == self.selection, "foreign-selection-request")
        prop = property_atom or target
        accepted = 0
        if target == self.targets:
            payload = struct.pack("<III", self.targets, self.timestamp_target, self.target)
            self.wire.change(requestor, prop, self.wire.atom("ATOM"), 32, payload)
            accepted = prop
        elif target == self.timestamp_target:
            self.wire.change(requestor, prop, self.wire.atom("INTEGER"), 32,
                             struct.pack("<I", self.installed_timestamp))
            accepted = prop
        elif target == self.target and (timestamp == 0 or not newer(self.installed_timestamp, timestamp)):
            self.wire.change(requestor, prop, self.target, 8, self.sentinel)
            accepted = prop
        self.wire.notify(event, accepted)

    def _event(self, predicate):
        # While owning a sentinel, service actual manager/requestor traffic;
        # only the bounded three read-only target conversions are supported.
        while True:
            event = self.wire.event(lambda e: e[0] & 127 == 30 or predicate(e), deadline=self.deadline)
            if event[0] & 127 == 30:
                self._serve(event)
            else:
                return event

    def _transfer(self, target, *, limit, format_bits, type_atom):
        require(self.wire.clock() < self.deadline, "clipboard-deadline")
        timestamp = self.wire.timestamp(self.window)
        self.wire.delete(self.window, self.result_property)
        self.wire.request(24, struct.pack("<IIIII", self.window, self.selection, target,
                                         self.result_property, timestamp))
        event = self._event(lambda e: e[0] & 127 == 31 and
                            struct.unpack_from("<IIII", e, 4) == (timestamp, self.window, self.selection, target))
        require(struct.unpack_from("<I", event, 20)[0] == self.result_property, "clipboard-target-refused")
        self.wire.discard_events(lambda e: e[0] & 127 == 28 and
                                struct.unpack_from("<II", e, 4) == (self.window, self.result_property))
        actual_type, actual_format, payload = self.wire.property(self.window, self.result_property,
                                                                limit=max(limit, 4), delete=True)
        pieces = 1
        incremental = actual_type == self.incr
        if incremental:
            require(actual_format == 32 and len(payload) == 4, "invalid-incr-announcement")
            # The announcement is a lower bound, not an exact size promise.
            require(struct.unpack("<I", payload)[0] <= limit, "incr-announcement-limit")
            output = bytearray()
            while True:
                require(pieces <= 128 and self.wire.clock() < self.deadline, "incr-event-or-time-limit")
                self._event(lambda e: e[0] & 127 == 28 and
                            struct.unpack_from("<II", e, 4) == (self.window, self.result_property) and e[16] == 0)
                actual_type, actual_format, part = self.wire.property(self.window, self.result_property,
                                                                     limit=min(65536, limit - len(output) + 1), delete=True)
                require(actual_type == type_atom and actual_format == format_bits, "incr-type-changed")
                pieces += 1
                require(len(output) + len(part) <= limit, "clipboard-transfer-limit")
                if not part:
                    payload = bytes(output)
                    break
                output.extend(part)
        else:
            require(actual_type == type_atom and actual_format == format_bits and len(payload) <= limit,
                    "clipboard-type-or-size")
        return payload, {"complete": True, "incremental": incremental, "pieces": pieces,
                         "size_bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}

    def _metadata(self):
        owner = self.wire.owner(self.selection)
        require(owner and self.owner_allowed(owner), "clipboard-owner-unverified")
        targets, _ = self._transfer(self.targets, limit=256, format_bits=32, type_atom=self.wire.atom("ATOM"))
        require(len(targets) % 4 == 0, "invalid-target-list")
        offered = struct.unpack("<" + "I" * (len(targets) // 4), targets)
        require(len(offered) == len(set(offered)) and self.target in offered and self.timestamp_target in offered,
                "clipboard-target-or-revision-unavailable")
        revision, _ = self._transfer(self.timestamp_target, limit=4, format_bits=32, type_atom=self.wire.atom("INTEGER"))
        require(len(revision) == 4 and self.wire.owner(self.selection) == owner, "clipboard-owner-raced")
        return owner, struct.unpack("<I", revision)[0], list(offered)

    def copy(self, nonce, dispatch):
        """Install and read sentinel, dispatch ordinary copy, collect to EOF.

        Persistent manager ownership is deliberately UNAVAILABLE here: no
        demonstrated content-revision contract has yet been frozen. Serving a
        sentinel allows a manager to acquire it, but does not certify that the
        manager's TIMESTAMP represents a later application's copy transaction.
        """
        self.deadline = min(self.ledger.deadline, self.wire.clock() + self.contract["seconds"])
        previous = self.wire.transaction_deadline
        self.wire.transaction_deadline = self.deadline
        try:
            result = self._copy(nonce, dispatch)
            require(self.wire.clock() < self.deadline, "clipboard-completion-after-deadline")
            return result
        finally:
            self.wire.transaction_deadline = previous

    def _copy(self, nonce, dispatch):
        self.sentinel = ("ORIGINAL-CAJ-CAP-SENTINEL:" + nonce).encode("ascii")
        self.installed_timestamp = self.wire.timestamp(self.window)
        self.requests_served = 0
        self.ledger.perform("clipboard", "install-sentinel",
                            lambda: self.wire.set_owner(self.selection, self.window, self.installed_timestamp))
        # Confirm installation by converting from our own live selection, not
        # by trusting the SetSelectionOwner request's apparent success.
        baseline_payload, _ = self._transfer(self.target, limit=256, format_bits=8, type_atom=self.target)
        require(baseline_payload == self.sentinel and self.wire.owner(self.selection) == self.window,
                "sentinel-not-live-or-manager-unavailable")
        self.ledger.perform("clipboard", "ordinary-copy-dispatch", dispatch)
        clear = self._event(lambda e: e[0] == 29 and
                            struct.unpack_from("<II", e, 8) == (self.window, self.selection))
        changed_at = struct.unpack_from("<I", clear, 4)[0]
        require(newer(changed_at, self.installed_timestamp), "clipboard-revision-not-fresh")
        owner, revision, offered = self.ledger.perform("clipboard", "selection-metadata", self._metadata)
        require(owner != self.window and revision == changed_at and newer(revision, self.installed_timestamp),
                "clipboard-transaction-not-bound")
        payload, transfer = self.ledger.perform("clipboard", "selection-payload",
            lambda: self._transfer(self.target, limit=self.contract["limit_bytes"], format_bits=8, type_atom=self.target))
        after_owner, after_revision, _ = self._metadata()
        require((after_owner, after_revision) == (owner, revision), "clipboard-changed-during-transfer")
        require(payload != self.sentinel, "stale-sentinel")
        return payload, {"status": "COMPLETE_EMPTY" if not payload else "COMPLETE", "origin": "ordinary-copy",
                         "sentinel_owner": self.window, "owner": owner, "baseline_revision": self.installed_timestamp,
                         "revision": revision, "selection_clear_timestamp": changed_at, "targets": offered,
                         "selection": self.selection, "target": self.contract["target"], "target_atom": self.target,
                         "timestamp_atom": self.timestamp_target, "selection_clear_hex": clear.hex(),
                         "sentinel": {"size_bytes": len(self.sentinel), "sha256": hashlib.sha256(self.sentinel).hexdigest()},
                         "after": {"owner": after_owner, "revision": after_revision},
                         "transfer": transfer, "manager_revision": "UNAVAILABLE",
                         "text": text_evidence(payload, self.contract["encoding"])}
