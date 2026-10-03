#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original HN-B axis controls; no production geometry is inferred."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

from c8_style_fixture import document


CONTROLS = (
    ("equivalence-high04", 0x04E7, ()),
    ("equivalence-high14", 0x14E7, ()),
    ("equivalence-normal7", 0x10E7, ()),
    ("zero-none", 0, ()),
    ("zero-width36", 0, (0x8070, 36)),
    ("zero-height36", 0, (0x8071, 36)),
    ("zero-both72", 0, (0x8070, 72, 0x8071, 72)),
    ("normal-both72", 0x1084, (0x8070, 72, 0x8071, 72)),
    ("model-style4", 0x1084, ()),
    ("model-explicit35", 0, (0x8070, 35, 0x8071, 35)),
    ("model-explicit35-reset4", 0, (0x8070, 35, 0x8071, 35, 0x8002, 0x1084)),
    ("model-explicit72-reset4", 0, (0x8070, 72, 0x8071, 72, 0x8002, 0x1084)),
    ("model-style12", 0xE58C, ()),
    ("model-explicit112", 0, (0x8070, 112, 0x8071, 112)),
)


ANCHORS = (
    ("reference0", 0, ()),
    ("explicit16", 0, (0x8070, 16, 0x8071, 16)),
    ("explicit21", 0, (0x8070, 21, 0x8071, 21)),
    ("reference35", 0x1084, ()),
    ("explicit35", 0, (0x8070, 35, 0x8071, 35)),
    ("explicit36", 0, (0x8070, 36, 0x8071, 36)),
)


def control(style, words, *, anchor=False, codes=None):
    """Reuse original glyph records in a separately authored compact container."""
    if anchor:
        c8 = document(
            [(style, 0, 6)], codes=(), width=300, height=250,
            first_x=4672, first_y=4294,
            run_words=words + (4672, 0xD6D0, 4792, 0xA0C1),
        )
    else:
        c8 = document([(style, 0, 6)] * (1 if codes is not None else 3),
                      run_words=words, codes=codes)
    return hn_container(c8)


def hn_container(c8):
    header = bytearray(216)
    struct.pack_into("<III", header, 0, 0x4E48, 200, 136)
    struct.pack_into("<IIII", header, 136, 0, 0, 1, 2)
    header[152:164] = c8[16:28]
    header[164:172] = c8[28:36]
    return bytes(header) + struct.pack("<III", 228, len(c8) - 100, 0) + c8[100:]


def line_control(style, marked, *, diagonal=False, c8_container=False):
    x = 4757 | (0xc000 if marked else 0)
    c8 = bytearray(document(
        [(0x1084, 0, 6)], codes=(0xD6D0, 0xA0C1), width=2000, height=1000,
        drawing=(0x8006, style, x - 5200), drawing_dy=0,
    ))
    if diagonal:
        struct.pack_into("<HH", c8, 32, 400, 400)
        struct.pack_into("<HHHH", c8, 104, 4690 | (0xc000 if marked else 0),
                         4350, 4900, 4500)
    return bytes(c8) if c8_container else hn_container(c8)


def skew_control(words, style=0x1084, *, c8_container=False):
    data = document(
        [(style, 0, 6)], codes=(0xD6D0,), width=400, height=400,
        first_x=4672, first_y=4374, run_words=words,
    )
    return data if c8_container else hn_container(data)


RESOURCE_CONTROLS = (
    ("base", ()), ("resource7", (0x8067, 7)), ("state69", (0x8069, 0x1084)),
    ("statece", (0x80CE, 1)), ("state72", (0x8072, 0x1084)),
    ("state73-30", (0x8073, 30)), ("state73-31", (0x8073, 31)),
    ("state73-32", (0x8073, 32)), ("state74-a", (0x8074, 0xB7BD)),
    ("state74-b", (0x8074, 0xCFC8)), ("state74-c", (0x8074, 0xC8CB)),
    ("extended", (0xC052, 0xA385, 0xD290, 0xB675)),
)


def resource_control(words):
    return hn_container(document(
        [(0x1067, 0, 6), (0x10E3, 4, 6)], codes=(), width=1100, height=650,
        first_x=4672, first_y=4394, row_step=250,
        run_words=words + (4672, 0xD6D0, 4902, 0xA0C1, 5152, 0xA0AE,
                           0x8006, 0xA381, 4672, 4600, 5482, 4620),
    ))


