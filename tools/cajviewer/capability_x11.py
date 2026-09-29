# SPDX-License-Identifier: MIT
"""Small bounded X11 core-protocol client, authored from public X.Org specs.

Only the task's private Unix display :99 is supported. No TCP, authority-file
discovery, toolkit loading, or application launch occurs during import.
"""

from __future__ import annotations

from collections import deque
import socket
import struct
import time

from capability_protocol import DISPLAY, Refusal, integer, require


def padding(data):
    return data + b"\0" * (-len(data) % 4)


class X11:
    def __init__(self, *, seconds=2, byte_limit=128 * 1024 ** 2,
                 request_limit=1024, clock=time.monotonic, connection=None, admit=None, deadline=None):
        self.clock, self.seconds = clock, seconds
        self.byte_limit, self.request_limit = byte_limit, request_limit
        self.requested = self.returned = self.sent = self.calls = self.requests = 0
        self.send_submitted = self.send_calls = 0
        self.send_completion = "COMPLETE"
        self.sequence = 0
        self.events, self.atoms = deque(), {}
        self.admit, self.phase_deadline = admit, deadline
        self.transaction_deadline = None
        self.sock = connection if connection is not None else socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            self.sock.settimeout(seconds)
            if connection is None:
                self.sock.connect("/tmp/.X11-unix/X99")
            self.deadline = min(self.clock() + seconds, self.phase_deadline or float("inf"))
            self._send(struct.pack("<BBHHHHH", 108, 0, 11, 0, 0, 0, 0))
            prefix = self._read(8)
            require(prefix[0] == 1 and struct.unpack_from("<H", prefix, 2)[0] == 11, "x11-setup-refused")
            length = struct.unpack_from("<H", prefix, 6)[0] * 4
            require(32 <= length <= 65536, "x11-setup-size")
            self.setup = self._read(length)
            self._parse_setup()
            self.next_resource = 1
        except BaseException:
            self.sock.close()
            raise

    def _parse_setup(self):
        data = self.setup
        self.resource_base, self.resource_mask = struct.unpack_from("<II", data, 4)
        require(self.resource_mask > 0 and self.resource_mask & (self.resource_mask + 1) == 0,
                "unsupported-resource-mask")
        vendor_length, self.max_request = struct.unpack_from("<HH", data, 16)
        screens, formats, order = data[20:23]
        require(screens == 1 and order == 0, "unsupported-x11-screen-or-byte-order")
        self.formats = {}
        position = 32 + (vendor_length + 3) // 4 * 4
        require(position + formats * 8 + 40 <= len(data), "truncated-x11-setup")
        for _ in range(formats):
            depth, bpp, pad = data[position:position + 3]
            self.formats[depth] = (bpp, pad)
            position += 8
        screen = data[position:position + 40]
        self.root = struct.unpack_from("<I", screen)[0]
        width, height, mm_width, mm_height = struct.unpack_from("<HHHH", screen, 20)
        self.visual = struct.unpack_from("<I", screen, 32)[0]
        depth, depth_count = screen[38:40]
        require((width, height) == DISPLAY and depth == 24 and self.formats.get(24) == (32, 32), "unsupported-display-grid")
        position += 40
        masks = None
        for _ in range(depth_count):
            require(position + 8 <= len(data), "truncated-depth-record")
            count = struct.unpack_from("<H", data, position + 2)[0]
            position += 8
            require(count <= 512 and position + count * 24 <= len(data), "visual-record-limit")
            for _ in range(count):
                visual = struct.unpack_from("<I", data, position)[0]
                if visual == self.visual:
                    masks = struct.unpack_from("<III", data, position + 8)
                position += 24
        require(masks == (0xff0000, 0x00ff00, 0x0000ff), "unsupported-display-masks")
        self.display = {"pixels": [width, height], "millimetres": [mm_width, mm_height],
                        "depth": depth, "bits_per_pixel": 32, "byte_order": "LSBFirst",
                        "masks": list(masks), "vendor_bytes_hex": data[32:32 + vendor_length].hex(),
                        "scope": "observed-x-server-only", "vendor_dpr": "UNAVAILABLE"}

    def _remaining(self):
        remaining = self.deadline - self.clock()
        require(remaining > 0, "x11-deadline")
        self.sock.settimeout(remaining)

    def _send(self, data):
        self._remaining()
        view = memoryview(data)
        while view:
            self._remaining()
            offered = min(65536, len(view))
            require(self.send_submitted + offered <= self.byte_limit, "x11-send-budget")
            self.send_submitted += offered
            self.send_calls += 1
            self.send_completion = "UNKNOWN_AFTER_SUBMISSION"
            count = self.sock.send(view[:offered])
            require(type(count) is int and 0 < count <= offered, "x11-invalid-send-progress")
            self.sent += count
            self.send_completion = "COMPLETE"
            view = view[count:]
        require(self.clock() < self.deadline, "x11-send-completion-after-deadline")

    def _read(self, count):
        payload = bytearray()
        while len(payload) < count:
            self._remaining()
            requested = min(65536, count - len(payload))
            require(self.requested + requested <= self.byte_limit, "x11-read-budget")
            self.requested += requested
            self.calls += 1
            chunk = self.sock.recv(requested)
            require(type(chunk) is bytes and 0 < len(chunk) <= requested, "x11-short-or-invalid-read")
            payload.extend(chunk)
            self.returned += len(chunk)
        require(self.clock() < self.deadline, "x11-completion-after-deadline")
        return bytes(payload)

    def _packet(self, max_payload):
        header = self._read(32)
        kind = header[0] & 127
        require(kind != 0, "x11-protocol-error")
        if kind == 1:
            length = struct.unpack_from("<I", header, 4)[0] * 4
            require(length <= max_payload, "x11-reply-size-limit")
            return header, self._read(length) if length else b""
        require(kind != 35, "x11-unexpected-generic-event")
        return header, None

    def request(self, opcode, payload=b"", *, detail=0, reply=False, limit=65536):
        require(self.requests < self.request_limit, "x11-request-limit")
        body = padding(payload)
        require((len(body) + 4) // 4 <= min(self.max_request, 65535), "x11-request-size")
        self.requests += 1
        event = self.admit(opcode) if self.admit is not None else None
        self.sequence = (self.sequence + 1) % 65536
        sequence = self.sequence
        self.deadline = min(self.clock() + self.seconds, self.phase_deadline or float("inf"),
                            self.transaction_deadline or float("inf"))
        self._send(struct.pack("<BBH", opcode, detail, (len(body) + 4) // 4) + body)
        if not reply:
            if event is not None:
                event["status"] = "SENT_NO_REPLY"
            return None
        while True:
            header, data = self._packet(limit)
            if data is not None:
                require(struct.unpack_from("<H", header, 2)[0] == sequence, "x11-reply-sequence")
                if event is not None:
                    event["status"] = "COMPLETE"
                return header, data
            require(len(self.events) < 256, "x11-event-queue-limit")
            self.events.append(header)

    def event(self, predicate, *, deadline):
        self.deadline = min(deadline, self.clock() + self.seconds, self.phase_deadline or float("inf"),
                            self.transaction_deadline or float("inf"))
        for index, event in enumerate(self.events):
            if predicate(event):
                del self.events[index]
                require(self.clock() < self.deadline, "x11-event-after-deadline")
                return event
        while True:
            event, data = self._packet(0)
            require(data is None, "x11-unexpected-reply")
            if predicate(event):
                return event
            require(len(self.events) < 256, "x11-event-queue-limit")
            self.events.append(event)

    def atom(self, name):
        if name not in self.atoms:
            payload = name.encode("ascii", "strict")
            require(0 < len(payload) <= 64, "atom-size-limit")
            header, _ = self.request(16, struct.pack("<HH", len(payload), 0) + payload, reply=True, limit=0)
            self.atoms[name] = struct.unpack_from("<I", header, 8)[0]
        return self.atoms[name]

    def discard_events(self, predicate):
        self.events = deque(e for e in self.events if not predicate(e))

    def window(self):
        require(self.next_resource <= self.resource_mask, "x11-resource-limit")
        window = self.resource_base | self.next_resource
        self.next_resource += 1
        # InputOnly 1x1 unmapped requestor with PropertyChangeMask.
        self.request(1, struct.pack("<IIhhHHHHIII", window, self.root, 0, 0, 1, 1,
                                   0, 2, 0, 1 << 11, 1 << 22))
        return window

    def change(self, window, atom, type_atom, format_bits, data):
        require(format_bits in (8, 32) and len(data) % (format_bits // 8) == 0, "property-data-format")
        self.request(18, struct.pack("<IIIB3xI", window, atom, type_atom, format_bits,
                                     len(data) // (format_bits // 8)) + data)

    def delete(self, window, atom):
        self.request(19, struct.pack("<II", window, atom))

    def property(self, window, atom, *, limit=4096, delete=False):
        header, data = self.request(20, struct.pack("<IIIII", window, atom, 0, 0, (limit + 3) // 4),
                                    detail=int(delete), reply=True, limit=(limit + 3) // 4 * 4)
        format_bits = header[1]
        type_atom, after, count = struct.unpack_from("<III", header, 8)
        require(format_bits in (0, 8, 16, 32) and after == 0, "property-partial-or-format")
        length = count * (format_bits // 8)
        require(length <= limit and len(data) == (length + 3) // 4 * 4, "property-length")
        return type_atom, format_bits, data[:length]

    def owner(self, selection):
        header, _ = self.request(23, struct.pack("<I", selection), reply=True, limit=0)
        return struct.unpack_from("<I", header, 8)[0]

    def set_owner(self, selection, window, timestamp):
        self.request(22, struct.pack("<III", window, selection, timestamp))
        require(self.owner(selection) == window, "selection-owner-not-installed")

    def timestamp(self, window):
        atom = self.atom("CAJ_CAP_TIME")
        self.change(window, atom, self.atom("STRING"), 8, b"")
        event = self.event(lambda e: e[0] & 127 == 28 and struct.unpack_from("<II", e, 4) == (window, atom)
                           and e[16] == 0, deadline=self.clock() + self.seconds)
        return struct.unpack_from("<I", event, 12)[0]

    def convert(self, window, selection, target, property_atom, timestamp, *, deadline):
        self.delete(window, property_atom)
        self.request(24, struct.pack("<IIIII", window, selection, target, property_atom, timestamp))
        event = self.event(lambda e: e[0] & 127 == 31 and
                           struct.unpack_from("<IIII", e, 4) == (timestamp, window, selection, target), deadline=deadline)
        require(struct.unpack_from("<I", event, 20)[0] == property_atom, "selection-conversion-refused")

    def notify(self, event, property_atom):
        # SelectionRequest: time, owner, requestor, selection, target, property.
        timestamp, _, requestor, selection, target, _ = struct.unpack_from("<IIIIII", event, 4)
        notification = struct.pack("<BBHIIIII8x", 31, 0, 0, timestamp, requestor, selection, target, property_atom)
        self.request(25, struct.pack("<II", requestor, 0) + notification)

    def children(self, window):
        header, data = self.request(15, struct.pack("<I", window), reply=True, limit=128 * 4)
        count = struct.unpack_from("<H", header, 16)[0]
        require(count <= 128 and len(data) == count * 4, "window-child-limit")
        return list(struct.unpack("<" + "I" * count, data))

    def viewable(self, window):
        header, data = self.request(3, struct.pack("<I", window), reply=True, limit=12)
        require(len(data) == 12, "window-attribute-length")
        return header[26] == 2 and header[27] == 0

    def geometry(self, window):
        header, _ = self.request(14, struct.pack("<I", window), reply=True, limit=0)
        width, height = struct.unpack_from("<HH", header, 16)
        translated, _ = self.request(40, struct.pack("<IIhh", window, self.root, 0, 0), reply=True, limit=0)
        require(translated[1] == 1, "window-on-other-screen")
        x, y = struct.unpack_from("<hh", translated, 12)
        return x, y, width, height

    def image(self, area):
        x, y, width, height = area
        require(0 <= x < DISPLAY[0] and 0 <= y < DISPLAY[1] and width > 0 and height > 0
                and x + width <= DISPLAY[0] and y + height <= DISPLAY[1], "capture-outside-display")
        header, data = self.request(73, struct.pack("<IhhHHI", self.root, x, y, width, height, 0xffffffff),
                                    detail=2, reply=True, limit=width * height * 4)
        require(header[1] == 24 and struct.unpack_from("<I", header, 8)[0] == self.visual
                and len(data) == width * height * 4, "capture-layout-mismatch")
        rgb = bytearray(width * height * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = data[2::4], data[1::4], data[0::4]
        return bytes(rgb)

    def close(self):
        self.sock.close()

    def accounting(self):
        return {"requests": self.requests, "requested_bytes": self.requested,
                "returned_bytes": self.returned, "sent_bytes": self.sent, "read_calls": self.calls,
                "send_submitted_bytes": self.send_submitted, "send_calls": self.send_calls, "send_completion": self.send_completion,
                "requested_read_limit_bytes": self.byte_limit, "submitted_send_limit_bytes": self.byte_limit,
                "reply_or_event_completion": "per-operation", "persistent_helper_processes": 0}
