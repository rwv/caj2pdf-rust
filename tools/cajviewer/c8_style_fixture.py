#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original, fixed-position C8 controls for manual viewer observations."""

import argparse
import hashlib
import json
from pathlib import Path
import struct


# These are observed raw values, not an interpreted font/style API.
VARIANTS = (
    ("baseline", 0x1084, 0, 6),
    ("vertical", 0x1085, 0, 6),
    ("horizontal", 0x10A4, 0, 6),
    ("weight", 0x1084, 4, 6),
    ("font5", 0x1084, 0, 5),
    ("font8", 0x1084, 0, 8),
    ("font9", 0x1084, 0, 9),
    ("highbits", 0x0884, 0, 6),
)


def document(styles, control_record=None, drawing=None, codes=None, *, row_step=500, height=7469):
    """One original page, with 中文AM1 as the default test string."""
    if codes is None:
        codes = (0xD6D0, 0xCEC4, 0xA0C1, 0xA0CD, 0xA0B1)
    records = bytearray()
    for row, (style, control, font) in enumerate(styles):
        pairs = [
            (0x8001, 4700 + row * row_step),
            (0x8002, style),
            (0x801D, control),
            (0x8067, font),
        ]
        if control_record is not None:
            pairs.append(control_record)
        if drawing is not None:
            tag, value, delta = drawing
            y = 4800 + row * row_step
            drawing_pairs = [(tag, value), (5200 + delta, y), (6300, y + 50)]
            if value != 0xA383:
                drawing_pairs.append((0xFFFF, 5))
            pairs = drawing_pairs + pairs
        pairs.extend(
            (5200 + column * 350, code)
            for column, code in enumerate(codes)
        )
        for pair in pairs:
            records.extend(struct.pack("<HH", *pair))
    records.extend(struct.pack("<HH", 0x8004, 1))
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 0xC8, 0, 1, 2)
    # Observed format identifier; no source document text or font data is copied.
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652, 4274, 5105, height)
    index = struct.pack("<IIIII", 100, len(records), 0, 0, 100 + len(records))
    return bytes(header + index + records)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    fixtures = [("grid", [v[1:] for v in VARIANTS], None, None)]
    fixtures.extend(
        (name, [(style, control, font)] * 8, None, None)
        for name, style, control, font in VARIANTS
    )
    baseline = [(0x1084, 0, 6)] * 8
    fixtures.extend(
        (name, baseline, (tag, value), None)
        for name, tag, value in (
            ("control72", 0x8072, 0), ("control73", 0x8073, 38),
            ("control74", 0x8074, 0), ("control53", 0xC053, 5200),
            ("control54", 0xC054, 5200), ("control53shift", 0xC053, 5700),
            ("control54shift", 0xC054, 5700),
        )
    )
    fixtures.extend(
        (name, baseline, None, (tag, value, delta))
        for name, tag, value, delta in (
            ("draw10", 0x8010, 1, 0), ("draw10shift", 0x8010, 1, 200),
            ("draw06", 0x8006, 0xA381, 0),
            ("draw06compact", 0x8006, 0xA383, 0),
            ("draw06alternate", 0x8006, 0xA38B, 0),
        )
    )
    symbols = {
        "letter-a": (0xA0C1,),
        "digit-one": (0xA0B1,),
        "size-squares": (0xA1F6, 0xA1F5, 0xCCEF, 0xB9FA, 0xD6D0),
        "symbols": (0xAAB3, 0xA0A6, 0xACA3, 0xA3A6, 0xA3AA),
        "symbols-permuted": (0xACA3, 0xA3AA, 0xA0A6, 0xA3A6, 0xAAB3),
    }
    fixtures.extend([
        ("letter-a", baseline, None, None),
        ("digit-one", baseline, None, None),
        ("size-ladder", [(0x1000 | (index << 5) | index, 0, 6)
                         for index in range(1, 13)], None, None),
        ("size-squares", [(0x1000 | (index << 5) | index, 0, 6)
                          for index in range(3, 11)], None, None),
        ("symbols", baseline, None, None),
        ("symbols-permuted", [(0x1084, 4, 6)] * 8, None, None),
    ])
    fixtures.append(("size-profile", [(0x1000 | (index << 5) | index, 0, 6)
                                      for index in (2, 3, 4, 5, 6, 8)], None, None))
    symbols["size-profile"] = (0xD6D0, 0xA0C1)
    manifest = []
    for name, styles, control_record, drawing in fixtures:
        geometry = {"row_step": 350, "height": 3200} if name == "size-profile" else {}
        data = document(styles, control_record, drawing, symbols.get(name), **geometry)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({
            "name": name, "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(), "rows": styles,
            "control_record": control_record, "drawing": drawing,
            "symbol_codes": symbols.get(name),
            "geometry": geometry,
        })
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