def punctuation_control(style, alternate, code, *, c8_container=False):
    data = document([(style, alternate, 6)], codes=(code,), width=400, height=400,
                    first_x=4772, first_y=4374)
    return data if c8_container else hn_container(data)


def book_control(code, alternate, dx=0, x=4772, y=4374):
    return hn_container(document([(0x10A5, alternate, 6)], codes=(code,),
                                 width=400, height=400, first_x=x + dx, first_y=y))


def geometry_control(width, height, dx=0, dy=0, *, c8_container=False):
    data = bytearray(document(
        [(0x1084, 0, 6)], codes=(0xD6D0,), width=width, height=height,
        first_x=4672, first_y=4374,
    ))
    struct.pack_into("<HH", data, 28, 4652 + dx, 4274 + dy)
    return bytes(data) if c8_container else hn_container(data)


def end_controls():
    """Original terminal controls with visible continuation and unaligned tails."""
    def page(words):
        return document([(0x1084, 0, 6)], codes=(), width=600, height=400,
                        first_x=4672, first_y=4374, run_words=words)

    first = (4672, 0xD6D0)
    second = (4872, 0xCEC4)
    for name, words in (("base", ()), ("middle44", (0x8004, 44)),
                        ("middle45", (0x8004, 45)), ("middle1", (0x8004, 1)),
                        ("middle0", (0x8004, 0)), ("middleffff", (0x8004, 0xFFFF))):
        yield f"end-continuation-{name}", hn_container(page(first + words + second))

    one = page(first)
    both = page(first + second)[100:]
    middle = page(first + (0x8004, 44) + second)[100:]
    opaque = one[100:-4] + struct.pack("<HH", 0x8004, 44) + bytes.fromhex("ff0180fe03")
    for name, width, payloads in (
        ("one12", 12, [one[100:]]), ("opaque12", 12, [opaque]),
        ("both20", 20, [both]), ("middle20", 20, [middle]),
        ("one20", 20, [one[100:]]), ("opaque20", 20, [opaque]),
        ("two-pages20", 20, [opaque, both]),
    ):
        header = bytearray(hn_container(one)[:216])
        struct.pack_into("<I", header, 136, 0 if width == 12 else 200)
        struct.pack_into("<I", header, 144, len(payloads))
        offset = 216 + width * len(payloads)
        index = bytearray()
        for payload in payloads:
            index.extend(struct.pack("<III", offset, len(payload), 0))
            index.extend(bytes(width - 12))
            offset += len(payload)
        yield f"end-tail-{name}", bytes(header + index) + b"".join(payloads)


def issue63_style_controls():
    """Compare required styles against independent explicit-size controls."""
    ordinary = [(f"style{style:04x}", style, ()) for style in
                (0x154A, 0x114A, 0x0484, 0x1084, 0x9C84, 0x0C84, 0x1000, 0)]
    ordinary.extend((f"explicit{size}", 0, (0x8070, size, 0x8071, size))
                    for size in (35, 36, 78, 83, 84, 85, 87, 88))
    held = [("154a", 0x154A, ()), ("84", 0, (0x8070, 84, 0x8071, 84)),
            ("0484", 0x0484, ()), ("9c84", 0x9C84, ()), ("1084", 0x1084, ())]
    for prefix, controls, alternate, x, y, c8 in (
        ("issue63", ordinary, 0, 4672, 4374, False),
        ("issue63-held", held, 4, 4693, 4357, False),
        ("issue63-c8", held, 4, 4693, 4357, True),
    ):
        for name, style, words in controls:
            data = document([(style, alternate, 6)], codes=(), width=600, height=400,
                            first_x=x, first_y=y,
                            run_words=words + (x, 0xD6D0, x + 200, 0xA0C1))
            yield f"{prefix}-{name}", data if c8 else hn_container(data)


