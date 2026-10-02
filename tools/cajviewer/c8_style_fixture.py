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


def document(styles, control_record=None, drawing=None, codes=None, *, row_step=500, height=7469, drawing_dy=50, width=5105, first_x=5200, first_y=4700, run_words=(), omit_controls=()):
    """One original page, with 中文AM1 as the default test string."""
    if codes is None:
        codes = (0xD6D0, 0xCEC4, 0xA0C1, 0xA0CD, 0xA0B1)
    records = bytearray()
    for row, (style, control, font) in enumerate(styles):
        pairs = [
            (0x8001, first_y + row * row_step),
            (0x8002, style),
            (0x801D, control),
            (0x8067, font),
        ]
        pairs = [pair for pair in pairs if pair[0] not in omit_controls]
        if control_record is not None:
            pairs.append(control_record)
        if drawing is not None:
            tag, value, delta = drawing
            y = 4800 + row * row_step
            drawing_pairs = [(tag, value), (5200 + delta, y), (6300, y + drawing_dy)]
            if value != 0xA383:
                drawing_pairs.append((0xFFFF, 5))
            pairs = drawing_pairs + pairs
        pairs.extend(zip(run_words[::2], run_words[1::2]))
        pairs.extend(
            (first_x + column * 350, code)
            for column, code in enumerate(codes)
        )
        for pair in pairs:
            records.extend(struct.pack("<HH", *pair))
    records.extend(struct.pack("<HH", 0x8004, 1))
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 0xC8, 0, 1, 2)
    # Observed format identifier; no source document text or font data is copied.
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652, 4274, width, height)
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
            # Nonzero, glyph-like payloads required by the first C8 profile.
            # Keep these raw controls distinct from the following glyph stream.
            ("control72-style", 0x8072, 0x1042),
            ("control72-punctuation", 0x8072, 0xA3A8),
            ("control72-latin", 0x8072, 0xA0F2),
            ("control74-han1", 0x8074, 0xB4A2),
            ("control74-han2", 0x8074, 0xD4B4),
            ("control74-low", 0x8074, 0x24A7),
            ("control74-space", 0x8074, 0xA1A1),
            ("control74-punctuation", 0x8074, 0xA3A9),
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
    fixtures.append(("draw10horizontal", baseline, None, (0x8010, 1, 0)))
    fixtures.append(("draw06a385", [(0x1084, 0, 6)], None, (0x8006, 0xA385, 0)))
    symbols = {
        "letter-a": (0xA0C1,),
        "digit-one": (0xA0B1,),
        "size-squares": (0xA1F6, 0xA1F5, 0xCCEF, 0xB9FA, 0xD6D0),
        "symbols": (0xAAB3, 0xA0A6, 0xACA3, 0xA3A6, 0xA3AA),
        "symbols-permuted": (0xACA3, 0xA3AA, 0xA0A6, 0xA3A6, 0xAAB3),
    }
    # Isolate resource selection from Unicode identity: the two ampersand
    # codes decode to the same Unicode character but need not use one font.
    for name, code in (("amp", 0xA0A6), ("star", 0xAAB3),
                       ("pointer", 0xACA3), ("fullamp", 0xA3A6),
                       ("fullstar", 0xA3AA), ("comma", 0xA3AC)):
        for weight in (0, 4):
            suffix = "-weight" if weight else ""
            name_with_state = f"symbol-role-{name}{suffix}"
            symbols[name_with_state] = (code,)
            fixtures.append((name_with_state, [(0x1084, weight, 6)] * 2, None, None))
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
    anchor_geometry = {}
    # Same-page pairs avoid comparing tab-specific zoom/sidebar state.
    name = "anchor-field2-same-page"
    fixtures.append((name, [(0x1042, 0, 6)], None, None))
    symbols[name] = ()
    anchor_geometry[name] = {
        "width": 300, "height": 250, "first_x": 4672, "first_y": 4294,
        "run_words": (
            4672, 0xD6D0, 4792, 0xA0C1,
            0x8001, 4394, 0x8002, 0x1042, 0x801D, 0, 0x8067, 6,
            4672, 0xD6D0, 4792, 0xA0C1,
        ),
    }
    # Hold page geometry fixed across the six sizes required by issue-66.
    # Both rows and both scripts remain unclipped, including the largest size.
    for field in (2, 3, 4, 5, 6, 7, 8):
        # Field 7 is a held-out size-model control, not a support claim.
        name = f"anchor-field{field}-large-page"
        style = 0x1000 | (field << 5) | field
        fixtures.append((name, [(style, 0, 6)], None, None))
        symbols[name] = ()
        anchor_geometry[name] = {
            "width": 500, "height": 500, "first_x": 4672, "first_y": 4294,
            "run_words": (
                4672, 0xD6D0, 4902, 0xA0C1,
                0x8001, 4524, 0x8002, style, 0x801D, 0, 0x8067, 6,
                4672, 0xD6D0, 4902, 0xA0C1,
            ),
        }
    # One-unit and long intervals distinguish content scaling from page framing.
    name = "coordinate-grid"
    fixtures.append((name, [(0x1042, 0, 6)], None, None))
    symbols[name] = ()
    words = []
    for y in (20, 170, 400):
        for delta, x in enumerate((20, 170, 320)):
            words.extend((0x8001, 4274 + y + delta, 4652 + x, 0xD6D0))
    anchor_geometry[name] = {
        "width": 500, "height": 500, "first_x": 4672, "first_y": 4294,
        "run_words": tuple(words),
    }
    for horizontal, vertical in ((3, 3), (3, 5), (5, 3), (5, 5)):
        for kind, code in (("cjk", 0xD6D0), ("latin", 0xA0C1)):
            name = f"axis-{kind}-{horizontal}-{vertical}"
            fixtures.append((name, [(0x1000 | (horizontal << 5) | vertical, 0, 6)], None, None))
            symbols[name] = (code,)
            anchor_geometry[name] = {"width": 150, "height": 100, "first_x": 4672, "first_y": 4294}
    # Separate omitted initial state from an explicit zero-valued control.
    # Required by the observed profile before its first font selection.
    for name, omitted in (
        ("initial-default", (0x801D, 0x8067)),
        ("initial-font-default", (0x8067,)),
        ("initial-weight-default", (0x801D,)),
    ):
        fixtures.append((name, baseline, None, None))
        anchor_geometry[name] = {"omit_controls": omitted}
    # Complete the observed ordinary-text weight/font combinations, holding
    # position, size and character codes fixed against the existing weight case.
    for font in (5, 8, 9):
        fixtures.append((f"weight-font{font}", [(0x1084, 4, font)] * 8, None, None))
    # Required non-Han codes in the first C8 profile. Positions and row content
    # are authored controls, not extracted document text. CJK and Latin anchors
    # separate geometry from resource choice in the all-alias marker fonts.
    role_codes = (
        0xA0A6, 0xA1A1, 0xA1A2, 0xA1A4, 0xA1AA, 0xA1AD, 0xA1AE, 0xA1AF,
        0xA1B0, 0xA1B1, 0xA1C6, 0xA1C8, *range(0xA2D9, 0xA2E0),
        0xA3A3, 0xA3A5, 0xA3A8, 0xA3A9, 0xA3AB, 0xA3AC, 0xA3AD, 0xA3AE,
        0xA3AF, *range(0xA3B0, 0xA3C0), 0xA3DB, 0xA3DC, 0xA3DD, 0xA3FB,
        0xA3FD, 0xA9AA, 0xAAB3, 0xACA3,
    )
    for code in role_codes:
        name = f"role-{code:04x}"
        words = []
        for row, weight in enumerate((0, 4)):
            words.extend((0x8001, 4294 + row * 230, 0x8002, 0x1084,
                          0x801D, weight, 0x8067, 6,
                          4672, 0xD6D0, 4872, 0xA0C1, 5072, code))
        fixtures.append((name, [(0x1084, 0, 6)], None, None))
        symbols[name] = ()
        anchor_geometry[name] = {"width": 600, "height": 600,
                                 "first_x": 4672, "first_y": 4294,
                                 "run_words": tuple(words)}
    for code in (0xA0A6, 0xA1A2, 0xA1A4, 0xA1AF, 0xA1B0,
                 0xA3A8, 0xA3A9, 0xA3BA, 0xA3DB, 0xACA3):
        name = f"role-sizes-{code:04x}"
        words = []
        for row, field in enumerate((2, 3, 4, 5, 6, 8)):
            words.extend((0x8001, 4344 + row * 230,
                          0x8002, 0x1000 | (field << 5) | field,
                          0x801D, 0, 0x8067, 6,
                          4672, 0xD6D0, 4872, 0xA0C1, 5072, code))
        fixtures.append((name, [(0x1084, 0, 6)], None, None))
        symbols[name] = ()
        anchor_geometry[name] = {"width": 800, "height": 1400,
                                 "first_x": 4672, "first_y": 4344,
                                 "run_words": tuple(words)}
    words = []
    for row, (width_field, height_field) in enumerate(((7, 7), (2, 8), (8, 2))):
        words.extend((0x8001, 4394 + row * 300,
                      0x8002, 0x1000 | (width_field << 5) | height_field,
                      0x801D, 4, 0x8067, 6, 4702, 0xD6D0, 5052, 0xA3BA))
    name = "role-colon-heldout"
    fixtures.append((name, [(0x1084, 0, 6)], None, None))
    symbols[name] = ()
    anchor_geometry[name] = {"width": 800, "height": 1100,
                             "run_words": tuple(words)}
    manifest = []
    for name, styles, control_record, drawing in fixtures:
        geometry = {"row_step": 350, "height": 3200} if name == "size-profile" else {}
        geometry.update(anchor_geometry.get(name, {}))
        if name == "draw10horizontal":
            geometry["drawing_dy"] = 0
        data = document(styles, control_record, drawing, symbols.get(name), **geometry)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({
            "name": name, "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(), "rows": styles,
            "control_record": control_record, "drawing": drawing,
            "symbol_codes": symbols.get(name),
            "geometry": geometry,
        })
    boundaries = [("baseline", ()), ("standalone", (0xFFFF, 5))]
    for value in (0xA381, 0xA385, 0xA38B):
        for suffix, following in (("bare", ()), ("footer", (0xFFFF, 5)),
                                  ("next-y", (0x8001, 5000))):
            boundaries.append((f"{value:x}-{suffix}",
                               (0x8006, value, 5200, 4800, 6300, 4850) + following))
    for suffix, words in boundaries:
        name = f"c8-drawing-boundary-{suffix}"
        data = document([(0x1084, 0, 6)], run_words=words)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, following in (("bare", ()), ("footer", (0xFFFF, 5)),
                              ("next-y", (0x8001, 5000))):
        name = f"c8-drawing10-boundary-{suffix}-horizontal-long"
        words = (0x8010, 1, 4900, 4800, 9200, 4800) + following
        data = document([(0x1084, 0, 6)], run_words=words)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for name, points in (
        ("short", (4900, 4800, 7050, 4800)),
        ("shift-y", (4900, 5300, 9200, 5300)),
        ("slope", (4900, 4800, 9200, 5300)),
        ("shift-x", (5200, 4800, 9500, 4800)),
        ("vertical", (4900, 4800, 4900, 6800)),
    ):
        name = "decoration-geometry-" + name
        words = (0x8010, 1) + points
        data = document([(0x1084, 0, 6)], run_words=words)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    words = []
    for row, style in enumerate((0xA381, 0xA383, 0xA38B)):
        for points in (((4682, 4304 + row * 70), (4832, 4304 + row * 70)),
                       ((4902 + row * 90, 4524), (4902 + row * 90, 4704))):
            words.extend((0x8006, style, *points[0], *points[1]))
    data = document([(0x1084, 0, 6)], codes=(), run_words=tuple(words),
                    width=600, height=600)
    name = "segment-axes"
    (args.output / f"{name}.caj").write_bytes(data)
    manifest.append({"name": name, "run_words": words, "bytes": len(data),
                     "sha256": hashlib.sha256(data).hexdigest()})
    for flags in (0x0800, 0x0C00, 0x1000):
        name = f"style-flags-{flags:04x}"
        styles = [(flags | field << 5 | field, 0, 6) for field in (2, 8)]
        data = document(styles, codes=(0xD6D0, 0xA0C1), width=600, height=600,
                        first_x=4672, first_y=4294, row_step=230)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "styles": styles, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    # Short spans distinguish repeated glyphs clipped at the endpoint from
    # whole-glyph admission. Keep six isolated rows on one small square page.
    lengths = (10, 50, 89, 91, 180, 430)
    words = []
    for row, length in enumerate(lengths):
        y = 4334 + row * 80
        words.extend((0x8010, 1, 4712, y, 4712 + length, y))
    name = "decoration-endpoints"
    data = document([(0x1084, 0, 6)], codes=(), run_words=tuple(words),
                    width=600, height=600)
    (args.output / f"{name}.caj").write_bytes(data)
    manifest.append({"name": name, "lengths": lengths, "run_words": words,
                     "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, control, font in (("state4", 4, 6), ("font9", 0, 9)):
        name = f"decoration-resource-{suffix}"
        words = (0x8010, 1, 4900, 4800, 9200, 4800)
        data = document([(0x1084, control, font)], codes=(), run_words=words)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "control": control, "font": font,
                         "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    # Isolate active size inheritance: no ordinary glyphs overlap the decoration.
    for horizontal, vertical in ((2, 2), (4, 4), (8, 8), (2, 8), (8, 2)):
        suffix = (f"size{horizontal}" if horizontal == vertical
                  else f"axis{horizontal}-{vertical}")
        name = f"decoration-inherited-{suffix}"
        style = 0x1000 | (horizontal << 5) | vertical
        words = (0x8010, 1, 4900, 4800, 9200, 4800)
        data = document([(style, 0, 6)], codes=(), run_words=words)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "style": style, "run_words": words,
                         "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
