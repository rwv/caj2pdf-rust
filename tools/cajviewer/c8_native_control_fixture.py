#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original two-glyph controls for additional native C8 framing."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

from c8_encoded_prefix_fixture import document


CONTROLS = (
    ("baseline", ()),
    ("80ce-0", (0x80CE, 0)), ("80ce-1", (0x80CE, 1)),
    ("801c-4", (0x801C, 4)), ("801d-3", (0x801D, 3)),
    ("8024-2800", (0x8024, 0x2800)), ("8024-281d", (0x8024, 0x281D)),
    ("8021-2000", (0x8021, 0x2000)),
    ("80d0-0", (0x80D0, 0)), ("80d1-1", (0x80D1, 1)),
    ("80d2-0", (0x80D2, 0)),
    ("8070-4", (0x8070, 4)), ("8071-4", (0x8071, 4)),
    ("81ff-1", (0x81FF, 1, 0, 200)),
    ("81ff-2", (0x81FF, 2, 0, 200)),
    ("81ff-3", (0x81FF, 3, 0, 200)),
    ("80cc-0204", (0x80CC, 0x0204, 33, 5)),
)


def control_document(words):
    base = document(None)
    # Preserve the run controls and first glyph; place one control before a
    # second, asymmetric glyph. In the original geometric font these are a
    # full square and upper half-square, so losing either is visible.
    records = base[100:120] + struct.pack("<" + "H" * len(words), *words)
    records += struct.pack("<HHHH", 4802, 0xCEC4, 0x8004, 1)
    header = bytearray(base[:100])
    struct.pack_into("<I", header, 84, len(records))
    struct.pack_into("<I", header, 96, 100 + len(records))
    return bytes(header + records)


def mixed_documents():
    """Original controls with independently distinguishable font resources."""
    from c8_image_fixture import jpeg, mixed_control

    labels = {"baseline", "80ce-0", "80ce-1", "8021-2000", "80d0-0",
              "80d1-1", "80d2-0", "81ff-1", "81ff-2", "81ff-3", "80cc-0204"}
    for state in (0, 4):
        for label, control in CONTROLS:
            if label not in labels:
                continue
            data = mixed_control(jpeg(), control)
            if state == 0:
                data = data.replace(struct.pack("<HH", 0x801D, 4),
                                    struct.pack("<HH", 0x801D, 0))
            yield f"mixed-{state}-{label}.caj", data


def color_documents():
    """Discriminate the required 81ff payload from unverified colors."""
    from c8_image_fixture import jpeg, mixed_control

    # Historical probe names are hypotheses, not established RGB meanings.
    controls = (
        ("baseline", ()),
        ("all", (0x81FF, 1, 0, 200, 0x81FF, 2, 0, 200, 0x81FF, 3, 0, 200)),
        ("third", (0x81FF, 3, 0, 200)),
        ("first-red", (0x81FF, 1, 255, 200)),
        ("second-red", (0x81FF, 2, 255, 200)),
        ("all-gray", (0x81FF, 1, 0x4444, 0x44, 0x81FF, 2, 0x4444, 0x44,
                      0x81FF, 3, 0x4444, 0x44)),
    )
    for label, control in controls:
        yield f"color-{label}.caj", mixed_control(jpeg(), control)
    for value in (1, 2, 3):
        data = bytearray(mixed_control(jpeg()))
        length = struct.unpack_from("<I", data, 84)[0]
        data[100:100] = struct.pack("<4H", 0x81FF, value, 0, 200)
        struct.pack_into("<I", data, 84, length + 8)
        struct.pack_into("<I", data, 96, len(data))
        descriptor = 100 + length + 8
        struct.pack_into("<I", data, descriptor + 4, descriptor + 12)
        yield f"color-once-{value}.caj", data


