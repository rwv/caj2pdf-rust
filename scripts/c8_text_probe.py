#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Stream a diagnostic prefix of measured raw C8 native-text records.

This is not a text extractor or renderer: glyph events are provisional, source
order is not reading order, font words are opaque, and unknown/vector/image
records stop the probe. No source text or whole-page buffer is retained.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import unicodedata

from hnc8_layout_source import FileInput, SourceExtractor, SourceLimits


def character(code: int) -> str | None:
    # The special A0xx Latin range is still unverified; do not decode it as
    # GB18030 private-use characters and pretend that those are source text.
    if code >> 8 == 0xa0:
        return None
    try:
        value = code.to_bytes(2, "big").decode("gb18030")
    except UnicodeError:
        return None
    if len(value) != 1 or unicodedata.category(value) in {"Co", "Cn", "Cc"}:
        return None
    return value


def probe(source, offset: int, length: int, emit, *, max_records=65536,
          cancelled=lambda: False) -> dict:
    """Read fixed records only while the independently observed grammar holds."""
    if not 0 <= offset <= source.size or not 0 <= length <= 1024 * 1024:
        raise ValueError("text span outside bounded source profile")
    if offset + length > source.size or not 0 < max_records <= 65536:
        raise ValueError("invalid span or record budget")
    position = offset
    end = offset + length
    records = glyphs = 0
    y = style = None

    def result(status, **extra):
        return {"status": status, "offset": position, "records": records,
                "glyphs": glyphs, "complete_text": False, **extra}

    while position < end:
        if records == max_records:
            return result("LIMIT", reason="record budget")
        if end - position < 4:
            return result("MALFORMED", reason="partial four-byte record")
        record = bytearray()
        while len(record) < 4:
            if cancelled():
                return result("CANCELLED")
            block = source.read_at(position + len(record), 4 - len(record))
            if not block or len(block) > 4 - len(record):
                return result("MALFORMED", reason="short or overreported source")
            record.extend(block)
        tag, payload = struct.unpack("<HH", record)
        records += 1
        if tag == 0x8004:
            return result("TERMINATOR", opaque_tail_bytes=end - position - 4)
        if tag == 0x8001:
            y = payload
        elif tag == 0x8002:
            style = payload
        elif tag == 0x801d and payload == 0:
            pass  # Only the measured neutral control is traversed.
        elif tag < 0x8000:
            if y is None or style is None:
                return result("UNSUPPORTED", reason="glyph lacks position/style context")
            emit({"kind": "provisional_glyph", "offset": position,
                  "x_word": tag, "y_word": y, "style_word": style,
                  "code_word": payload, "unicode_candidate": character(payload)})
            glyphs += 1
        else:
            return result("UNSUPPORTED", tag=tag,
                          reason="unverified control, vector or image record")
        position += 4
    return result("MALFORMED", reason="missing terminator")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--sha256", required=True, help="expected source SHA-256")
    parser.add_argument("--page", type=int, required=True)
    parser.add_argument("--max-records", type=int, default=65536)
    args = parser.parse_args()
    with args.source.open("rb") as stream:
        before = hashlib.file_digest(stream, "sha256").hexdigest()
    if before != args.sha256:
        raise ValueError("source SHA-256 differs")
    emit = lambda value: print(json.dumps(value, ensure_ascii=False), flush=True)
    with FileInput(args.source) as source:
        reader = SourceExtractor(source, args.source.name,
                                 SourceLimits(max_text_span_bytes=1024 * 1024))
        if reader.header["variant"] != "C8":
            raise ValueError("this diagnostic accepts only C8")
        page = reader.read_page(args.page)
        result = probe(source, page["text_offset"], page["text_length"], emit,
                       max_records=args.max_records)
    with args.source.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != before:
            raise ValueError("source changed during diagnostic")
    emit({"kind": "summary", "source_sha256": before, "page": args.page, **result})
    return 0 if result["status"] == "TERMINATOR" else 1


if __name__ == "__main__":
    raise SystemExit(main())
