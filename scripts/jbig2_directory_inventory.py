#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Read-only, optional header inventory for five SHA-pinned HN/C8 sources.

Python locates the HN/C8 type-3 record spans using repository-owned container
helpers. A Rust example then calls the shared JBIG2 directory API on every
embedded payload. Neither path decodes pixels or copies compressed payloads.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import conformance
import jbig1_oracle as container


ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/conformance/matrix.json"
EXPECTED_BY_SHA = {
    "826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134": ("HN", 105),
    "46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa": ("HN", 208),
    "01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab": ("HN", 228),
    "8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85": ("C8", 4),
    "90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6": ("C8", 1),
}
EXPECTED_IMAGES = 546
MAX_IMAGE_RECORDS = 100_000
MAX_IMAGE_BYTES = 64 * 1024 * 1024
WRAPPER_BYTES = 48


class InventoryError(Exception):
    """A pinned input, container field, or Rust directory check failed."""


def selected_rows() -> list[dict]:
    rows = conformance.load_matrix(MATRIX)
    selected = [row for row in rows if row.get("sha256") in EXPECTED_BY_SHA]
    if len(selected) != len(EXPECTED_BY_SHA) or len({row["sha256"] for row in selected}) != len(selected):
        raise InventoryError("matrix no longer contains five unique pinned HN/C8 sources")
    for row in selected:
        expected_variant, _ = EXPECTED_BY_SHA[row["sha256"]]
        if row["variant"] != expected_variant:
            raise InventoryError(f"{row['id']}: matrix variant changed")
    return selected


def checked_source(root: Path, row: dict) -> Path:
    path = conformance.contained_file(root, conformance.relative_path(row["path"]))
    size = path.stat().st_size
    if size != row["size_bytes"]:
        raise InventoryError(f"{row['id']}: source size differs from matrix")
    blob, sha = conformance.file_hashes(path, size, True)
    if blob != row["git_blob_oid"] or sha != row["sha256"]:
        raise InventoryError(f"{row['id']}: source SHA-256 or Git blob differs from matrix")
    return path


def dimensions(wrapper: bytes, label: str) -> tuple[int, int]:
    if len(wrapper) != WRAPPER_BYTES or container.u32(wrapper, 0) != 40:
        raise InventoryError(f"{label}: type-3 image lacks the 48-byte DIB/palette wrapper")
    width, height = container.i32(wrapper, 4), container.i32(wrapper, 8)
    if width <= 0 or height <= 0:
        raise InventoryError(f"{label}: DIB dimensions are not positive")
    if container.u16(wrapper, 12) != 1 or container.u16(wrapper, 14) != 1:
        raise InventoryError(f"{label}: DIB is not one-plane, one-bit")
    return width, height


def discover_type3(row: dict, path: Path) -> list[dict]:
    images: list[dict] = []
    with path.open("rb") as source:
        before = os.fstat(source.fileno())
        size = before.st_size
        variant, page_count, table_at = container.page_table(source, size)
        if variant != row["variant"]:
            raise InventoryError(f"{row['id']}: detected {variant}, expected {row['variant']}")
        table_end = table_at + page_count * container.PAGE_ROW_BYTES
        total_records = 0
        for page in range(1, page_count + 1):
            page_row = container.read_exact(
                source,
                table_at + (page - 1) * container.PAGE_ROW_BYTES,
                container.PAGE_ROW_BYTES,
                size,
                f"{row['id']} page {page} row",
            )
            text_offset = container.i32(page_row, 0)
            text_length = container.i32(page_row, 4)
            image_count = container.i16(page_row, 8)
            container.require_span(text_offset, text_length, size, f"{row['id']} page {page} text")
            if not 0 <= image_count <= container.MAX_IMAGES_PER_PAGE:
                raise InventoryError(f"{row['id']} page {page}: invalid image count")
            total_records += image_count
            if total_records > MAX_IMAGE_RECORDS:
                raise InventoryError(f"{row['id']}: image record limit exceeded")
            record_at = text_offset + text_length
            if image_count and record_at < table_end:
                raise InventoryError(f"{row['id']} page {page}: image records overlap page table")
            for image in range(1, image_count + 1):
                label = f"{row['id']} page {page} image {image}"
                record = container.read_exact(
                    source, record_at, container.IMAGE_ROW_BYTES, size, f"{label} record"
                )
                image_type = container.i32(record, 0)
                offset = container.i32(record, 4)
                length = container.i32(record, 8)
                if image_type < 0:
                    raise InventoryError(f"{label}: negative image type")
                container.require_span(offset, length, size, label)
                if offset < record_at + container.IMAGE_ROW_BYTES or length <= 0:
                    raise InventoryError(f"{label}: record overlaps image or has zero length")
                record_at = offset + length
                if image_type != 3:
                    continue
                if length <= WRAPPER_BYTES or length > MAX_IMAGE_BYTES:
                    raise InventoryError(f"{label}: type-3 record exceeds bounded image range")
                wrapper = container.read_exact(source, offset, WRAPPER_BYTES, size, f"{label} DIB")
                width, height = dimensions(wrapper, label)
                images.append(
                    {
                        "page": page,
                        "image": image,
                        "offset": offset,
                        "length": length,
                        "width": width,
                        "height": height,
                    }
                )
        after = os.fstat(source.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ino) != (
            after.st_size,
            after.st_mtime_ns,
            after.st_ino,
        ):
            raise InventoryError(f"{row['id']}: source changed during discovery")
    expected = EXPECTED_BY_SHA[row["sha256"]][1]
    if len(images) != expected:
        raise InventoryError(f"{row['id']}: expected {expected} type-3 images, found {len(images)}")
    return images