def mode_documents():
    """Distinguish persistent CJK mode from resource selection and reset."""
    from c8_image_fixture import jpeg, mixed_control

    for state in (0, 4):
        for label, control in (
            ("baseline", None), ("once-zero", None),
            ("zero-one", (0x80CE, 0, 0x80CE, 1)),
            ("one-zero", (0x80CE, 1, 0x80CE, 0)),
        ):
            data = bytearray(mixed_control(jpeg(), control))
            data = data.replace(struct.pack("<HH", 0x801D, 4),
                                struct.pack("<HH", 0x801D, state))
            if label == "once-zero":
                length = struct.unpack_from("<I", data, 84)[0]
                data[100:100] = struct.pack("<HH", 0x80CE, 0)
                struct.pack_into("<I", data, 84, length + 4)
                struct.pack_into("<I", data, 96, len(data))
                descriptor = 100 + length + 4
                struct.pack_into("<I", data, descriptor + 4, descriptor + 12)
            yield f"mode-{state}-{label}.caj", data


def extended_string_documents():
    """Check atomic metadata payloads in both ordinary and CJK modes."""
    from c8_image_fixture import jpeg, mixed_control

    for mode in (0, 1):
        for label, payload in (
            ("baseline", None), ("source342", (342, 5)),
            ("source420", (420, 7)), ("zero", (0, 0)),
            ("max", (65535, 65535)), ("tag", (0x8004, 1)),
        ):
            control = [0x80CE, mode]
            if payload is not None:
                control.extend((0x80CC, 0x0204, *payload))
            yield f"extended-{mode}-{label}.caj", mixed_control(jpeg(), control)


def font_state_documents():
    """Separate extended Latin-resource states from fullwidth glyph mapping."""
    from c8_image_fixture import jpeg, mixed_control

    for state in (0, 4, 28, 31):
        for label, code in (("ordinary", 0xA0C1), ("required", 0xA3CA)):
            data = mixed_control(jpeg()).replace(
                struct.pack("<HH", 0x801D, 4), struct.pack("<HH", 0x801D, state))
            data = data.replace(struct.pack("<HH", 4972, 0xA0C1),
                                struct.pack("<HH", 4972, code))
            yield f"font-{state}-{label}.caj", data

    for states in ((28, 31, 0), (31, 28, 4)):
        parts = mixed_control(jpeg()).split(struct.pack("<HH", 0x801D, 4))
        assert len(parts) == 4
        data = parts[0] + b"".join(
            struct.pack("<HH", 0x801D, state) + part
            for state, part in zip(states, parts[1:]))
        yield "transition-" + "-".join(map(str, states)) + ".caj", data


def alphabet_documents():
    """Compare all fullwidth Latin letters to the same-position CJK resource."""
    for state, mode in ((0, 1), (4, 1), (28, 1), (31, 1), (31, 0)):
        for kind, first in (("upper", 0xA3C1), ("lower", 0xA3E1), ("cjk", None)):
            header = bytearray(80)
            struct.pack_into("<IIII", header, 0, 200, 0, 1, 2)
            header[16:28] = "北大二扫1.00".encode("gbk")
            struct.pack_into("<HHHH", header, 28, 4652, 4274, 800, 1000)
            words = [(0x8002, 0x10A5), (0x801D, state), (0x80CE, mode), (0x8067, 6)]
            for index in range(28):
                row, column = divmod(index, 4)
                if column == 0:
                    words.append((0x8001, 4334 + row * 130))
                code = first + index if first is not None and index < 26 else 0xD6D0
                words.append((4672 + column * 170, code))
            words.append((0x8004, 1))
            records = b"".join(struct.pack("<HH", *pair) for pair in words)
            data = header + struct.pack("<IIIII", 100, len(records), 0, 0, 100 + len(records)) + records
            yield f"alphabet-{state}-{mode}-{kind}.caj", data


def field4_style_documents():
    """Hold resource/mode/position constant and vary only observed style bits."""
    from c8_style_fixture import document as style_document

    for mode in (0, 1):
        for style in (0x1084, 0x1484, 0x1085):
            data = style_document(
                [(style, 0, 6), (style, 4, 6), (style, 28, 6)],
                codes=(0xD6D0, 0xA0C1), width=800, height=600,
                first_x=4672, first_y=4334, row_step=130,
                run_words=(0x80CE, mode))
            yield f"style-{style:04x}-mode-{mode}.caj", data


