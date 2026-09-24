#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original, tiny PDF and malformed CAJ-family test fixtures.

The PDF layout is authored here from the published PDF 1.7 format. No source
document, converter output, or implementation code is imported.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT_DIR = REPOSITORY_ROOT / "tests" / "fixtures"
PDF_HEADER = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"
EMBEDDED_BYTES = b"\x00\xffendstream\nendobj\n5 0 obj\nxref\nstartxref\n%%EOF\n\x80"


def stream_object(payload: bytes, *, declared_length: int | None = None) -> bytes:
    length = len(payload) if declared_length is None else declared_length
    return f"<< /Length {length} >>\nstream\n".encode("ascii") + payload + b"\nendstream"


def pdf_objects(*, inflated_stream_length: bool = False, page_count: int = 2) -> dict[int, bytes]:
    first_page = b"0 0 0 rg\n10 10 40 20 re f\n"
    second_page = b"0 0 0 rg\n30 30 20 40 re f\n"
    declared_length = len(first_page) + 1024 if inflated_stream_length else None
    embedded_stream = (
        f"<< /Type /EmbeddedFile /Length {len(EMBEDDED_BYTES)} >>\nstream\n".encode("ascii")
        + EMBEDDED_BYTES
        + b"\nendstream"
    )
    return {
        1: (
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R /PageMode /UseOutlines "
            b"/Names << /EmbeddedFiles << /Names [(marker.bin) 11 0 R] >> >> >>"
        ),
        2: f"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count {page_count} >>".encode("ascii"),
        3: (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] "
            b"/Resources << >> /Contents 5 0 R >>"
        ),
        4: (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 250] "
            b"/Resources << >> /Contents 6 0 R >>"
        ),
        5: stream_object(first_page, declared_length=declared_length),
        6: stream_object(second_page),
        7: b"<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 2 >>",
        8: (
            b"<< /Title (Part One) /Parent 7 0 R /First 9 0 R /Last 9 0 R "
            b"/Count 1 /Dest [3 0 R /Fit] >>"
        ),
        9: b"<< /Title (Chapter Two) /Parent 8 0 R /Dest [4 0 R /XYZ 0 250 null] >>",
        10: embedded_stream,
        11: b"<< /Type /Filespec /F (marker.bin) /EF << /F 10 0 R >> >>",
    }


def render_pdf(objects: dict[int, bytes], physical_order: tuple[int, ...]) -> bytes:
    if set(physical_order) != set(objects):
        raise ValueError("physical order must include every object")

    pdf = bytearray(PDF_HEADER)
    offsets: dict[int, int] = {}
    for number in physical_order:
        offsets[number] = len(pdf)
        pdf.extend(f"{number} 0 obj\n".encode("ascii"))
        pdf.extend(objects[number])
        pdf.extend(b"\nendobj\n")

    xref_offset = len(pdf)
    pdf.extend(f"xref\n0 {max(objects) + 1}\n".encode("ascii"))
    pdf.extend(b"0000000000 65535 f \n")
    for number in range(1, max(objects) + 1):
        pdf.extend(f"{offsets[number]:010d} 00000 n \n".encode("ascii"))
    pdf.extend(
        f"trailer\n<< /Size {max(objects) + 1} /Root 1 0 R >>\n"
        f"startxref\n{xref_offset}\n%%EOF\n".encode("ascii")
    )
    return bytes(pdf)


def fixtures() -> list[tuple[str, bytes, str, str, str, dict[str, object]]]:
    order = tuple(range(1, 12))
    valid = render_pdf(pdf_objects(), order)
    out_of_order = render_pdf(pdf_objects(), (10, 4, 2, 11, 8, 5, 1, 9, 6, 3, 7))
    oversized_length = render_pdf(pdf_objects(inflated_stream_length=True), order)
    invalid_count = render_pdf(pdf_objects(page_count=3), order)
    duplicate = render_pdf(pdf_objects(), order + (6,))

    xref_header = b"xref\n0 12\n"
    xref_offset = valid.rfind(xref_header)
    assert xref_offset >= 0
    object_five_offset = xref_offset + len(xref_header) + 5 * 20
    invalid_xref = (
        valid[:object_five_offset] + b"9999999999" + valid[object_five_offset + 10 :]
    )
    truncated_xref = valid[: xref_offset + len(xref_header) + 7]

    valid_metadata: dict[str, object] = {
        "pages": [[200, 300], [400, 250]],
        "outlines": [
            {"title": "Part One", "page": 1, "children": [
                {"title": "Chapter Two", "page": 2, "children": []}
            ]}
        ],
    }
    return [
        (
            "valid_nested_outline.pdf", valid, "PDF", "valid",
            "Two page sizes, nested destinations, and PDF-like markers inside an embedded binary stream.",
            valid_metadata,
        ),
        (
            "valid_out_of_order_objects.pdf", out_of_order, "PDF", "valid",
            "Object definitions are physically out of numerical order; xref entries remain correct.",
            valid_metadata,
        ),
        (
            "invalid_stream_length.pdf", oversized_length, "PDF", "malformed",
            "A page content stream declares more bytes than are present before endstream.", {},
        ),
        (
            "invalid_xref_offset.pdf", invalid_xref, "PDF", "malformed",
            "The xref entry for object 5 points beyond the end of the file.", {},
        ),
        (
            "invalid_page_count.pdf", invalid_count, "PDF", "malformed",
            "The Pages node declares three pages but has only two leaf children.", {},
        ),
        (
            "duplicate_object.pdf", duplicate, "PDF", "malformed",
            "Object 6 is defined twice in one body without an incremental update section.", {},
        ),
        (
            "truncated_xref.pdf", truncated_xref, "PDF", "malformed",
            "The xref table stops inside its first entry and has no trailer or EOF marker.", {},
        ),
        ("truncated_caj.caj", b"CAJ", "CAJ", "malformed", "Signature only; header is absent.", {}),
        ("truncated_hn.hn", b"HN", "HN", "malformed", "Signature only; header is absent.", {}),
        ("truncated_c8.c8", b"\xc8\x00\x00\x00", "C8", "malformed", "Signature only; header is absent.", {}),
        ("truncated_kdh.kdh", b"KDH", "KDH", "malformed", "Signature only; header is absent.", {}),
        ("truncated_teb.teb", b"TEB", "TEB", "malformed", "Signature only; header is absent.", {}),
    ]


def generated_files() -> dict[str, bytes]:
    files: dict[str, bytes] = {}
    manifest_entries = []
    for path, payload, file_format, outcome, condition, metadata in fixtures():
        files[path] = payload
        manifest_entries.append({
            "path": path,
            "sha256": hashlib.sha256(payload).hexdigest(),
            "size": len(payload),
            "format": file_format,
            "expected_outcome": outcome,
            "condition": condition,
            **metadata,
        })
    manifest = {
        "schema_version": 1,
        "generator": "scripts/generate_fixtures.py",
        "license": "MIT",
        "fixtures": sorted(manifest_entries, key=lambda entry: entry["path"]),
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    parser.add_argument("--check", action="store_true", help="fail if generated files differ from disk")
    args = parser.parse_args()

    files = generated_files()
    if args.check:
        mismatches = [
            name for name, payload in files.items()
            if not (args.output_dir / name).is_file() or (args.output_dir / name).read_bytes() != payload
        ]
        if mismatches:
            parser.exit(1, "Fixture mismatch: " + ", ".join(sorted(mismatches)) + "\n")
        return 0

    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, payload in files.items():
        (args.output_dir / name).write_bytes(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
