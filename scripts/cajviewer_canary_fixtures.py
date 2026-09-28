#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Create original PDF controls for the external, offline CAJViewer canary.

These tiny development fixtures are not a document conversion API. All glyphs,
colors and PDF objects are authored here. No installed or bundled font is read.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import zlib


GLYPHS = {
    " ": ("00000",) * 7,
    "R": ("11110", "10001", "10001", "11110", "10100", "10010", "10001"),
    "U": ("10001", "10001", "10001", "10001", "10001", "10001", "01110"),
    "S": ("01111", "10000", "10000", "01110", "00001", "00001", "11110"),
    "T": ("11111", "00100", "00100", "00100", "00100", "00100", "00100"),
    "0": ("01110", "10001", "10011", "10101", "11001", "10001", "01110"),
    "1": ("00100", "01100", "00100", "00100", "00100", "00100", "01110"),
    "2": ("01110", "10001", "00001", "00010", "00100", "01000", "11111"),
    "3": ("11110", "00001", "00001", "01110", "00001", "00001", "11110"),
    "4": ("00010", "00110", "01010", "10010", "11111", "00010", "00010"),
    "5": ("11111", "10000", "10000", "11110", "00001", "00001", "11110"),
    "6": ("01110", "10000", "10000", "11110", "10001", "10001", "01110"),
    "7": ("11111", "00001", "00010", "00100", "01000", "01000", "01000"),
    "8": ("01110", "10001", "10001", "01110", "10001", "10001", "01110"),
    "9": ("01110", "10001", "10001", "01111", "00001", "00001", "01110"),
    "中": ("00100", "11111", "10101", "10101", "11111", "00100", "00100"),
    "口": ("11111", "10001", "10001", "10001", "10001", "10001", "11111"),
    "一": ("00000", "00000", "00000", "11111", "00000", "00000", "00000"),
}
CHARACTERS = tuple(GLYPHS)
CODES = {character: index + 1 for index, character in enumerate(CHARACTERS)}
TEXT = ((24, 125, "RUST 0123456789 中口一"), (24, 82, "RUST 123"), (145, 82, "987 中口一"))
WIDTH, HEIGHT = 256.5, 192.25
RASTER_WIDTH, RASTER_HEIGHT = 1026, 769


def stream(payload: bytes, entries: str = "") -> bytes:
    return (f"<< {entries} /Length {len(payload)} >>\nstream\n".encode("ascii")
            + payload + b"\nendstream")


def assemble(objects: list[bytes]) -> bytes:
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode("ascii") + obj + b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode("ascii"))
    output.extend((f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\n"
                   f"startxref\n{xref}\n%%EOF\n").encode("ascii"))
    return bytes(output)


def rectangles(text=TEXT):
    # Unequal corner blocks and all four edge bars expose clipping/rotation.
    result = [(0, 0, WIDTH, 1, (0, 0, 255)),
              (0, HEIGHT - 1, WIDTH, 1, (255, 0, 0)),
              (0, 0, 1, HEIGHT, (0, 255, 0)),
              (WIDTH - 1, 0, 1, HEIGHT, (255, 0, 255)),
              (2, 2, 6, 4, (0, 0, 255)),
              (2, HEIGHT - 10, 4, 8, (255, 0, 0)),
              (WIDTH - 12, HEIGHT - 6, 10, 4, (0, 255, 0)),
              (WIDTH - 7, 2, 5, 12, (255, 0, 255))]
    for x, y, line in text:
        for index, character in enumerate(line):
            for row, cells in enumerate(GLYPHS[character]):
                for column, cell in enumerate(cells):
                    if cell == "1":
                        result.append((x + index * 7.2 + column * 1.2,
                                       y + (6 - row) * 1.2, 1.2, 1.2, (0, 0, 0)))
    return result


def raster() -> bytes:
    # Fixed <= 2.4 MiB original control, independent of input document sizes.
    pixels = bytearray(b"\xff" * (RASTER_WIDTH * RASTER_HEIGHT * 3))
    for x, y, width, height, color in rectangles():
        left, right = round(x * 4), round((x + width) * 4)
        top, bottom = RASTER_HEIGHT - round((y + height) * 4), RASTER_HEIGHT - round(y * 4)
        for row in range(top, bottom):
            begin = (row * RASTER_WIDTH + left) * 3
            pixels[begin:begin + (right - left) * 3] = bytes(color) * (right - left)
    return bytes(pixels)