def rust_directory_rows(records: list[tuple[Path, dict]]) -> list[list[dict]]:
    with tempfile.TemporaryDirectory(prefix="caj2pdf-jbig2-inventory-") as temporary:
        manifest = Path(temporary) / "spans.tsv"
        with manifest.open("w", encoding="utf-8") as output:
            for path, image in records:
                if any(character in str(path) for character in "\t\r\n"):
                    raise InventoryError("source path cannot be represented in manifest")
                output.write(f"{path}\t{image['offset']}\t{image['length']}\n")
        command = [
            "cargo", "run", "--quiet", "--locked", "-p", "caj2pdf-core",
            "--example", "jbig2_directory_inventory", "--", str(manifest),
        ]
        try:
            completed = subprocess.run(
                command, cwd=ROOT, capture_output=True, text=True, timeout=300, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise InventoryError(f"cannot run Rust directory inventory: {exc}") from exc
    if completed.returncode != 0:
        raise InventoryError(f"Rust directory inventory failed: {completed.stderr.strip()}")
    try:
        lines = [json.loads(line) for line in completed.stdout.splitlines()]
    except json.JSONDecodeError as exc:
        raise InventoryError(f"Rust directory output is not JSON: {exc}") from exc
    if len(lines) != len(records) or any(row.get("index") != index for index, row in enumerate(lines)):
        raise InventoryError("Rust directory output omitted or reordered an image")
    return [row["segments"] for row in lines]


def inventory(corpus_dir: Path | None) -> dict:
    report = {
        "status": "NOT_RUN",
        "expected_images": EXPECTED_IMAGES,
        "checked_images": 0,
        "samples": [],
        "failures": [],
    }
    if corpus_dir is None:
        report["reason"] = "CAJ2PDF_CORPUS_DIR is unset"
        return report
    root = corpus_dir.resolve(strict=True)
    if not root.is_dir():
        raise InventoryError(f"corpus directory is not a directory: {corpus_dir}")
    rows = selected_rows()
    paths: list[Path] = []
    records: list[tuple[Path, dict]] = []
    for row in rows:
        path = checked_source(root, row)  # SHA-256 precedes all container reads.
        paths.append(path)
        sample = {
            "id": row["id"],
            "path": row["path"],
            "source_sha256": row["sha256"],
            "variant": row["variant"],
            "images": discover_type3(row, path),
        }
        report["samples"].append(sample)
        records.extend((path, image) for image in sample["images"])
    if len(records) != EXPECTED_IMAGES:
        raise InventoryError(f"expected {EXPECTED_IMAGES} type-3 records, found {len(records)}")
    directories = rust_directory_rows(records)
    for (_, image), segments in zip(records, directories, strict=True):
        image["segments"] = segments
    for row, path in zip(rows, paths, strict=True):
        checked_source(root, row)  # Detect source replacement during Rust parsing.
    report["status"] = "PASS"
    report["checked_images"] = len(records)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path, default=None)
    parser.add_argument("--json", action="store_true", help="emit full metadata JSON")
    args = parser.parse_args()
    supplied = args.corpus_dir or (
        Path(os.environ["CAJ2PDF_CORPUS_DIR"]) if os.environ.get("CAJ2PDF_CORPUS_DIR") else None
    )
    try:
        report = inventory(supplied)
    except (InventoryError, conformance.ConformanceError, container.OracleError, OSError) as exc:
        report = {
            "status": "FAIL",
            "expected_images": EXPECTED_IMAGES,
            "checked_images": 0,
            "samples": [],
            "failures": [str(exc)],
        }
    if args.json:
        print(json.dumps(report, ensure_ascii=False, separators=(",", ":")))
    else:
        print(f"JBIG2 directory inventory: {report['status']} "
              f"({report['checked_images']}/{report['expected_images']} images)")
        for failure in report["failures"]:
            print(f"  {failure}", file=sys.stderr)
        if "reason" in report:
            print(f"  {report['reason']}")
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