def state_axis_documents():
    """Isolate state persistence, axis overrides, and style reset in C8."""
    from c8_style_fixture import document as style_document

    controls = (
        ("base", ()), ("state", (0x801C, 4)),
        ("axes4", (0x8070, 4, 0x8071, 4)),
        ("state-axes4", (0x801C, 4, 0x8070, 4, 0x8071, 4)),
        ("width4", (0x8070, 4)), ("height4", (0x8071, 4)),
        ("axes36", (0x8070, 36, 0x8071, 36)),
        ("axes36-state", (0x8070, 36, 0x8071, 36, 0x801C, 4)),
        ("axes4-reset", (0x8070, 4, 0x8071, 4, 0x8002, 0x10A5)),
        ("state-axes4-reset", (0x801C, 4, 0x8070, 4, 0x8071, 4, 0x8002, 0x10A5)),
    )
    for style in (0x1084, 0x10A5):
        for label, words in controls:
            yield f"axis-{style:04x}-{label}.caj", style_document(
                [(style, 0, 6), (style, 4, 6)], codes=(0xD6D0, 0xA0C1),
                width=800, height=600, first_x=4672, first_y=4334,
                row_step=130, run_words=words)
    for size in (3, 4, 5, 36):
        yield f"axis-detail-{size}.caj", style_document(
            [(0x1084, 0, 6)], codes=(), width=200, height=200, first_y=4334,
            run_words=(0x801C, 4, 0x8070, size, 0x8071, size,
                       4672, 0xD6D0, 4752, 0xA0C1))


def at_sign_documents():
    """Compare the required fullwidth at sign to verified comma placement."""
    for name, source in alphabet_documents():
        if "-1-upper" not in name:
            continue
        state = name.split("-")[1]
        for label, code in (("at", 0xA3C0), ("comma", 0xA3AC)):
            data = bytearray(source)
            for offset in range(100, len(data), 4):
                x, value = struct.unpack_from("<HH", data, offset)
                if x < 0x8000 and 0xA3C1 <= value <= 0xA3DA:
                    struct.pack_into("<H", data, offset + 2, code)
            yield f"at-{state}-{label}.caj", data


def record_9002_documents():
    """Check the observed four-byte control in black ordinary/CJK mixed pages."""
    from c8_image_fixture import jpeg, mixed_control

    for mode in (0, 1):
        for state in (0, 4):
            for kind in ("base", "record"):
                words = [0x80CE, mode, 0x81FF, 1, 0, 200]
                if kind == "record":
                    words.extend((0x9002, 0))
                data = mixed_control(jpeg(), words).replace(
                    struct.pack("<HH", 0x801D, 4), struct.pack("<HH", 0x801D, state))
                yield f"record-{mode}-{state}-{kind}.caj", data


def additional_style_documents():
    """Compare required field-5/6 flags and unequal-axis controls in both modes."""
    from c8_style_fixture import document as style_document

    for mode, styles in ((None, (0x10C6, 0x14C6, 0x10C5)),
                         (1, (0x04C6,)), (0, (0x10C6, 0x04C6, 0x10C5)),
                         (0, (0x10A5, 0x14A5, 0x10A4)),
                         (1, (0x10A5, 0x14A5, 0x10A4))):
        for style in styles:
            suffix = "" if mode is None else f"-mode-{mode}"
            codes = (0xD6D0, 0xA0C1, 0xD6D0 if mode == 0 else 0xAAB3)
            yield f"style-{style:04x}{suffix}.caj", style_document(
                [(style, 0, 6), (style, 4, 6), (style, 28, 6)],
                codes=codes, width=1000, height=700,
                first_x=4672, first_y=4334, row_step=180,
                run_words=() if mode is None else (0x80CE, mode))


