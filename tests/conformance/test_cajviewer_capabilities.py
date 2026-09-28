# SPDX-License-Identifier: MIT
"""Original mandatory capability controls: no desktop, Docker or vendor calls.

Synthetic GUI glyph/property contracts and X11 replies are deliberately not an
operational profile. Tests invoke the production parsers/adapters/state machine.
"""

import base64
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "tools/cajviewer"))
import capability_protocol as P
import capability_io as IO
import capability_collect as COLLECT
import capability_x11 as X
import capability_clipboard as C
import capability_pages as G
import capability_session as S
import capability_runtime as R
import run_capabilities as H
import run as STARTUP
import cajviewer_canary_fixtures as F


def pin(payload=b"original"):
    return {"size_bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}


def blob(value):
    return json.dumps(value, separators=(",", ":")).encode("utf-8")


def binding(token):
    return {"kind": "property", "atom": "ORIGINAL_TEST_" + token.upper(), "encoding": "utf-8",
            "template": "{" + token + "}"}


def metadata_contract(page=False):
    result = {"document": binding("document"), "count": binding("count"),
              "extent": {"kind": "property", "atom": "ORIGINAL_TEST_EXTENT", "encoding": "ascii",
                         "template": "{width} x {height} pt"}}
    if page:
        result["page"] = binding("page")
    return result


def profile():
    """Only fabricated original controls; none of these pins bind a runtime."""
    action = {"kind": "key", "value": "F1", "target": "main"}
    return {"protocol": "original-pdf-capabilities-v1", "status": "FROZEN",
        "review": {"root": "1" * 64, "independent": "2" * 64}, "image": "sha256:" + "3" * 64,
        "runtime_view": {key: pin() for key in R.RUNTIME_FILES}, "runtime_sources": {"original.py": pin()},
        "sources": {"original.py": pin()}, "controls": {"digital.pdf": pin(), "image-only.pdf": pin()},
        "host_tools": {"docker": {"path": "/usr/bin/docker", "identity": pin()},
                       "python": {"path": "/usr/bin/python3.13", "identity": pin()}}, "host_environment": pin(),
        "environment": {"DISPLAY": ":99", "HOME": "/home/canary", "XDG_RUNTIME_DIR": "/runtime"},
        "caps": copy.deepcopy(P.CAPS), "seconds": 60,
        "bindings": {"window_class": "original-test", "open_dialog": {"class": "original-open-test", "role": "file-open",
                     "marker": {"kind": "property", "atom": "ORIGINAL_DIALOG", "encoding": "ascii", "template": "Open original PDF"}},
                     "documents": {name: metadata_contract() for name in ("digital.pdf", "image-only.pdf")},
                     "pages": {key: metadata_contract(True) for key in P.PAGE_KEYS}, "about": None},
        "actions": {"discover": [], "open-image": [action], "copy": [action], "navigate": {key: [action] for key in P.PAGE_KEYS},
                    "fit": [], "about": []},
        "pages": {key: {"rect": [100, 50, 769, 1026] if key == "digital-3" else [100, 50, 1026, 769],
                        "grid": [769, 1026] if key == "digital-3" else [1026, 769],
                        "points": ["2052", "1538"] if key == "digital-4" else ["256.5", "192.25"],
                        "rotation": 90 if key == "digital-3" else 0, "origin": "complete-page-capture", "outside_rgb": [64, 64, 64]}
                  for key in P.PAGE_KEYS},
        "settings": {"display": [1600, 1200, 24, 96], "layout": "original test layout", "color": "original RGB8",
                     "font": "authored Type3", "print": "not used"},
        "clipboard": {"target": "UTF8_STRING", "encoding": "utf-8", "limit_bytes": 64, "seconds": 10, "manager_revision": "TIMESTAMP"}}


def observation(serial, key="digital-1", document=False, owner=(51, 71)):
    negative = key == "image-only-1"
    page = 1 if negative else int(key[-1])
    width, height = ("2052", "1538") if key == "digital-4" else ("256.5", "192.25")
    value = {"document": b"image-only.pdf" if negative else b"digital.pdf", "count": b"1" if negative else b"4",
             "extent": (width + " x " + height + " pt").encode(), "serial": serial, "owner": owner}
    if not document:
        value["page"] = str(page).encode()
    return value


class ContractTests(unittest.TestCase):
    def test_no_input_entries_do_not_touch_files_environment_tools_or_processes(self):
        class RefusingEnvironment:
            def refuse(self, *args, **kwargs):
                raise AssertionError("no-input entry accessed ENV")

            get = __getitem__ = __iter__ = __contains__ = __len__ = keys = items = values = refuse

        with mock.patch.object(H, "execute") as execute, mock.patch.object(S, "run_session") as session, \
                mock.patch.object(H, "read_file") as read, mock.patch.object(S.subprocess, "Popen") as popen, \
                mock.patch.object(os, "environ", RefusingEnvironment()), \
                mock.patch.object(H.argparse, "ArgumentParser", side_effect=AssertionError("no-input parser construction")) as parser:
            for entry in (H.main, S.main, COLLECT.main):
                for argv in ([], None):
                    with self.subTest(entry=entry.__module__, argv=argv), \
                            mock.patch.object(sys, "argv", ["original-no-input-entry"]), \
                            contextlib.redirect_stdout(io.StringIO()) as output:
                        self.assertEqual(entry(argv), 0)
                    expected = {"status": "NOT_RUN", "application_launches": 0, "vendor_passes": 0}
                    if entry is H.main:
                        expected.update(historical_launch_outcomes=12, maximum_cumulative_launches=14)
                    self.assertEqual(json.loads(output.getvalue()), expected)
            execute.assert_not_called(); session.assert_not_called(); read.assert_not_called(); popen.assert_not_called()
            parser.assert_not_called()

    def test_complete_original_contract_and_unknown_or_duplicate_contract_refusal(self):
        self.assertEqual(P.validate_profile(blob(profile()))["caps"], P.CAPS)
        for payload in (b'{"a":1,"a":2}', b'{"a":NaN}', b'\xff', b'[]'):
            with self.subTest(payload=payload), self.assertRaises(P.Refusal):
                P.validate_profile(payload)
        for key, value in (("status", "DRAFT"), ("image", "latest"), ("runtime_view", {})):
            value_profile = profile(); value_profile[key] = value
            with self.subTest(key=key), self.assertRaises(P.Refusal):
                P.validate_profile(blob(value_profile))

    def test_frozen_numbers_reject_boolean_float_and_missing_source_tokens(self):
        for path, value in ((["caps", "pids"], 256.0), (["settings", "display", 2], 24.0),
                            (["pages", "digital-1", "rotation"], False), (["pages", "digital-1", "grid", 0], 1026.0),
                            (["bindings", "pages", "digital-1", "count", "template"], "4")):
            candidate = profile(); target = candidate
            for key in path[:-1]: target = target[key]
            target[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(P.Refusal): P.validate_profile(blob(candidate))

    def test_glyph_decoder_checks_every_cell_unknown_glyph_and_semantic_number(self):
        black, white = b"\0\0\0", b"\xff\xff\xff"
        contract = {"kind": "bitmap", "rect": [1, 1, 2, 1], "cell": [1, 1],
                    "glyphs": {"4": pin(black), " ": pin(white)}, "template": "{count} "}
        P.validate_binding(contract, ("count",))
        self.assertEqual(P.validate_observation(black + white, contract, {"count": 4})["decoded_text"], "4 ")
        for data, count in ((black + white, 1), (black + b"\x01\x01\x01", 4), (black, 4)):
            with self.subTest(data=data), self.assertRaises(P.Refusal): P.validate_observation(data, contract, {"count": count})

    def test_page_gate_requires_first_document_owner_source_count_extent_and_order(self):
        for field, value in (("owner", (52, 72)), ("document", b"wrong.pdf"), ("count", b"1"),
                             ("extent", b"2052 x 1538 pt"), ("serial", 1), ("page", b"2")):
            gate = G.PageGate(profile()); gate.opened("digital.pdf", observation(1, document=True))
            raw = observation(2); raw[field] = value
            with self.subTest(field=field), self.assertRaises(P.Refusal): gate.page("digital-1", raw)

    def test_blank_rotation_large_and_negative_require_identical_physical_sequence(self):
        gate = G.PageGate(profile()); gate.opened("digital.pdf", observation(1, document=True))
        for serial, key in enumerate(P.PAGE_KEYS[:4], 2): gate.page(key, observation(serial, key))
        gate.opened("image-only.pdf", observation(6, "image-only-1", document=True))
        result = gate.page("image-only-1", observation(7, "image-only-1"))
        self.assertEqual((result["physical_index"], result["document_generation"], gate.next_index), (0, 2, 5))
        with self.assertRaises(P.Refusal): gate.page("image-only-1", observation(8, "image-only-1"))

    def test_admission_before_effect_primary_failure_and_independent_closing(self):
        clock = mock.Mock(return_value=0)
        ledger = P.Ledger(1, clock=clock); effects = []
        with self.assertRaises(OSError): ledger.perform("launcher", "first", lambda: (_ for _ in ()).throw(OSError()))
        with self.assertRaises(P.Refusal): ledger.perform("launcher", "second", lambda: effects.append(1))
        self.assertEqual(effects, []); self.assertEqual(ledger.counts["launcher"], 1)
        primary = dict(ledger.first_failure); clock.return_value = 2
        ledger.perform("closing", "close", lambda: effects.append(2))
        with self.assertRaises(P.Refusal): ledger.perform("gui", "late", lambda: effects.append(3))
        ledger.fail("closing-fault", OSError(), closing=True)
        self.assertEqual(ledger.first_failure, primary); self.assertEqual(effects, [2])


def frame(key="digital-1"):
    """Original producer independent of the validation and its boundary scans."""
    width, height = 1026, 769
    if key == "digital-2":
        pixels = b"\xff" * (width * height * 3)
    elif key == "digital-3":
        # Use the original authoring facts, with a different row/column
        # traversal from the production orientation validator.
        rows = [bytearray(b"\xff" * (width * 3)) for _ in range(height)]
        for x, y, w, h, color in F.rectangles(((24, 125, "RUST 321"),)):
            begin, end = round(x * 4), round((x + w) * 4)
            for row in range(height - round((y + h) * 4), height - round(y * 4)):
                rows[row][begin * 3:end * 3] = bytes(color) * (end - begin)
        pixels = b"".join(bytes(rows[row][column * 3:column * 3 + 3])
                          for column in range(width) for row in range(height - 1, -1, -1))
        width, height = height, width
    else:
        pixels = F.raster()
    data = bytearray(bytes([64, 64, 64]) * (1600 * 1200))
    for row in range(height):
        start = ((50 + row) * 1600 + 100) * 3
        data[start:start + width * 3] = pixels[row * width * 3:(row + 1) * width * 3]
    return b"P6\n1600 1200\n255\n" + data


class PageGridTests(unittest.TestCase):
    def test_complete_original_grid_raw_and_decoded_identity_are_distinct(self):
        for key in P.PAGE_KEYS:
            with self.subTest(key=key):
                result = G.verify_page_frame(frame(key), key, profile()["pages"][key], F)
                self.assertEqual(result["full_grid"], "EXACT_ORIGINAL")
                self.assertNotEqual(result["raw"]["sha256"], result["decoded"]["sha256"])

    def test_midpoints_do_not_hide_corner_edge_interior_or_boundary_clipping(self):
        original = frame(); header = len(b"P6\n1600 1200\n255\n")
        for x, y in ((100, 50), (101, 51), (300, 50), (99, 700), (120, 900), (1100, 820), (500, 500)):
            damaged = bytearray(original); offset = header + (y * 1600 + x) * 3
            damaged[offset:offset + 3] = b"\x07\x08\x09"
            # (120,900)/(1100,820) are outside the required rectangle/ring;
            # changes there must not affect the independently bounded page.
            inside = 99 <= x <= 1126 and 49 <= y <= 819
            if inside:
                with self.subTest(x=x,y=y), self.assertRaises(P.Refusal): G.verify_page_frame(bytes(damaged), "digital-1", profile()["pages"]["digital-1"], F)
            else:
                G.verify_page_frame(bytes(damaged), "digital-1", profile()["pages"]["digital-1"], F)

    def test_truncated_extra_wrong_grid_rotated_and_unbounded_blank_refuse(self):
        original = frame()
        for data, key in ((original[:-1], "digital-1"), (original + b"x", "digital-1"), (original, "digital-3"),
                          (frame("digital-2"), "digital-1")):
            with self.subTest(key=key), self.assertRaises(P.Refusal): G.verify_page_frame(data, key, profile()["pages"][key], F)
        contract = profile()["pages"]["digital-2"]; contract["rect"][0] = 0
        with self.assertRaises(P.Refusal): G.verify_page_frame(frame("digital-2"), "digital-2", contract, F)


class CollectorTests(unittest.TestCase):
    def test_prewrite_budget_refuses_before_effect_and_partial_writes_are_ordered(self):
        stream = mock.Mock(); meter = IO.Meter(3)
        with self.assertRaises(P.Refusal): meter.write(stream, b"abcd")
        stream.write.assert_not_called()
        output = bytearray()
        def short(view): output.extend(view[:1]); return 1
        stream.write.side_effect = short; meter = IO.Meter(16); meter.write(stream, b"abcd")
        self.assertEqual(bytes(output), b"abcd"); self.assertEqual(meter.written, 4)
        for count in (0, -1, 1.5, None, 5):
            with self.subTest(count=count):
                stream.write.side_effect = None; stream.write.return_value = count
                meter = IO.Meter(16)
                with self.assertRaises(P.Refusal): meter.write(stream, b"abcd")
                self.assertEqual(meter.write_completion, "UNKNOWN_AFTER_SUBMISSION")
        # Positive partial progress consumes every offered byte before effect.
        stream=mock.Mock(); stream.write.return_value=1; meter=IO.Meter(5)
        with self.assertRaisesRegex(P.Refusal,"write-budget"): meter.write(stream,b"abcd")
        stream.write.assert_called_once()
        self.assertEqual((meter.write_submitted,meter.written),(4,1))
        self.assertEqual(meter.summary()["write_submitted_limit_bytes"],5)
        stream=mock.Mock(); stream.write.side_effect=OSError; meter=IO.Meter(8)
        with self.assertRaises(OSError): meter.write(stream,b"abcd")
        self.assertEqual((meter.write_submitted,meter.written,meter.write_completion),(4,0,"UNKNOWN_AFTER_SUBMISSION"))

    def test_exact_flat_roundtrip_and_immutable_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            source, target = Path(tmp)/"source", Path(tmp)/"target"; source.mkdir(); target.mkdir()
            (source/"ready").write_bytes(b"capability-receipt-complete\n"); (source/"session.json").write_bytes(b"{}")
            wire = io.BytesIO(); expected = IO.produce(source, wire, IO.Meter())
            records = IO.consume(io.BytesIO(wire.getvalue()), target, IO.Meter())
            self.assertEqual(records, expected); self.assertEqual((target/"ready").stat().st_mode & 0o777, 0o400)

    def test_duplicate_extra_traversal_short_transport_and_trailing_bytes_refuse(self):
        def member(name, data): return struct.pack("<BQ", len(name), len(data)) + name.encode() + data
        valid = IO.MAGIC + member("session.json", b"{}") + member("ready", b"ok")
        for wire in (valid + member("ready", b"again") + b"\0", IO.MAGIC + member("extra", b"x") + b"\0",
                     IO.MAGIC + member("../ready", b"x") + b"\0", valid[:-1], valid + b"\0tail"):
            with tempfile.TemporaryDirectory() as tmp, self.subTest(wire=wire[:20]), self.assertRaises(P.Refusal):
                IO.consume(io.BytesIO(wire), Path(tmp), IO.Meter())

    def test_symlink_ancestor_leaf_fifo_extra_file_and_oversize_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root/"session.json").write_bytes(b"{}"); (root/"ready").write_bytes(b"ok")
            (root/"extra").write_bytes(b"x")
            with self.assertRaises(P.Refusal): IO.produce(root, io.BytesIO(), IO.Meter())
            (root/"extra").unlink(); (root/"ready").unlink(); (root/"ready").symlink_to(root/"session.json")
            with self.assertRaises(OSError): IO.produce(root, io.BytesIO(), IO.Meter())
            (root/"ready").unlink(); os.mkfifo(root/"ready")
            with self.assertRaises(P.Refusal): IO.produce(root, io.BytesIO(), IO.Meter())
            (root/"ready").unlink()
            with (root/"ready").open("wb") as oversized: oversized.truncate(65)
            with self.assertRaises(P.Refusal): IO.produce(root, io.BytesIO(), IO.Meter())
            alias = root/"alias"; alias.symlink_to(root, target_is_directory=True)
            with self.assertRaises(OSError): IO.read_file(alias/"session.json", IO.Meter())

    def test_changed_file_short_read_and_requested_read_budget_do_not_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/"original"; path.write_bytes(b"abc")
            meter = IO.Meter(2)
            with self.assertRaises(P.Refusal): IO.read_file(path, meter)
            self.assertEqual(meter.returned, 0)
            with mock.patch.object(IO, "stable", side_effect=[(1,), (2,)]), self.assertRaises(P.Refusal): IO.read_file(path, IO.Meter())
        with self.assertRaises(P.Refusal): IO.Meter().exact(io.BytesIO(b"a"), 2)


def setup_bytes():
    header = struct.pack("<IIIIHHBBBBBBBB4x", 1, 0x200000, 0x1fffff, 0, 0, 65535, 1, 1, 0, 0, 32, 32, 8, 255)
    pixmap = struct.pack("<BBB5x", 24, 32, 32)
    screen = struct.pack("<IIIIIHHHHHHIBBBB", 1, 2, 0xffffff, 0, 0, 1600, 1200, 423, 317, 1, 1, 42, 0, 0, 24, 1)
    visual = struct.pack("<IBBHIII4x", 42, 4, 8, 256, 0xff0000, 0xff00, 0xff)
    body = header + pixmap + screen + struct.pack("<BxH4x", 24, 1) + visual
    return struct.pack("<BBHHH", 1, 0, 11, 0, len(body)//4) + body


class SocketTranscript:
    """Scripted original server bytes; no real socket/child is created."""
    def __init__(self, payload, maximum=65536): self.payload=bytearray(payload); self.maximum=maximum; self.sent=bytearray(); self.closed=False
    def settimeout(self, value): self.timeout=value
    def send(self, data): self.sent.extend(data[:self.maximum]); return min(len(data), self.maximum)
    def recv(self, count):
        result = bytes(self.payload[:min(count,self.maximum)]); del self.payload[:len(result)]; return result
    def close(self): self.closed=True


def reply(sequence=1, detail=0, data=b"", fields=None):
    head=bytearray(struct.pack("<BBHI24x", 1, detail, sequence, (len(data)+3)//4))
    for offset, value in (fields or {}).items(): struct.pack_into("<I",head,offset,value)
    return bytes(head)+X.padding(data)


class WireTests(unittest.TestCase):
    def test_real_wire_encoder_setup_fragmented_reads_and_partial_sends_are_metered(self):
        connection = SocketTranscript(setup_bytes() + reply(fields={8:17}), maximum=1)
        wire=X.X11(connection=connection); result=wire.request(43, reply=True, limit=0)
        self.assertEqual(struct.unpack_from("<I",result[0],8)[0],17)
        self.assertGreater(wire.requested,wire.returned); self.assertEqual(connection.sent[:12],struct.pack("<BBHHHHH",108,0,11,0,0,0,0))
        self.assertEqual(connection.sent[-4:],b"+\0\x01\0")

    def test_partial_property_reply_oversize_sequence_and_short_reply_refuse(self):
        partial=reply(detail=8,data=b"a",fields={8:7,12:1,16:1})
        for packet, operation in ((partial, lambda w:w.property(1,7,limit=4)),
                                  (reply(data=b"abcd"),lambda w:w.request(43,reply=True,limit=0)),
                                  (reply(sequence=2),lambda w:w.request(43,reply=True,limit=0)),
                                  (b"\x01",lambda w:w.request(43,reply=True,limit=0))):
            connection=SocketTranscript(setup_bytes()+packet); wire=X.X11(connection=connection)
            with self.subTest(packet=packet[:4]),self.assertRaises(P.Refusal): operation(wire)

    def test_phase_transaction_and_late_completion_bound_actual_wire(self):
        clock=mock.Mock(return_value=0); connection=SocketTranscript(setup_bytes()+reply())
        wire=X.X11(connection=connection,clock=clock,deadline=2)
        wire.transaction_deadline=0.1
        original=connection.recv
        def late(count):
            value=original(count); clock.return_value=0.2; return value
        connection.recv=late
        with self.assertRaises(P.Refusal): wire.request(43,reply=True,limit=0)
        self.assertEqual(wire.deadline,0.1)
        clock.return_value=3
        with self.assertRaises(P.Refusal): wire.event(lambda e:True,deadline=100)

    def test_event_request_and_byte_caps_refuse_before_unbounded_work(self):
        connection=SocketTranscript(setup_bytes()); wire=X.X11(connection=connection,request_limit=1)
        wire.request(19,struct.pack("<II",1,1))
        sent=len(connection.sent)
        with self.assertRaises(P.Refusal): wire.request(19,struct.pack("<II",1,1))
        self.assertEqual(len(connection.sent),sent)
        wire.byte_limit=wire.requested
        with self.assertRaises(P.Refusal): wire._read(1)
        # Exercise the actual production send loop after the original setup.
        connection=SocketTranscript(setup_bytes()); wire=X.X11(connection=connection,clock=lambda:0)
        baseline=wire.send_submitted; known=wire.sent
        connection.send=mock.Mock(return_value=1); wire.byte_limit=baseline+5
        with self.assertRaisesRegex(P.Refusal,"x11-send-budget"): wire._send(b"abcd")
        connection.send.assert_called_once()
        self.assertEqual((wire.send_submitted-baseline,wire.sent-known),(4,1))
        wire.byte_limit=baseline+16; connection.send=mock.Mock(side_effect=OSError)
        with self.assertRaises(OSError): wire._send(b"abcd")
        self.assertEqual(wire.send_submitted-baseline,8)
        self.assertEqual(wire.accounting()["send_completion"],"UNKNOWN_AFTER_SUBMISSION")


class SelectionTranscript:
    """Minimal original selection-owner fixture for the real adapter logic.

    It emits raw ICCCM events and target/property bytes. GUI dispatch and
    selection-transfer results are never returned as a VERIFIED summary.
    """
    ATOMS={"CLIPBOARD":1,"CAJ_CAP_RESULT":2,"UTF8_STRING":3,"INCR":4,"TARGETS":5,"TIMESTAMP":6,"STRING":7,"INTEGER":8,"ATOM":9}
    def __init__(self,payload=b"RUST \xe4\xb8\xad",revision=101,incr=False):
        self.clock=lambda:0; self.transaction_deadline=None; self.current=42; self.properties={}; self.events=[]
        self.payload=payload; self.revision=revision; self.incr=incr; self.parts=[]; self.calls=[]
    def window(self): return 42
    def atom(self,name): return self.ATOMS[name]
    def timestamp(self,window): return 100
    def owner(self,selection): return self.current
    def set_owner(self,selection,window,timestamp): self.current=window
    def delete(self,window,prop): self.properties.pop((window,prop),None)
    def change(self,window,prop,type_atom,bits,data): self.properties[(window,prop)]=(type_atom,bits,data)
    def notify(self,event,prop):
        time,_,window,sel,target,_=struct.unpack_from("<IIIIII",event,4)
        self.events.append(struct.pack("<BBHIIIII8x",31,0,0,time,window,sel,target,prop))
    def discard_events(self,predicate): self.events=[event for event in self.events if not predicate(event)]
    def request(self,opcode,data):
        self.calls.append((opcode,data)); window,selection,target,prop,time=struct.unpack("<IIIII",data)
        if self.current==42:
            self.events.append(struct.pack("<BBHIIIIII4x",30,0,0,time,42,window,selection,target,prop))
            return
        if target==5: value=(9,32,struct.pack("<III",3,5,6))
        elif target==6: value=(8,32,struct.pack("<I",self.revision))
        elif self.incr:
            value=(4,32,struct.pack("<I",len(self.payload)))
            self.parts=[(3,8,self.payload),(3,8,b"")]
        else: value=(3,8,self.payload)
        self.properties[(window,prop)]=value
        self.events.append(struct.pack("<BBHIIIII8x",31,0,0,time,window,selection,target,prop))
    def property(self,window,prop,limit,delete):
        value=self.properties[(window,prop)]
        if delete:
            if self.parts:
                self.properties[(window,prop)]=self.parts.pop(0)
                self.events.append(struct.pack("<BBHIIIB15x",28,0,0,window,prop,101,0))
            else: self.properties.pop((window,prop),None)
        P.require(len(value[2])<=limit,"property-partial-or-format")
        return value
    def event(self,predicate,deadline):
        for index,value in enumerate(self.events):
            if predicate(value): return self.events.pop(index)
        raise P.Refusal("x11-deadline")
    def copied(self):
        self.current=77
        self.events.append(struct.pack("<BBHIII16x",29,0,0,101,42,1))


class ClipboardTests(unittest.TestCase):
    def adapter(self,wire,**kwargs):
        return C.Clipboard(wire,P.Ledger(60,clock=lambda:0),profile()["clipboard"],owner_allowed=lambda owner:owner==77)

    def test_live_production_sentinel_targets_revision_and_complete_payload(self):
        wire=SelectionTranscript(); clipboard=self.adapter(wire)
        payload,result=clipboard.copy("original",wire.copied)
        self.assertEqual(payload,wire.payload); self.assertTrue(result["transfer"]["complete"])
        self.assertEqual((result["owner"],result["revision"],result["selection_clear_timestamp"]),(77,101,101))
        self.assertEqual(result["text"]["unicode_origin"],"UNVERIFIED")

    def test_complete_empty_is_distinct_from_absent_and_stale_sentinel(self):
        wire=SelectionTranscript(b""); payload,result=self.adapter(wire).copy("original",wire.copied)
        self.assertEqual((payload,result["status"]),(b"","COMPLETE_EMPTY"))
        for data,dispatch in ((b"ORIGINAL-CAJ-CAP-SENTINEL:original",True),(b"unobserved",False)):
            wire=SelectionTranscript(data)
            with self.subTest(data=data),self.assertRaises(P.Refusal): self.adapter(wire).copy("original",wire.copied if dispatch else lambda:None)

    def test_incr_completion_and_missing_terminal_type_or_size_mutations(self):
        wire=SelectionTranscript(incr=True); payload,result=self.adapter(wire).copy("original",wire.copied)
        self.assertEqual(payload,wire.payload); self.assertTrue(result["transfer"]["incremental"])
        for failure in ("missing-end","wrong-type","oversize"):
            wire=SelectionTranscript(b"x"*65 if failure=="oversize" else b"abc",incr=True)
            original=wire.property
            def mutated(window,prop,limit,delete,original=original,failure=failure,wire=wire):
                value=original(window,prop,limit,delete)
                if value[0]==4 and failure=="missing-end": wire.parts=[]
                if value[0]==3 and wire.current==77 and failure=="wrong-type": return (9,32,value[2])
                return value
            wire.property=mutated
            with self.subTest(failure=failure),self.assertRaises(P.Refusal): self.adapter(wire).copy("original",wire.copied)

    def test_owner_revision_target_mutations_and_manager_stability_never_prove_copy(self):
        for failure in ("old-revision","foreign-owner","missing-target","manager-owner"):
            wire=SelectionTranscript(revision=100 if failure=="old-revision" else 101)
            def dispatch(wire=wire,failure=failure):
                wire.copied()
                if failure in ("foreign-owner","manager-owner"): wire.current=99
            if failure=="missing-target":
                original=wire.property
                def property_without_target(window,prop,limit,delete):
                    result=original(window,prop,limit,delete)
                    return (9,32,struct.pack("<II",5,6)) if result[0]==9 else result
                wire.property=property_without_target
            with self.subTest(failure=failure),self.assertRaises(P.Refusal): self.adapter(wire).copy("original",dispatch)

    def test_transaction_deadline_restored_and_no_late_completion_acceptance(self):
        wire=SelectionTranscript(); original=wire.property
        def late(window,prop,limit,delete):
            result=original(window,prop,limit,delete)
            if wire.current==77: wire.clock=lambda:11
            return result
        wire.property=late
        with self.assertRaises(P.Refusal): self.adapter(wire).copy("original",wire.copied)
        self.assertIsNone(wire.transaction_deadline)

    def test_raw_encoding_codepoint_order_and_timestamp_wrap_are_not_normalized(self):
        text=P.text_evidence(" 中\nRUST ".encode(),"utf-8")
        self.assertEqual(text["text_prefix"]," 中\nRUST "); self.assertFalse(text["exact_expected"])
        self.assertEqual(P.text_evidence(b"\xff","utf-8")["decode"],"FAIL")
        self.assertTrue(C.newer(2,2**32-2)); self.assertFalse(C.newer(5,5)); self.assertFalse(C.newer(4,5))

    def test_host_independently_binds_raw_clear_sentinel_targets_transfer_and_codepoints(self):
        wire=SelectionTranscript(); payload,evidence=self.adapter(wire).copy("original",wire.copied)
        self.assertEqual(C.validate_copy(evidence,payload,profile()["clipboard"],"original")["status"],"COMPLETE")
        for key,value in (("selection_clear_hex","00"*32),("baseline_revision",evidence["revision"]),
                          ("owner",evidence["sentinel_owner"]),("targets",[]),("after",{}),("text",{})):
            changed=copy.deepcopy(evidence); changed[key]=value
            with self.subTest(key=key),self.assertRaises(P.Refusal): C.validate_copy(changed,payload,profile()["clipboard"],"original")
        with self.assertRaises(P.Refusal): C.validate_copy(evidence,payload,profile()["clipboard"],"other-session")


class DesktopTests(unittest.TestCase):
    class Wire:
        root=1
        def __init__(self): self.pid=71; self.visible=True; self.kind="_NET_WM_WINDOW_TYPE_NORMAL"; self.focus=51
        def atom(self,name): return name
        def viewable(self,window): return self.visible
        def geometry(self,window): return (10,10,1500,1150)
        def property(self,window,atom,**kwargs):
            if atom=="_NET_WM_PID": return 0,32,struct.pack("<I",self.pid)
            if atom=="_NET_WM_WINDOW_TYPE": return 0,32,struct.pack("<I",2 if self.kind=="_NET_WM_WINDOW_TYPE_DIALOG" else 3)
            if atom=="WM_CLASS": return 0,8,b"original-test\0"
            if atom=="_NET_ACTIVE_WINDOW": return 0,32,struct.pack("<I",self.focus)
            return 0,8,b"unapproved"
    def desktop(self):
        wire=self.Wire()
        original_atom=wire.atom
        wire.atom=lambda name:{"_NET_WM_WINDOW_TYPE_NORMAL":3,"_NET_WM_WINDOW_TYPE_DIALOG":2}.get(name,original_atom(name))
        command=mock.Mock(return_value=HostTests.result(b"51\n"))
        return S.Desktop(profile(),P.Ledger(60),71,wire,command),wire,command

    def test_actual_observer_rejects_foreign_disappeared_dialog_and_changed_owner(self):
        with mock.patch.object(S.os,"getpgid",side_effect=lambda pid:pid):
            desktop,wire,_=self.desktop(); self.assertEqual(desktop.owned(),(51,71))
            for field,value in (("pid",72),("visible",False),("kind","_NET_WM_WINDOW_TYPE_DIALOG")):
                desktop,wire,_=self.desktop(); setattr(wire,field,value)
                with self.subTest(field=field),self.assertRaises(P.Refusal): desktop.owned()
            desktop,wire,_=self.desktop(); desktop.owned(); wire.pid=72
            with self.assertRaises(P.Refusal): desktop.owned()

    def test_focus_and_pointer_admission_refuse_before_unapproved_action(self):
        with mock.patch.object(S.os,"getpgid",return_value=71):
            desktop,wire,command=self.desktop(); wire.focus=52
            with self.assertRaises(P.Refusal): desktop.dispatch(profile()["actions"]["copy"])
            self.assertFalse(any("key" in args[0] for args,kwargs in command.call_args_list))
            desktop,wire,command=self.desktop()
            with self.assertRaises(P.Refusal): desktop.dispatch([{"kind":"click","target":"main","value":[0,0]}])
            self.assertFalse(any("mousemove" in args[0] for args,kwargs in command.call_args_list))

    def test_absent_window_readiness_is_a_complete_failure_observation_not_success(self):
        desktop,_,command=self.desktop(); command.return_value=HostTests.result(status="FAIL",code=1)
        with self.assertRaisesRegex(P.Refusal,"owned-normal-or-open-dialog-unavailable"): desktop.owned()
        self.assertEqual(desktop.ledger.counts["query"],1)
        command.return_value["prefix_truncated"]=True
        with self.assertRaisesRegex(P.Refusal,"desktop-helper-incomplete"): desktop.owned()

    def test_real_log_drain_retains_prefix_refuses_limit_and_keeps_cleanup_separate(self):
        with tempfile.TemporaryDirectory() as tmp,mock.patch.object(S.threading,"Thread") as thread,mock.patch.object(S.os,"killpg") as kill:
            thread.return_value.is_alive.return_value=False
            process=mock.Mock(pid=71,stdout=io.BytesIO(b"abcdef")); fail=mock.Mock()
            pump=S.LogPump(process,Path(tmp)/"log",4,on_failure=fail); pump._drain()
            self.assertFalse(pump.summary()["complete"]); fail.assert_called_once(); kill.assert_called_once()
            with self.assertRaises(P.Refusal): pump.close()

    def test_finite_menu_observation_selects_only_exact_safe_branch_and_stops_on_unknown(self):
        step={"actions":[],"binding":{"kind":"property","atom":"ORIGINAL_MENU","encoding":"ascii","template":"Original open menu"},
              "outcomes":[{"label":"open","text":"Original open menu","actions":profile()["actions"]["copy"]},
                          {"label":"closed","text":"Original closed menu","actions":[]}]}
        p=profile(); p["actions"]["discover"]=[step]; P.validate_profile(blob(p))
        desktop,_,_=self.desktop(); desktop.profile=p
        with mock.patch.object(desktop,"owned",return_value=(51,71)),mock.patch.object(desktop,"read_binding",return_value=b"Original open menu"), \
                mock.patch.object(desktop,"dispatch") as dispatch:
            desktop.discover([step]); self.assertEqual(desktop.discovery_raw[0]["label"],"open")
            self.assertEqual(dispatch.call_args_list[-1].args[0],step["outcomes"][0]["actions"])
        with mock.patch.object(desktop,"owned",return_value=(51,71)),mock.patch.object(desktop,"read_binding",return_value=b"Unknown account prompt"), \
                mock.patch.object(desktop,"dispatch") as dispatch,self.assertRaises(P.Refusal):
            desktop.discover([step])
        self.assertEqual(dispatch.call_count,1)
        p["actions"]["discover"]= [copy.deepcopy(step)]*7
        with self.assertRaises(P.Refusal): P.validate_profile(blob(p))


class SessionSupervisorTests(unittest.TestCase):
    def run_failure(self,process_factory,*,baseline_failure=False,expiry=False):
        p=profile(); ledger=P.Ledger(60,clock=lambda:0)
        if expiry: ledger.deadline=0
        metrics={"memory.events":"oom 0\noom_kill 0\n","memory.peak":"1000","pids.peak":"2"}
        with tempfile.TemporaryDirectory() as tmp,contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.dict(S.os.environ,p["environment"],clear=True))
            for name,value in (("getuid",1000),("getgid",1000)):
                stack.enter_context(mock.patch.object(S.os,name,return_value=value))
            for name,value in (("Ledger",ledger),("session_environment",{"status":"PASS"}),
                               ("session_limits",{"status":"PASS"}),("cgroup_snapshot",metrics),
                               ("process_metadata",[]),("close_owned_tree",{"status":"PASS","remaining":0})):
                stack.enter_context(mock.patch.object(S,name,return_value=value))
            stack.enter_context(mock.patch.object(S,"run_bounded",return_value=HostTests.result()))
            stack.enter_context(mock.patch.object(S.os,"killpg"))
            log=mock.Mock(); log.summary.return_value={"complete":True}
            stack.enter_context(mock.patch.object(S,"LogPump",return_value=log))
            baseline=stack.enter_context(mock.patch.object(S,"collector_baseline",return_value={1:1}))
            if baseline_failure: baseline.side_effect=P.Refusal("collector-missing")
            wire=mock.Mock(display={}); wire.accounting.return_value={"requests":0}
            result=S.run_session(p,Path(tmp),1,process_factory=process_factory,wire_factory=mock.Mock(return_value=wire))
            self.assertEqual(json.loads((Path(tmp)/"session.json").read_bytes())["first_failure"],result["first_failure"])
            return result

    def test_real_supervisor_refuses_before_helpers_and_keeps_unknown_escape_admission(self):
        factory=mock.Mock(side_effect=KeyboardInterrupt)
        result=self.run_failure(factory,baseline_failure=True); factory.assert_not_called()
        self.assertEqual(result["application_launches"],0)
        result=self.run_failure(factory); self.assertEqual(result["admitted"]["persistent-helper"],1)
        self.assertEqual(result["events"][0]["status"],"UNKNOWN_AFTER_ADMISSION")
        self.assertEqual(result["process_cleanup"]["status"],"PASS"); self.assertEqual(result["application_launches"],0)
        factory.reset_mock(); result=self.run_failure(factory,expiry=True); factory.assert_not_called()
        self.assertEqual(result["admitted"]["persistent-helper"],0)

    def test_official_launcher_admission_precedes_escape_and_all_known_helpers_close(self):
        processes=[mock.Mock(pid=81),mock.Mock(pid=82)]
        factory=mock.Mock(side_effect=[*processes,KeyboardInterrupt])
        result=self.run_failure(factory)
        self.assertEqual(result["application_launches"],1); self.assertEqual(result["admitted"]["launcher"],1)
        self.assertEqual(result["counts"]["pages_remaining"],5); self.assertEqual(result["counts"]["copy_remaining"],2)
        for process in processes: process.wait.assert_called_once_with(timeout=5)
        self.assertEqual(result["first_failure"]["stage"],"official-launcher")
        self.assertEqual(result["cleanup"],"AWAITING_HOST_CONTAINER_REMOVAL")

    def test_live_collector_lease_requires_actual_pid_birth_and_exact_original_command(self):
        record={"protocol":"original-capability-collector/1","pid":71,"birth":123,"uid_gid":[1000,1000],"seconds":120}
        command=b"/usr/bin/python3\0-B\0/opt/capability/capability_collect.py\0--wait-seconds\0"+b"120\0"
        with mock.patch.object(S,"read_file",return_value=(blob(record),pin())),mock.patch.object(S,"process_identities",return_value={1:1,71:123}), \
                mock.patch("builtins.open",return_value=io.BytesIO(command)):
            self.assertEqual(S.collector_baseline(IO.Meter(),60),{1:1,71:123})
        for key,value in (("birth",124),("seconds",120.0),("uid_gid",[1000.0,1000])):
            changed=dict(record); changed[key]=value
            with self.subTest(key=key),mock.patch.object(S,"read_file",return_value=(blob(changed),pin())), \
                    mock.patch.object(S,"process_identities",return_value={1:1,71:123}),self.assertRaises(P.Refusal):
                S.collector_baseline(IO.Meter(),60)

    def test_actual_cgroup_cap_file_parser_refuses_missing_zero_swap_and_file_limit_changes(self):
        values={"memory.max":str(P.CAPS["memory_bytes"]),"memory.swap.max":"0","pids.max":"256","cpu.max":"200000 100000"}
        limits=lambda kind:(0,0) if kind==S.resource.RLIMIT_CORE else (67108864,67108864)
        with mock.patch.object(S,"cgroup_value",side_effect=lambda name,meter:values[name]),mock.patch.object(S.resource,"getrlimit",side_effect=limits):
            self.assertEqual(S.session_limits(IO.Meter())["cpu_quota"],200000)
            for name,value in (("memory.max","max"),("memory.swap.max","1"),("pids.max","512"),("cpu.max","max 100000")):
                original=values[name]; values[name]=value
                with self.subTest(name=name),self.assertRaises(P.Refusal): S.session_limits(IO.Meter())
                values[name]=original


class WorkflowTests(unittest.TestCase):
    class Desktop:
        def __init__(self): self.actions=[]; self.next=iter([observation(1,document=True),observation(2),
            observation(3,"digital-2"),observation(4,"digital-3"),observation(5,"digital-4"),
            observation(6,"image-only-1",document=True),observation(7,"image-only-1")]); self.frames=iter(P.PAGE_KEYS)
        def dispatch(self,actions,**kwargs): self.actions.append(actions)
        def discover(self,steps): self.actions.append(steps)
        def observe(self,bindings): return next(self.next)
        def page_area(self,area): pass
        def capture(self): return frame(next(self.frames))
    class Clipboard:
        def __init__(self): self.calls=[]
        def copy(self,nonce,dispatch):
            self.calls.append(nonce); dispatch(); return b"",{"transfer":{"complete":True}}

    def test_two_identical_fresh_original_sessions_execute_all_pages_and_copies(self):
        with tempfile.TemporaryDirectory() as tmp:
            outcomes=[]
            for index in (1,2):
                output=Path(tmp)/str(index); output.mkdir(); desktop=self.Desktop(); clipboard=self.Clipboard()
                workflow=S.Workflow(profile(),P.Ledger(60),desktop,clipboard,output,IO.Meter())
                result=workflow.run(index); outcomes.append(result)
                self.assertEqual((result["counts"]["pages_completed"],result["counts"]["copy_completed"]),(5,2))
                self.assertEqual(len(list(output.glob("*.ppm"))),5); self.assertEqual(len(clipboard.calls),2)
            self.assertEqual([p["decoded"] for p in outcomes[0]["pages"]],[p["decoded"] for p in outcomes[1]["pages"]])

    def test_failed_frame_preserved_first_failure_stops_remaining_pages_and_copy(self):
        with tempfile.TemporaryDirectory() as tmp:
            desktop=self.Desktop(); desktop.capture=lambda:frame("digital-2")
            clipboard=self.Clipboard(); workflow=S.Workflow(profile(),P.Ledger(60),desktop,clipboard,tmp,IO.Meter())
            with self.assertRaises(P.Refusal): workflow.run(1)
            self.assertTrue((Path(tmp)/"digital-1.ppm").exists())
            self.assertEqual((workflow.counts["pages_failed"],workflow.counts["pages_remaining"],len(clipboard.calls)),(1,4,0))

    def test_interrupted_capture_counts_unverified_and_never_admits_sixth_frame(self):
        with tempfile.TemporaryDirectory() as tmp:
            desktop=self.Desktop(); desktop.capture=mock.Mock(side_effect=KeyboardInterrupt)
            ledger=P.Ledger(60); workflow=S.Workflow(profile(),ledger,desktop,self.Clipboard(),tmp,IO.Meter())
            with self.assertRaises(KeyboardInterrupt): workflow.run(1)
            self.assertEqual((workflow.counts["pages_attempted"],workflow.counts["pages_unverified"]),(1,1))
            self.assertEqual(ledger.counts["capture"],1)

    def test_cleanup_unknown_survivor_and_primary_vs_closing_are_separate(self):
        baseline={1:1,10:2}
        with mock.patch.object(S,"process_identities",return_value={1:1,10:2,11:3}), \
                mock.patch.object(S,"process_identity",return_value=3),mock.patch.object(S.os,"kill") as kill, \
                mock.patch.object(S.time,"sleep"), self.assertRaises(P.Refusal): S.close_owned_tree(baseline)
        self.assertEqual(kill.call_count,3)
        with mock.patch.object(S,"process_identities",return_value=baseline):
            self.assertEqual(S.close_owned_tree(baseline)["remaining"],0)
        report={"protocol":"original-pdf-capabilities-v1","status":"FAIL","first_failure":{"reason":"original"},
                "pages":["x"*P.RECEIPT_LIMIT],"admitted":{"launcher":1},"application_launches":1,"counts":{},"cleanup":"FAIL"}
        refusal=json.loads(S.receipt_bytes(report)); self.assertFalse(refusal["receipt_complete"])
        self.assertEqual(refusal["first_failure"],report["first_failure"])


def runtime_fixture(mutate=None):
    """Independent original data shaped like the public producer metadata.

    These 2731 directory records are fabricated controls, not vendor hashes.
    All bindings are rebuilt after a deliberate semantic mutation so semantic
    controls are not merely testing the outer byte pin.
    """
    p=profile(); producer=b"# SPDX-License-Identifier: MIT\n# original test producer, never executed\n"
    declared=lambda path: {"path":path,**pin(b"original control declaration")}
    plan={"schema":"cajviewer-public-runtime-inventory-preparation/1","status":"FROZEN","image_id":p["image"],
          "history":[],"public_dependencies":[],"wrapper":{"path":"/original/producer.py",**pin(producer)},
          "inline_entry":declared("/original/inline.py"),"protocol_doc":declared("/original/protocol.md"),
          "tools":{"docker":declared("/original/docker"),"python":declared("/original/python")},
          "public_inventory_transport":{"source":declared("/original/inventory.py")},
          "source_root":"/original/source","sources":[declared("control.py")]}
    inventory={"status":"PASS","scope":"opaque-runtime-inventory-only","files":[
        {"path":"/original/control/"+str(index).zfill(6),"mode":0o755,"type":"directory"} for index in range(2731)],
        "tools":{name:{"path":"/original/tools/"+name,**pin()} for name in R.TOOLS},
        "observations":{"packages":"original-package\t1\tall\n","fontconfig":"/original/font\tOriginal\tRegular\n"},
        "files_hashed_bytes":0,"app_launches":0,"vendor_passes":0}
    envpin=pin(b"original environment identity")
    pending={"plan":pin(blob(plan)),"pid":123,"parent_environment_identity":envpin,"child_environment_identity":envpin}
    token={"schema":plan["schema"],"decision":"APPROVE","root_review":True,"independent_review":True,"pid":123,
           "plan":pin(blob(plan)),"pending_preflight":pin(blob(pending)),"parent_environment":envpin,"child_environment":envpin}
    source_rows=[]
    for ordinal,(_,role,path,size,sha) in enumerate(STARTUP.PUBLIC_SOURCE_PINS,1):
        identity={"size_bytes":size,"sha256":sha}
        source_rows.append({"ordinal":ordinal,"role":role,"path":path,"expected_identity":identity,"actual_identity":identity,
            "stages":dict.fromkeys(("read","pin","compile","exec"),"PASS"),"status":"PASS","failure_stage":None,"reason":None,"error_type":None})
    p["runtime_sources"]={row["path"].rsplit("/",1)[-1]:row["actual_identity"] for row in source_rows}
    loading={"observation":"COMPLETE_ORDERED_RECORDS","declared_sources":2,"records_attempted":2,"records_completed":2,
             "records_failed":0,"records_remaining":0,"records":source_rows,
             "stages":{stage:{"attempted":2,"completed":2,"failed":0} for stage in ("read","pin","compile","exec")}}
    caps={"memory_max_bytes":512*P.MIB,"memory_swap_max_bytes":0,"memory_peak_bytes":1000,"pids_max":64,"pids_peak":6,
          "cpu_period":100000,"cpu_quota":200000,"memory_events":{"oom":0,"oom_kill":0}}
    records=[plan["wrapper"],plan["inline_entry"],plan["protocol_doc"],*plan["tools"].values(),plan["public_inventory_transport"]["source"],
             {**plan["sources"][0],"path":"/original/source/control.py"}]
    audits=[]
    for phase in ("public-preflight","post-token-before-Docker","final"):
        for row in records:
            audits.append({"phase":phase,"ordinal":len(audits)+1,"path":row["path"],"status":"PASS",
                           "identity":{k:row[k] for k in ("size_bytes","sha256")}})
    actions=[]
    for index,stage in enumerate(("name-absent","create-one-inventory-container","one-inventory-start-attach","remove-owned-container","name-final-absent"),1):
        actions.append({"ordinal":index,"stage":stage,"status":"PASS","helper_status":"PASS","spawned":True,"exit_code":0,
                        "bytes_read":{"stdout":0,"stderr":0},"stdout":{**pin(b""),"complete":True,"hash_scope":"retained-stream"},
                        "stderr":{**pin(b""),"complete":True,"hash_scope":"retained-stream"}})
    nested=[]
    for index,name in enumerate(("packages","fontconfig"),1):
        raw=inventory["observations"][name].encode("utf-8")
        nested.append({"ordinal":index,"argv":R.NESTED_COMMANDS[index-1],"status":"PASS","helper_status":"PASS",
            "spawned":True,"exit_code":0,"bytes_read":{"stdout":len(raw),"stderr":0},
            "captures":{"stdout":{**pin(raw),"complete":True,"hash_scope":"retained-stream"},
                        "stderr":{**pin(b""),"complete":True,"hash_scope":"retained-stream"}}})
    receipt={"schema":plan["schema"],"status":"PASS","scope":"public-runtime-inventory-only","inventory_attempts":1,
        "inventory_completed":1,"app_launches":0,"vendor_passes":0,"docker_launch_attempts":5,"docker_successful_spawns":5,
        "actions":actions,"nested_helpers":{"observation":"COMPLETE_ENVELOPE","attempted":2,"spawned":2,"spawn_unknown":0,"actions":nested},
        "comparison":{"status":"PASS","records_planned":2731,"records_attempted":2731,"records_passed":2731,
            "records_failed":0,"records_remaining":0,"tools":"PASS","packages_fonts":"PASS","declared_byte_total":"PASS"},
        "public_source_loading":loading,"before_audit":"PASS","after_audit":"PASS","output_integrity":"PASS",
        "container_creation":"VERIFIED_CREATED","cleanup":"REMOVE_COMMAND_PASS","final_container_absence":"PASS",
        "file_audits":audits,"container_environment":{"uid_gid":[1000,1000],"uid_gid_after":[1000,1000],
            "closing_audits":{"caps":"PASS","environment":"PASS","user":"PASS"},"closing_failures":[],
            "environment_identity":envpin,"environment_after_identity":envpin},"container_caps":{"caps_before":caps,"caps_after":copy.deepcopy(caps)},
        "closing_checks":[{"kind":"dynamic-pin","status":"PASS"} for _ in range(3)]+[{"kind":"host-environment","status":"PASS"}],
        "output_audit_counts":{"planned":4,"attempted":4,"passed":4,"failed":0,"remaining":0},
        "retained_output_expectations":[{"file":"original-output-"+str(index),"device":1,"inode":index+1,"mode":0o400,**pin(b"")}
                                        for index in range(4)],
        "output_audits":[{"status":"PASS","file":"original-output-"+str(index),"identity":pin(b"")} for index in range(4)]}
    objects={"receipt":receipt,"plan":plan,"pending":pending,"token":token,"inventory":inventory}
    if mutate: mutate(objects)
    # Rebind actual raw identities as a real fixture author would. Review
    # assertions themselves remain unchanged except in their explicit controls.
    pending["plan"]=pin(blob(plan)); token["plan"]=pin(blob(plan)); token["pending_preflight"]=pin(blob(pending))
    data={key:blob(value) for key,value in objects.items() if key!="receipt"}; data["producer"]=producer
    for key in ("plan","pending","token","inventory"): receipt[key]=pin(data[key])
    receipt["retained_output_expectations"][3].update(pin(data["inventory"]))
    receipt["output_audits"][3]["identity"]=pin(data["inventory"])
    data["receipt"]=blob(receipt); pins={key:pin(value) for key,value in data.items()}
    for role,schema in (("root_review","cajviewer-root-v12-operational-closing/1"),("independent_review","cajviewer-independent-v12-operational-closing/1")):
        review={"schema":schema,"review_status":"PASS","operational_status":"PASS","actual_receipt":pins["receipt"],
                "image_id":p["image"],"bindings":{"frozen_plan":pins["plan"],**{key:pins[key] for key in ("pending","token","inventory","producer")}},
                "checks":dict.fromkeys(R.CHECKS,"PASS")}
        data[role]=blob(review); pins[role]=pin(data[role])
    p["runtime_view"]=pins
    return data,pins,p


class RuntimeGateTests(unittest.TestCase):
    def test_actual_producer_shape_complete_original_fixture_passes_metadata_gate_only(self):
        data,pins,p=runtime_fixture()
        self.assertEqual(R.validate_runtime(data,pins,p)["scope"],"closed-pinned-runtime-view-only")

    def test_historical_fail_and_zero_partial_or_float_comparison_never_authorize(self):
        changes=[lambda o:o["receipt"].update(status="FAIL"),lambda o:o["receipt"].update(inventory_completed=0),
                 lambda o:o["receipt"]["comparison"].update(records_attempted=2730,records_remaining=1),
                 lambda o:o["receipt"]["comparison"].update(records_passed=2731.0),
                 lambda o:o["receipt"]["comparison"].update(tools="NOT_RUN")]
        for change in changes:
            data,pins,p=runtime_fixture(change)
            with self.subTest(change=change),self.assertRaises(P.Refusal): R.validate_runtime(data,pins,p)

    def test_missing_second_source_wrong_order_path_pin_or_compile_failure_refuses(self):
        def mutate_source(o,kind):
            rows=o["receipt"]["public_source_loading"]["records"]
            if kind=="missing": rows.pop()
            elif kind=="order": rows.reverse()
            elif kind=="path": rows[1]["path"]="/original/unverified.py"
            elif kind=="pin": rows[1]["actual_identity"]=pin(b"wrong")
            else: rows[1]["stages"]["compile"]="FAIL"
        for kind in ("missing","order","path","pin","compile"):
            data,pins,p=runtime_fixture(lambda o:mutate_source(o,kind))
            with self.subTest(kind=kind),self.assertRaises(P.Refusal): R.validate_runtime(data,pins,p)

    def test_three_ordered_audits_and_final_env_caps_output_ownership_are_mandatory(self):
        changes=[lambda o:o["receipt"]["file_audits"].pop(),lambda o:o["receipt"]["file_audits"][0].update(phase="final"),
                 lambda o:o["receipt"]["container_environment"].update(uid_gid_after=[0,0]),
                 lambda o:o["receipt"]["container_caps"]["caps_after"].update(pids_max=64.0),
                 lambda o:o["receipt"]["container_caps"]["caps_after"]["memory_events"].update(oom_kill=1),
                 lambda o:o["receipt"].update(final_container_absence="FAIL"),lambda o:o["receipt"]["output_audits"][0].update(status="FAIL"),
                 lambda o:o["receipt"]["retained_output_expectations"][0].update(mode=0o600),
                 lambda o:o["receipt"]["output_audits"][0].update(file="different-output")]
        for change in changes:
            data,pins,p=runtime_fixture(change)
            with self.subTest(change=change),self.assertRaises(P.Refusal): R.validate_runtime(data,pins,p)

    def test_consumed_plan_token_inventory_and_independent_actual_closing_bindings(self):
        data,pins,p=runtime_fixture(); wrong=copy.deepcopy(data)
        review=json.loads(wrong["independent_review"]); review["checks"]["source_loading"]="PENDING"
        wrong["independent_review"]=blob(review); pins["independent_review"]=pin(wrong["independent_review"])
        with self.assertRaises(P.Refusal): R.validate_runtime(wrong,pins,p)
        for change in (lambda o:o["token"].update(pid=124),lambda o:o["token"].update(independent_review=False),
                       lambda o:o["inventory"]["files"].pop()):
            data,pins,p=runtime_fixture(change)
            with self.subTest(change=change),self.assertRaises(P.Refusal): R.validate_runtime(data,pins,p)

    def test_raw_receipt_pin_mismatch_rejects_even_with_all_statuses_pass(self):
        data,pins,p=runtime_fixture(); data["receipt"]+=b"\n"
        with self.assertRaisesRegex(P.Refusal,"runtime-blob-pin-mismatch"): R.validate_runtime(data,pins,p)

    def test_nested_actual_producer_counts_order_outputs_and_inventory_metadata_are_bound(self):
        changes=[lambda o:o["receipt"]["nested_helpers"].update(attempted=1),
                 lambda o:o["receipt"]["nested_helpers"].update(spawn_unknown=1),
                 lambda o:o["receipt"]["nested_helpers"]["actions"].reverse(),
                 lambda o:o["receipt"]["nested_helpers"]["actions"][0].update(argv=["other-helper"]),
                 lambda o:o["receipt"]["nested_helpers"]["actions"][0]["captures"]["stdout"].update(complete=False),
                 lambda o:o["receipt"]["nested_helpers"]["actions"][1]["bytes_read"].update(stdout=0),
                 lambda o:o["inventory"]["observations"].update(packages="changed\n"),
                 lambda o:o["inventory"]["tools"].pop("Xvfb"),
                 lambda o:o["inventory"]["tools"]["python3"].update(size_bytes=True)]
        for change in changes:
            data,pins,p=runtime_fixture(change)
            with self.subTest(change=change),self.assertRaises(P.Refusal): R.validate_runtime(data,pins,p)


class HostTests(unittest.TestCase):
    @staticmethod
    def result(stdout=b"",status="PASS",code=0,stderr=b""):
        return {"status":status,"exit_code":code,"stdout":stdout,"stderr":stderr,
                "bytes_read":{"stdout":len(stdout),"stderr":len(stderr)},"prefix_truncated":False}

    def test_stderr_bearing_create_has_no_positive_ownership_or_arbitrary_removal(self):
        cid="a"*64; calls=[]
        def command(argv,**kwargs):
            calls.append(argv)
            if "create" in argv: return self.result((cid+"\n").encode(),stderr=b"unexpected closing text")
            return self.result()
        # A stderr-bearing create cannot positively prove ownership, so no
        # arbitrary name/ID may be removed as a fallback.
        with tempfile.TemporaryDirectory() as tmp:
            host=H.Host(60,IO.Meter(),command)
            receipt=H.attempt(host,1,profile(),ROOT,ROOT,Path(tmp)/"p",Path(tmp)/"a",Path(tmp))
        self.assertIsNone(receipt["container_id"]); self.assertFalse(any("rm" in call for call in calls))

    def test_positive_creation_survives_deadline_refusal_and_is_removed_by_id(self):
        cid="b"*64; clock=mock.Mock(return_value=0); calls=[]
        def command(argv,**kwargs):
            calls.append(argv)
            if "create" in argv:
                clock.return_value=100
                return self.result((cid+"\n").encode())
            if "inspect" in argv: return self.result(blob(container_fixture(cid,calls[1][1:],profile())))
            return self.result()
        with tempfile.TemporaryDirectory() as tmp:
            host=H.Host(60,IO.Meter(),command,clock=clock)
            receipt=H.attempt(host,1,profile(),ROOT,ROOT,Path(tmp)/"p",Path(tmp)/"a",Path(tmp))
        self.assertEqual(receipt["container_id"],cid); self.assertEqual(receipt["cleanup"],"PASS")
        self.assertIn(["/usr/bin/docker","rm","--force",cid],calls)
        self.assertEqual(receipt["application_launches"],0)

    def test_prefix_whitespace_or_untyped_create_never_adopts_id_or_removes_by_name(self):
        for stdout,prefix,code,stderr in ((b"a"*64+b"\n",True,0,b""),(b" "+b"a"*64+b"\n",False,0,b""),
                                  (b"a"*64+b"\n\n",False,0,b""),(b"a"*64+b" ",False,0,b""),(b"a"*64,False,False,b""),
                                  (b"a"*64,False,0,None)):
            calls=[]
            def command(argv,**kwargs):
                calls.append(argv)
                result=self.result(stdout,code=code) if "create" in argv else self.result()
                if "create" in argv: result.update(prefix_truncated=prefix,stderr=stderr)
                return result
            with tempfile.TemporaryDirectory() as tmp,self.subTest(stdout=stdout[-3:],prefix=prefix,code=code):
                host=H.Host(60,IO.Meter(),command)
                receipt=H.attempt(host,1,profile(),ROOT,ROOT,Path(tmp)/"p",Path(tmp)/"a",Path(tmp))
            self.assertIsNone(receipt["container_id"]); self.assertEqual(receipt["cleanup"],"UNVERIFIED_NO_OWNED_ID")
            self.assertFalse(any("rm" in call for call in calls))

    def test_create_escape_never_claims_zero_owned_runtime_or_removes_foreign_name(self):
        calls=[]
        def command(argv,**kwargs):
            calls.append(argv)
            if "create" in argv: raise KeyboardInterrupt
            return self.result()
        with tempfile.TemporaryDirectory() as tmp:
            host=H.Host(60,IO.Meter(),command)
            receipt=H.attempt(host,1,profile(),ROOT,ROOT,Path(tmp)/"p",Path(tmp)/"a",Path(tmp))
        self.assertEqual(receipt["cleanup"],"UNVERIFIED_NO_OWNED_ID")
        self.assertTrue(host.first_failure); self.assertFalse(any("rm" in call for call in calls))

    def test_exact_delivery_is_readonly_no_pull_and_environment_is_explicit(self):
        args=H.create_argv("original-capability-test",profile(),ROOT,ROOT,ROOT/"profile",ROOT/"approval")
        self.assertEqual(args[args.index("--pull")+1],"never")
        self.assertIn("none",args); self.assertIn("--read-only",args)
        binds=[args[i+1] for i,value in enumerate(args) if value=="--mount"]
        self.assertTrue(all(value.endswith(",readonly") for value in binds))
        self.assertEqual(len(binds),len(H.SOURCE_PATHS)+4)
        self.assertIn("-i",args); self.assertNotIn("run.py --protocol",args)

    def test_malformed_or_missing_explicit_gate_refuses_before_any_host_command(self):
        with mock.patch.object(H,"run_bounded") as command,contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(H.main(["--profile","/original/missing"]),1)
        command.assert_not_called(); self.assertEqual(json.loads(output.getvalue())["vendor_passes"],0)
        for key,value in (("prefix_truncated",None),("stderr",None)):
            result=self.result(); result[key]=value
            host=H.Host(60,IO.Meter(),mock.Mock(return_value=result))
            with self.subTest(key=key),self.assertRaises((P.Refusal,TypeError)): host.call(["container","ls"],stage="original-helper")
            self.assertTrue(host.first_failure); self.assertNotEqual(host.actions[0].get("hash_scope"),"complete-stream")

    def test_final_container_config_must_remain_exact_before_owned_removal(self):
        p=profile(); argv=H.create_argv("original-capability-test",p,ROOT,ROOT,ROOT/"p",ROOT/"a")
        original=container_fixture("c"*64,argv,p)
        self.assertEqual(H.validate_container(blob(original),"c"*64,"original-capability-test",p,argv)["status"],"PASS")
        for key,value in (("Privileged",True),("Memory",P.CAPS["memory_bytes"]*2),("PidsLimit",256.0),("Devices",[{}])):
            wrong=copy.deepcopy(original); wrong[0]["HostConfig"][key]=value
            with self.subTest(key=key),self.assertRaises(P.Refusal): H.validate_container(blob(wrong),"c"*64,"original-capability-test",p,argv)

    def test_publication_failure_terminal_retains_known_actions_and_primary_failure(self):
        retained={"new_application_launches":1,"host_helper_admissions":11,"sessions_attempted":1,
                  "first_failure":{"reason":"original-primary"},"closing_errors":[{"reason":"publication"}]}
        argv=[]
        for name in ("profile","runtime-packet","approval","source-root","controls-root","output"):
            argv.extend(["--"+name,"/original/metadata"])
        argv.extend(["--profile-size","1","--profile-sha256","a"*64])
        with mock.patch.object(H,"execute",side_effect=H.PublicationFailure(retained)),contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(H.main(argv),1)
        result=json.loads(output.getvalue()); self.assertEqual(result["new_application_launches"],1)
        self.assertEqual(result["host_helper_admissions"],11); self.assertEqual(result["first_failure"],retained["first_failure"])

    def test_count_adoption_requires_complete_typed_matching_session_and_full_launcher_ledger(self):
        ledger=P.Ledger(60,clock=lambda:0)
        ledger.perform("launcher","official-launcher",lambda:None)
        original={"protocol":profile()["protocol"],"session_index":1,"receipt_complete":True,"application_launches":1,**ledger.summary()}
        self.assertEqual(H.validate_launch_accounting(original,profile(),1),1)
        changes=[lambda r:r.update(application_launches=0),lambda r:r.update(application_launches=False),
                 lambda r:r.update(receipt_complete=False),lambda r:r.update(session_index=1.0),
                 lambda r:r["events"][0].update(name="other-launcher"),lambda r:r["events"][0].update(index=1),
                 lambda r:r["events"].clear(),lambda r:r["admitted"].update(launcher=True)]
        for change in changes:
            report=copy.deepcopy(original); change(report)
            with self.subTest(change=change),self.assertRaises(P.Refusal): H.validate_launch_accounting(report,profile(),1)

    def test_actual_attempt_keeps_unknown_after_malformed_zero_and_adopts_proved_prelaunch_zero(self):
        # This complete zero is emitted by the actual production supervisor
        # after an original prelaunch refusal, with no real child admitted.
        complete=SessionSupervisorTests().run_failure(mock.Mock(side_effect=AssertionError),baseline_failure=True)
        self.assertEqual(H.validate_launch_accounting(complete,profile(),1),0)
        malformed=copy.deepcopy(complete)
        malformed["admitted"]["launcher"]=1
        malformed["events"].append({"index":len(malformed["events"]),"kind":"launcher","name":"official-launcher",
                                    "status":"UNKNOWN_AFTER_ADMISSION"})
        for report,expected in ((malformed,None),(complete,0)):
            cid="d"*64; calls=[]; declared=[]
            payload=blob(report)
            wire=IO.MAGIC
            for name,data in (("session.json",payload),("ready",b"capability-receipt-complete\n")):
                wire+=struct.pack("<BQ",len(name),len(data))+name.encode()+data
            wire+=b"\0"
            def command(argv,**kwargs):
                calls.append(argv)
                if "create" in argv:
                    declared.extend(argv[1:]); return self.result((cid+"\n").encode())
                if "inspect" in argv:
                    if "--format" in argv:
                        return self.result((cid+" /"+declared[declared.index("--name")+1]+"\n").encode())
                    return self.result(blob(container_fixture(cid,declared,profile())))
                if "--produce" in argv:
                    kwargs["stdout_sink"].write(wire)
                    result=self.result(); result["bytes_read"]["stdout"]=len(wire); return result
                if "rm" in argv and expected is None: return self.result(status="FAIL",code=1,stderr=b"original closing failure")
                return self.result()
            with tempfile.TemporaryDirectory() as tmp,self.subTest(expected=expected):
                host=H.Host(60,IO.Meter(),command)
                result=H.attempt(host,1,profile(),ROOT,ROOT,Path(tmp)/"p",Path(tmp)/"a",Path(tmp))
            self.assertEqual(result["reported_application_launches"],0)
            self.assertEqual(result["application_launches"],expected)
            self.assertEqual(result["reported_first_failure"],complete["first_failure"])
            self.assertEqual(result["status"],"FAIL")
            self.assertIn(["/usr/bin/docker","rm","--force",cid],calls)
            if expected is None:
                self.assertEqual(result["launcher_slot"],"ADMITTED_UNKNOWN_AFTER_START")
                self.assertEqual(result["first_failure"]["reason"],"launch-count-event-mismatch")
                self.assertTrue(host.closing_errors)
            else:
                self.assertEqual(result["launch_accounting"]["status"],"VERIFIED_COUNT_AND_LEDGER")


def container_fixture(cid,argv,p):
    """Original Docker inspect transcript derived from the declared argv."""
    name=argv[argv.index("--name")+1]
    mounts=[]
    for index,arg in enumerate(argv):
        if arg=="--mount":
            parts=dict(part.split("=",1) for part in argv[index+1].split(",") if "=" in part)
            mounts.append({"Type":"bind","Source":parts["src"],"Destination":parts["dst"],"RW":False})
    tmpfs={argv[i+1].split(":",1)[0]:argv[i+1].split(":",1)[1] for i,arg in enumerate(argv) if arg=="--tmpfs"}
    host={"NetworkMode":"none","ReadonlyRootfs":True,"Memory":P.CAPS["memory_bytes"],"MemorySwap":P.CAPS["memory_swap_bytes"],
          "NanoCpus":2000000000,"PidsLimit":256,"ShmSize":P.CAPS["shm_bytes"],"CgroupnsMode":"private","IpcMode":"private",
          "Init":True,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges"],"Tmpfs":tmpfs,
          "Ulimits":[{"Name":"core","Soft":0,"Hard":0},{"Name":"fsize","Soft":67108864,"Hard":67108864}]}
    return [{"Id":cid,"Name":"/"+name,"Image":p["image"],"HostConfig":host,"Mounts":mounts,
             "Config":{"User":"1000:1000","Hostname":"original-capability","Entrypoint":["/usr/bin/env"],
                       "WorkingDir":"/home/canary","Cmd":argv[argv.index(p["image"])+1:]}}]


class CollectedEvidenceTests(unittest.TestCase):
    def test_host_uses_actual_retained_grid_identity_event_and_work_counts_not_pass_summary(self):
        p=profile()
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); records={}
            def write(name,payload):
                (root/name).write_bytes(payload); records[name]=pin(payload)
            copies=[]
            for key in P.PAGE_KEYS: write(key+".ppm",frame(key))
            for key in P.COPY_KEYS:
                wire=SelectionTranscript(b"RUST" if key=="digital-1" else b"")
                clipboard=C.Clipboard(wire,P.Ledger(60,clock=lambda:0),p["clipboard"],owner_allowed=lambda owner:owner==77)
                payload,evidence=clipboard.copy("1:"+key,wire.copied); evidence["page_key"]=key
                copies.append(evidence); write(key+".clipboard",payload)
            raw=[observation(1,document=True),observation(2),observation(3,"digital-2"),observation(4,"digital-3"),
                 observation(5,"digital-4"),observation(6,"image-only-1",document=True),observation(7,"image-only-1")]
            observed={"workflow":[{"serial":row["serial"],"owner":list(row["owner"]),"raw":{
                       key:{"encoding":"base64","data":base64.b64encode(value).decode()} for key,value in row.items() if type(value) is bytes}}
                       for row in raw],"discovery":[]}
            write("observations.json",blob(observed)); write("ready",b"capability-receipt-complete\n")
            for name in ("application.log","xvfb.log","window-manager.log"): write(name,b"")
            write("display.json",b"{}")
            envpin=pin(S.encoded(p["environment"]))
            environment={"status":"PASS","uid_gid":[1000,1000],"identity":envpin}
            limits={"status":"PASS","memory_max_bytes":P.CAPS["memory_bytes"],"memory_swap_max_bytes":0,"pids_max":256,
                    "file_limits":[67108864]*2,"core_limits":[0,0],"cpu_period":100000,"cpu_quota":200000}
            report={"protocol":p["protocol"],"receipt_complete":True,"application_launches":1,"session_index":1,
                    "status":"CAPABILITIES_OBSERVED_ACTIVE_SETTINGS_UNVERIFIED","first_failure":None,"closing_errors":[],
                    "vendor_passes":0,"process_cleanup":{"status":"PASS"},"environment_before":environment,
                    "environment_after":environment,"limits_before":limits,
                    "limits_after":limits,"oom_kill_delta":0,"pages":[{"page_key":key} for key in P.PAGE_KEYS],"copies":copies,
                    "counts":{"pages_attempted":5,"pages_completed":5,"pages_failed":0,"pages_unverified":0,"pages_remaining":0,
                              "copy_attempted":2,"copy_completed":2,"copy_failed":0,"copy_unverified":0,"copy_remaining":0}}
            ledger=P.Ledger(60,clock=lambda:0); ledger.perform("launcher","official-launcher",lambda:None)
            report.update(ledger.summary())
            write("session.json",blob(report))
            self.assertEqual(len(H.validate_collected(root,records,p,IO.Meter())["pages"]),5)
            # Each mutation rebuilds the artifact pin; the independent semantic
            # gates must fail even though the supplied report still says PASS.
            report["counts"]["pages_completed"]=4; write("session.json",blob(report))
            with self.assertRaises(P.Refusal): H.validate_collected(root,records,p,IO.Meter())
            report["counts"]["pages_completed"]=5
            report["copies"][0]["selection_clear_hex"]="00"*32; write("session.json",blob(report))
            with self.assertRaises(P.Refusal): H.validate_collected(root,records,p,IO.Meter())


if __name__ == "__main__": unittest.main()