def native_mode_controls():
    """Original mode-only and alphabet controls; no glyph outlines are copied."""
    groups = ((0xA980, 0xA98A, 0xA996, 0xA99A, 0xA99B, 0xA99C, 0xA99D),
              (0xA99E, 0xA9A8, 0xA9AB, 0xA9AC, 0xA9AD, 0xA9B2))
    for index, codes in enumerate(groups):
        words = tuple(word for i, code in enumerate(codes) for word in (4672 + i * 100, code))
        for mode in (2, 0):
            data = bytearray(hn_container(document(
                [(0x1084, 0, 6)], codes=(), width=800, height=250,
                first_x=4672, first_y=4374, run_words=words)))
            struct.pack_into("<I", data, 148, mode)
            suffix = "" if mode == 2 else "mode0-"
            yield f"issue63-a9-{suffix}{index}", bytes(data)
    for name, codes in (("a9upper", range(0xA980, 0xA99A)),
                        ("a9lower", range(0xA99A, 0xA9B4)),
                        ("a3upper", range(0xA3C1, 0xA3DB)),
                        ("a3lower", range(0xA3E1, 0xA3FB)),
                        ("a0upper", range(0xA0C1, 0xA0DB))):
        words = []
        for i, code in enumerate(codes):
            if i == 13:
                words.extend((0x8001, 4474))
            words.extend((4672 + i % 13 * 65, code))
        data = bytearray(hn_container(document(
            [(0x1084, 0, 6)], codes=(), width=950, height=400,
            first_x=4672, first_y=4374, run_words=tuple(words))))
        struct.pack_into("<I", data, 148, 0)
        yield f"legacy-{name}", bytes(data)


def legacy_geometry_control(width=600, height=400, dx=0, dy=0, style=0x1084):
    data = bytearray(hn_container(document(
        [(style, 0, 6)], codes=(), width=width, height=height,
        first_x=100 + dx, first_y=100 + dy,
        run_words=(100 + dx, 0xD6D0, 300 + dx, 0xA3C1, 450 + dx, 0xA980))))
    struct.pack_into("<I", data, 148, 0)
    struct.pack_into("<HHHHHHHHHH", data, 152,
                     0x8003, width, 0x8003, height, 0x8003, 0, 0, 1, width, height)
    return bytes(data)