def skew_281c_documents():
    """Measure C8 shear using unequal axes, a large glyph, and state controls."""
    from hnb_geometry_fixture import skew_control
    from c8_style_fixture import document as style_document

    for style in (0x1067, 0x10E3, 0xE58C):
        for value in (0x2800, 0x281C, 0x281D):
            yield f"skew-{style:04x}-{value:04x}.caj", skew_control(
                (0x8024, value), style, c8_container=True)
    for label, words in (
        ("base", ()), ("active", (0x8024, 0x281C)),
        ("reset", (0x8024, 0x281C, 0x8024, 0x2800)),
        ("style", (0x8024, 0x281C, 0x8002, 0x1084)),
    ):
        yield f"skew-state-{label}.caj", skew_control(words, c8_container=True)
        yield f"skew-latin-{label}.caj", style_document(
            [(0x1084, 0, 6)], codes=(), width=400, height=400, first_y=4374,
            run_words=words + (4672, 0xD6D0, 4852, 0xA0C1))


def low_letter_documents():
    """Distinguish the observed low-byte letter from ordinary Latin rendering."""
    for name, source in alphabet_documents():
        if "-1-upper" not in name or name.split("-")[1] not in ("0", "4"):
            continue
        state = name.split("-")[1]
        for label, code in (("low", 0x006C), ("latin", 0xA0EC)):
            data = bytearray(source)
            for offset in range(100, len(data), 4):
                x, value = struct.unpack_from("<HH", data, offset)
                if x < 0x8000 and 0xA3C1 <= value <= 0xA3DA:
                    struct.pack_into("<H", data, offset + 2, code)
            yield f"low-{state}-{label}.caj", data
            if label != "low":
                continue
            mode0 = bytearray(data)
            struct.pack_into("<H", mode0, 110, 0)
            yield f"low-{state}-mode0.caj", mode0
            words, y, reset = [], None, False
            for offset in range(100, len(data), 4):
                x, value = struct.unpack_from("<HH", data, offset)
                if x == 0x8001:
                    y, reset = value, False
                    value += 15
                elif x < 0x8000 and value == 0x006C:
                    value = 0xD6D0
                elif x < 0x8000 and value == 0xD6D0 and not reset:
                    words.append((0x8001, y))
                    reset = True
                words.append((x, value))
            reference = data[:100] + b"".join(struct.pack("<HH", *pair) for pair in words)
            struct.pack_into("<I", reference, 84, len(reference) - 100)
            struct.pack_into("<I", reference, 96, len(reference))
            yield f"low-{state}-shifted-cjk.caj", reference
            if state == "0":
                for variant, original in (("low", data), ("shifted-cjk", reference)):
                    skew = bytearray(original)
                    skew[116:116] = struct.pack("<HH", 0x8024, 0x281C)
                    struct.pack_into("<I", skew, 84, len(skew) - 100)
                    struct.pack_into("<I", skew, 96, len(skew))
                    yield f"low-{state}-{variant}-skew.caj", skew


def field1_documents():
    """Isolate small glyph axes and Latin baseline with original marker fonts."""
    from c8_style_fixture import document as style_document

    for label, style, words in (
        ("field1", 0x1021, ()), ("field2", 0x1042, ()),
        ("axis24", 0x1042, (0x8070, 24, 0x8071, 24)),
        ("width1", 0x1022, ()), ("height1", 0x1041, ()),
    ):
        yield label + ".caj", style_document(
            [(style, 0, 6)], codes=(), width=200, height=200,
            first_y=4334, run_words=(*words, 4672, 0xD6D0, 4752, 0xA0C1))


def small_bracket_documents():
    """Separate bracket resource/axes from overlapping neighboring glyphs."""
    from c8_style_fixture import document as style_document

    for style, dx, dy in ((0x1021, 21, 3), (0x1022, 21, 1), (0x1041, 24, 3)):
        for state in (0, 4):
            for label, code in (("open", 0xA3DB), ("close", 0xA3DD)):
                yield f"{style:04x}-{state}-{label}.caj", style_document(
                    [(style, state, 6)], codes=(), width=200, height=200,
                    first_y=4334, run_words=(4672, code))
        yield f"{style:04x}-reference.caj", style_document(
            [(style, 0, 6)], codes=(), width=200, height=200,
            first_y=4334 + dy - 15, run_words=(4672 + dx, 0xA3AC))


