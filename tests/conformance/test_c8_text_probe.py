# SPDX-License-Identifier: MIT
"""Original tiny native-text controls; never use the external corpus in CI."""

from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from c8_text_probe import character, probe


class Source:
    def __init__(self, records, short=4):
        self.bytes = b"".join(struct.pack("<HH", *r) for r in records)
        self.size = len(self.bytes)
        self.short = short
        self.requests = []

    def read_at(self, offset, count):
        self.requests.append(count)
        return self.bytes[offset:offset + min(count, self.short)]


class NativeTextProbeTests(unittest.TestCase):
    def run_probe(self, records, **kwargs):
        source = Source(records, short=1)
        events = []
        result = probe(source, 0, source.size, events.append, **kwargs)
        self.assertLessEqual(max(source.requests, default=0), 4)
        self.assertFalse(result["complete_text"])
        return result, events

    def test_source_order_positions_and_unicode_are_not_reading_order(self):
        result, events = self.run_probe([
            (0x801d, 0), (0x8001, 4500), (0x8002, 4200),
            (7000, 0xcee4), (5000, 0xb2e2), (0x8001, 4700),
            (6000, 0xcec4), (0x8004, 0), (0x800a, 0),
        ])
        self.assertEqual(result["status"], "TERMINATOR")
        self.assertEqual(result["opaque_tail_bytes"], 4)
        self.assertEqual([x["x_word"] for x in events], [7000, 5000, 6000])
        self.assertEqual([x["y_word"] for x in events], [4500, 4500, 4700])
        self.assertEqual([x["unicode_candidate"] for x in events], ["武", "测", "文"])

    def test_unknowns_stop_instead_of_reinterpreting_payload_as_glyphs(self):
        for tag, payload in [(0x8006, 0xa381), (0x800a, 0), (0x801d, 4), (0xffff, 5)]:
            result, events = self.run_probe([(0x8001, 1), (0x8002, 2),
                                             (tag, payload), (70, 0xcec4)])
            self.assertEqual(result["status"], "UNSUPPORTED")
            self.assertEqual(result["offset"], 8)
            self.assertEqual(events, [])

    def test_uncertain_codes_are_explicit_not_replacement_characters(self):
        for code in [0xa0c4, 0xaab3, 0xffff, 0, 0x800a]:
            self.assertIsNone(character(code))
        result, events = self.run_probe([(0x8001, 1), (0x8002, 2),
                                         (70, 0xa0c4), (0x8004, 0)])
        self.assertIsNone(events[0]["unicode_candidate"])
        self.assertEqual(result["glyphs"], 1)

    def test_limits_cancellation_truncation_and_missing_context(self):
        self.assertEqual(self.run_probe([(0x8001, 1), (0x8004, 0)], max_records=1)[0]["status"], "LIMIT")
        self.assertEqual(self.run_probe([(0x8001, 1)], cancelled=lambda: True)[0]["status"], "CANCELLED")
        self.assertEqual(self.run_probe([(70, 0xcec4)])[0]["status"], "UNSUPPORTED")
        self.assertEqual(self.run_probe([(0x8001, 1)])[0]["status"], "MALFORMED")
        source = Source([(0x8001, 1)])
        self.assertEqual(probe(source, 0, 3, lambda _: None)["status"], "MALFORMED")
        for offset, length, limit in [(-1, 4, 1), (0, 5, 1), (0, 4, 0), (0, 2**20+1, 1)]:
            with self.assertRaises(ValueError):
                probe(source, offset, length, lambda _: None, max_records=limit)
        source.short = 0
        self.assertEqual(probe(source, 0, 4, lambda _: None)["status"], "MALFORMED")


if __name__ == "__main__":
    unittest.main()
