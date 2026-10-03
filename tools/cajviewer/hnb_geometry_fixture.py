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
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