def state3_documents():
    """Identify state-3 resources, switching and persistence independently."""
    from c8_style_fixture import document as style_document

    for state in (0, 3, 4):
        for mode in (0, 1):
            yield f"state-{state}-mode-{mode}.caj", style_document(
                [(0x10A5, state, 6)], codes=(0xD6D0, 0xA0C1, 0xA3F3, 0xA3A8),
                width=1600, height=500, first_x=4672, first_y=4334,
                run_words=(0x80CE, mode))
    for label, words in (
        ("transitions", (0x80CE, 1, 4672, 0xA0C1, 0x801D, 4, 5022, 0xA0C1,
                         0x801D, 3, 5372, 0xA0C1, 0x801D, 0, 5722, 0xA0C1)),
        ("persistence", (0x80CE, 1, 4672, 0xA0C1, 0x80CE, 0, 5022, 0xA0C1,
                         0x80CE, 1, 5372, 0xA0C1, 0x8002, 0x10A5, 5722, 0xA0C1)),
    ):
        yield label + ".caj", style_document(
            [(0x10A5, 3, 6)], codes=(), width=1600, height=500,
            first_y=4334, run_words=words)


def required_greek_documents():
    """Compare required Greek letters to symbol and Latin baseline controls."""
    from c8_style_fixture import document as style_document

    for state in (0, 3, 4):
        yield f"greek-{state}.caj", style_document(
            [(0x10A5, state, 6)], codes=(0xA6C5, 0xA6C8, 0xA3AC, 0xA0C1),
            width=1600, height=500, first_x=4672, first_y=4334,
            run_words=(0x80CE, 1))


def radical_record_documents():
    """Investigate observed radical drawing framing without admitting it."""
    from c8_style_fixture import document as style_document

    for label, words in (
        ("base", ()),
        ("record", (0x8090, 0xA3E6, 0xC000 | 4702, 4364, 0xC08F, 125)),
        ("short", (0x8090, 0xA3E6, 0xC000 | 4702, 4364, 0xC08F, 60)),
        ("moved", (0x8090, 0xA3E6, 0xC000 | 4762, 4404, 0xC08F, 125)),
        ("narrow", (0x8090, 0xA3E6, 0xC000 | 4702, 4364, 0xC040, 125)),
        ("unflagged", (0x8090, 0xA3E6, 4702, 4364, 143, 125)),
    ):
        yield f"radical-{label}.caj", style_document(
            [(0x1021, 3, 6)], codes=(), width=600, height=500, first_y=4334,
            run_words=(*words, 0x8001, 4514, 0x8002, 0x10A5,
                       4672, 0xD6D0, 4922, 0xA0C1))


def radical_detail_documents():
    """Separate radical hook geometry from dimensions, style and translation."""
    from c8_style_fixture import document as style_document

    for label, style, width, height, x, y, state in (
        ("base", 0x1021, 143, 125, 4802, 4354, 3),
        ("style5", 0x10A5, 143, 125, 4802, 4354, 3),
        ("wide", 0x1021, 250, 125, 4802, 4354, 3),
        ("tall", 0x1021, 143, 200, 4802, 4354, 3),
        ("small", 0x1021, 64, 60, 4802, 4354, 3),
        ("holdout", 0x1084, 190, 90, 4782, 4374, 4),
    ):
        yield f"radical-detail-{label}.caj", style_document(
            [(style, state, 6)], codes=(), width=400, height=400,
            first_y=4334,
            run_words=(0x8090, 0xA3E6, 0xC000 | x, y, 0xC000 | width, height))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for label, words in CONTROLS:
        data = control_document(words)
        name = f"control-{label}.caj"
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
    for name, data in (*mixed_documents(), *color_documents(), *mode_documents(),
                       *extended_string_documents(), *font_state_documents(),
                       *alphabet_documents(), *field4_style_documents(),
                       *state_axis_documents(), *at_sign_documents(),
                       *record_9002_documents(), *additional_style_documents(),
                       *skew_281c_documents(), *low_letter_documents(),
                       *field1_documents(), *small_bracket_documents(),
                       *state3_documents(), *required_greek_documents(),
                       *radical_record_documents(), *radical_detail_documents()):
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