def legacy_geometry_controls():
    """Vary page metadata, positions, styles and optional resource controls."""
    for name, width, height, dx, dy in (
        ("base", 600, 400, 0, 0), ("wide", 800, 400, 0, 0),
        ("tall", 600, 600, 0, 0), ("shiftx", 600, 400, 30, 0),
        ("shifty", 600, 400, 0, 40),
    ):
        yield f"legacy-geometry-{name}", legacy_geometry_control(width, height, dx, dy)
    base = legacy_geometry_control()
    for name, offset, value in (
        ("originx", 164, 30), ("originy", 166, 41),
        ("extent-width", 168, 800), ("prefix-width", 154, 800),
        ("extent-height", 170, 600), ("prefix-height", 158, 600),
    ):
        data = bytearray(base)
        struct.pack_into("<H", data, offset, value)
        yield f"legacy-header-{name}", bytes(data)
    for style in (0, 0x0484, 0x04E7, 0x0884, 0x0CA4, 0x0CE7,
                  0x1000, 0x1084, 0x10A4, 0x154A, 0x9C84):
        yield f"legacy-style-{style:04x}", legacy_geometry_control(style=style)
    data = bytearray(base)
    del data[236:244]  # Remove authored 801d/8067 controls; retain glyph order.
    struct.pack_into("<I", data, 220, len(data) - 228)
    yield "legacy-default-resources", bytes(data)
    data = bytearray(base)
    struct.pack_into("<H", data, 238, 4)
    yield "legacy-alternate4", bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for prefix, controls, anchor in (("axis", CONTROLS, False), ("anchor", ANCHORS, True)):
        for name, style, words in controls:
            data = control(style, words, anchor=anchor)
            filename = f"{prefix}-{name}.caj"
            (args.output / filename).write_bytes(data)
            manifest.append({"file": filename, "style": style, "words": words,
                             "sha256": hashlib.sha256(data).hexdigest()})
    codes = (0xA0C1, 0xA0AE, 0xA0AF, 0xA0BA, 0xAAB1, 0xAAB2, 0xA0CD)
    for filename, order in (("unknown-symbols.caj", codes),
                            ("unknown-symbols-reversed.caj", codes[::-1])):
        data = control(0x1084, (), codes=order)
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "codes": order,
                         "sha256": hashlib.sha256(data).hexdigest()})
    for diagonal in (False, True):
        for name, style, marked in (("ordinary", 0xA381, False),
                                    ("a385-low", 0xA385, False),
                                    ("a385-marked", 0xA385, True)):
            filename = "line-" + ("diagonal-" if diagonal else "") + name + ".caj"
            data = line_control(style, marked, diagonal=diagonal)
            (args.output / filename).write_bytes(data)
            manifest.append({"file": filename, "style": style, "marked": marked,
                             "sha256": hashlib.sha256(data).hexdigest()})
    for name, words in (
        ("base", ()), ("skew", (0x8024, 0x281D)),
        ("reset", (0x8024, 0x281D, 0x8024, 0x2800)),
        ("style", (0x8024, 0x281D, 0x8002, 0x1084)),
    ):
        data = skew_control(words)
        filename = f"skew-{name}.caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
        data = skew_control(words, c8_container=True)
        filename = f"skew-c8-{name}.caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
    for style in (0x1067, 0x10E3, 0xE58C):
        for name, words in (("base", ()), ("skew", (0x8024, 0x281D))):
            data = skew_control(words, style)
            filename = f"skew-axis-{style:04x}-{name}.caj"
            (args.output / filename).write_bytes(data)
            manifest.append({"file": filename, "style": style, "words": words,
                             "sha256": hashlib.sha256(data).hexdigest()})
    for name, width, height, dx, dy in (
        ("base", 400, 400, 0, 0), ("origin-x", 400, 400, 30, 0),
        ("origin-y", 400, 400, 0, 40), ("wide", 500, 400, 0, 0),
        ("tall", 400, 500, 0, 0),
    ):
        for variant in ("c8", "hnb"):
            data = geometry_control(width, height, dx, dy, c8_container=variant == "c8")
            filename = f"geometry-{name}-{variant}.caj"
            (args.output / filename).write_bytes(data)
            manifest.append({"file": filename, "width": width, "height": height,
                             "origin": [4652 + dx, 4274 + dy],
                             "sha256": hashlib.sha256(data).hexdigest()})
    for name, words in RESOURCE_CONTROLS:
        data = resource_control(words)
        filename = f"resource-mixed-{name}.caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
    for style in (0x1067, 0x10E3):
        for alternate in (0, 4):
            for code in (0xA1A3, 0xA1B6, 0xA1B7, 0xA0A6, 0xA1A2, 0xA3A8):
                data = punctuation_control(style, alternate, code)
                filename = f"punct-{style:04x}-{alternate}-{code:04x}.caj"
                (args.output / filename).write_bytes(data)
                manifest.append({"file": filename, "style": style, "alternate": alternate,
                                 "code": code, "sha256": hashlib.sha256(data).hexdigest()})
            data = punctuation_control(style, alternate, 0xA1A3, c8_container=True)
            filename = f"punct-c8-{style:04x}-{alternate}.caj"
            (args.output / filename).write_bytes(data)
            manifest.append({"file": filename, "style": style, "alternate": alternate,
                             "sha256": hashlib.sha256(data).hexdigest()})
    books = [
        (f"book-five-{alt}-{name}", code, alt, dx, 4772, 4374)
        for alt in (0, 4)
        for name, code, dx in (("left", 0xA1B6, 0), ("right", 0xA1B7, 0),
                               ("paren", 0xA3A8, 0), ("left-candidate", 0xA3A8, 3),
                               ("right-candidate", 0xA3A8, -6))
    ]
    books.extend((f"book-confirm-{name}", code, alt, dx, x, y)
                 for name, code, dx, alt, x, y in (
        ("left4-0", 0xA3A8, 4, 0, 4772, 4374), ("left4-4", 0xA3A8, 4, 4, 4772, 4374),
        ("held-left", 0xA1B6, 0, 0, 4793, 4357), ("held-right", 0xA1B7, 0, 0, 4793, 4357),
        ("held-left-reference", 0xA3A8, 4, 0, 4793, 4357),
        ("held-right-reference", 0xA3A8, -6, 0, 4793, 4357),
    ))
    for name, code, alt, dx, x, y in books:
        data = book_control(code, alt, dx, x, y)
        filename = name + ".caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "code": code, "alternate": alt, "dx": dx,
                         "x": x, "y": y, "sha256": hashlib.sha256(data).hexdigest()})
    for name, data in (*end_controls(), *issue63_style_controls(), *native_mode_controls(),
                       *legacy_geometry_controls()):
        filename = name + ".caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