def font(objects: list[bytes], *, alternate: bool) -> int:
    glyph_ids = []
    for character in CHARACTERS:
        paths = ["600 0 0 0 600 700 d1"]
        for row, cells in enumerate(GLYPHS[character]):
            for column, cell in enumerate(cells):
                if cell == "1":
                    paths.append(f"{column * 100} {(6 - row) * 100} 100 100 re f")
        objects.append(stream(("\n".join(paths) + "\n").encode("ascii")))
        glyph_ids.append(len(objects))
    mappings = []
    for character, code in CODES.items():
        unicode_value = "\ue000" if alternate and character == "R" else character
        mappings.append(f"<{code:02X}> <{unicode_value.encode('utf-16-be').hex().upper()}>")
    cmap = ("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
            "/CIDSystemInfo << /Registry (Original) /Ordering (Canary) /Supplement 0 >> def\n"
            "/CMapName /OriginalCanary def\n/CMapType 2 def\n"
            "1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
            f"{len(mappings)} beginbfchar\n" + "\n".join(mappings)
            + "\nendbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n")
    objects.append(stream(cmap.encode("ascii")))
    cmap_id = len(objects)
    charprocs = " ".join(f"/g{i + 1} {obj} 0 R" for i, obj in enumerate(glyph_ids))
    names = " ".join(f"/g{i + 1}" for i in range(len(CHARACTERS)))
    widths = " ".join("600" for _ in CHARACTERS)
    objects.append((f"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] "
                    f"/FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {charprocs} >> "
                    f"/Encoding << /Type /Encoding /Differences [1 {names}] >> "
                    f"/FirstChar 1 /LastChar {len(CHARACTERS)} /Widths [{widths}] "
                    f"/Resources << >> /ToUnicode {cmap_id} 0 R >>").encode("ascii"))
    return len(objects)


def page_content(*, text=TEXT) -> bytes:
    commands = []
    for x, y, width, height, color in rectangles(()) :
        commands.append(" ".join(str(component / 255) for component in color) + " rg")
        commands.append(f"{x} {y} {width} {height} re f")
    commands.append("0 0 0 rg")
    for x, y, line in text:
        codes = bytes(CODES[character] for character in line).hex().upper()
        commands.append(f"BT /F 12 Tf 1 0 0 1 {x} {y} Tm <{codes}> Tj ET")
    return ("\n".join(commands) + "\n").encode("ascii")


def pdf(*, alternate=False, image_only=False, second=False) -> bytes:
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b""]
    if image_only:
        objects.append(stream(zlib.compress(raster(), level=9),
                              f"/Type /XObject /Subtype /Image /Width {RASTER_WIDTH} "
                              f"/Height {RASTER_HEIGHT} /ColorSpace /DeviceRGB "
                              "/BitsPerComponent 8 /Filter /FlateDecode"))
        resource = b"<< /XObject << /I 3 0 R >> >>"
        pages = [(WIDTH, HEIGHT, 0, f"q {WIDTH} 0 0 {HEIGHT} 0 0 cm /I Do Q\n".encode())]
    else:
        font_id = font(objects, alternate=alternate)
        resource = f"<< /Font << /F {font_id} 0 R >> >>".encode("ascii")
        content = page_content(text=((24, 125, "RUST 9876543210 中口一"),) if second else TEXT)
        pages = [(WIDTH, HEIGHT, 0, content)]
        if not second:
            pages += [(WIDTH, HEIGHT, 0, b""),
                      (WIDTH, HEIGHT, 90, page_content(text=((24, 125, "RUST 321"),))),
                      (WIDTH * 8, HEIGHT * 8, 0, b"q 8 0 0 8 0 0 cm\n" + content + b"Q\n")]
    page_ids = []
    for width, height, rotation, content in pages:
        objects.append(stream(content))
        content_id = len(objects)
        objects.append((f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] "
                        f"/Rotate {rotation} /Contents {content_id} 0 R /Resources ").encode("ascii")
                       + resource + b" >>")
        page_ids.append(len(objects))
    objects[1] = (f"<< /Type /Pages /Count {len(page_ids)} /Kids ["
                  + " ".join(f"{number} 0 R" for number in page_ids) + "] >>").encode("ascii")
    return assemble(objects)


def generate(output_dir: Path) -> dict:
    output_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    records = []
    for name, options, count in (("digital.pdf", {}, 4),
                                 ("alternate-unicode.pdf", {"alternate": True}, 4),
                                 ("image-only.pdf", {"image_only": True}, 1),
                                 ("second-text.pdf", {"second": True}, 1)):
        payload = pdf(**options)
        with (output_dir / name).open("xb") as file:
            file.write(payload)
        records.append({"path": name, "size_bytes": len(payload),
                        "sha256": hashlib.sha256(payload).hexdigest(), "physical_pages": count})
    result = {"schema_version": 1, "basis": "original-mit-controls", "vendor_passes": 0,
              "page_size_points": ["256.5", "192.25"],
              "image_grid": [RASTER_WIDTH, RASTER_HEIGHT], "files": records,
              "unicode_origin_control": "Only ToUnicode changes R to U+E000; visible glyphs remain identical."}
    with (output_dir / "controls.json").open("x") as file:
        json.dump(result, file, indent=2)
        file.write("\n")
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True,
                        help="new external directory; existing directories are never overwritten")
    args = parser.parse_args(argv)
    try:
        result = generate(args.output_dir)
    except OSError as error:
        parser.exit(1, f"control generation failed: {type(error).__name__}\n")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
